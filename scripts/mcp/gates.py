#!/usr/bin/env python3
"""OmniGet MCP acceptance gates (G01-G11) against a built desktop binary.

  python3 scripts/mcp/gates.py --app <path/to/omniget> [--worker-dir DIR]
      [--gates G01,G03,...] [--live --corpus cases.json [--live-limit N]]
      [--ytdlp PATH] [--out DIR] [--keep]

What it does
  * Creates disposable profiles + HOMEs under --out (default: a new dir under
    $TMPDIR) and launches the app with OMNIGET_DATA_DIR and
    OMNIGET_TEST_DRIVER=1. It records the PIDs it starts and signals only
    those (never pkill); the worker children of its own app instances are
    stopped by exact PID only if they outlive the app.
  * Uses the test driver (POST /v1/debug/eval) for the trusted Settings
    commands: enable MCP, create clients, folder and network grants.
  * Starts scripts/mcp/fixture_server.py on free loopback ports (plus a canary
    port that must never be reached) and runs the offline gates:
      G01 parity: HTTP vs stdio adapter vs UI commands on one fixture job
      G03 operations: inspect/preflight/batch/status/wait/pause/resume/cancel/
          retry/history/logs/artifacts, multi-file listing, >64 MiB transfer
      G04 idempotency: replay, conflicts, concurrent duplicates, lost response,
          kill -9 with a job in flight -> visible Unknown -> reconcile, no duplicate
      G05 diagnosis after restart: attempts, typed next actions, bounded
          package, bundle, log cursors
      G06 synthetic secrets absent from profile files, logs, MCP responses,
          window events, UI commands, /v1/queue and bundles; client isolation
      G07 protocol: initialize/revisions, JSON-RPC errors, outputSchema on
          every tool with structuredContent validation, HTTP + stdio batches,
          offline adapter, optional official SDK check (/tmp/omniget-mcp-sdk)
      G09 isolation: bridge routes with an MCP token, Origin/Host, SSRF,
          redirect and manifest egress
      G10 files: traversal, symlink, overwrite, transfer grants, revocation
      G11 UI: connect/test/revoke clicking through the real window
    Independent gates run in parallel (separate clients; G04 and G05 get
    their own app instance because they kill/restart it).
  * --live also runs scripts/mcp/desktop_benchmark.py against --corpus on the
    main instance (real network; needs --ytdlp for yt-dlp sites).

Requirements: Python 3.9+ stdlib, ffmpeg/ffprobe on PATH, the omniget-worker
(and omniget-mcp for stdio) next to the app or in --worker-dir. A debug build
loads its UI from localhost:1420: --frontend auto (default) uses a server
already listening there, otherwise starts `vite dev` from this checkout (own
PID, stopped at the end); --frontend <dir> serves a built frontend instead.
Output: a table on stdout and <out>/gates.json; exit code 1 if any gate fails.
Tokens are never printed; they live in memory and in chmod-600 files under --out.
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
import pathlib
import re
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import urllib.error
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
BASE_SCOPES = ['discover', 'enqueue', 'control', 'diagnostics', 'artifacts', 'history']
ALL_GATES = ['G01', 'G03', 'G04', 'G05', 'G06', 'G07', 'G09', 'G10', 'G11']
SETTINGS_TEMPLATE = {
    'schema_version': 2,
    'appearance': {'language': 'en', 'sidebar_collapsed': False, 'theme': 'system'},
    'download': {
        'always_ask_path': False, 'always_use_managed_cookies': False, 'auto_download_on_paste': False,
        'bilibili_cdn_hosts': '', 'bilibili_cdn_prefer_alternatives': False, 'bilibili_container': 'mp4',
        'bilibili_cover_format': 'jpg', 'bilibili_cover_sidecar': False, 'bilibili_danmaku_enabled': False,
        'bilibili_danmaku_format': 'xml',
        'bilibili_naming_bangumi': '{series_title}/Season {season_number}/{episode_number_pad2} - {episode_title}',
        'bilibili_naming_cheese': '{series_title}/{section_title}/{episode_number_pad2} - {episode_title}',
        'bilibili_naming_collection': '{collection_title}/{title}',
        'bilibili_naming_multi_part': '{parent_title}/P{page} - {leaf_title}', 'bilibili_naming_video': '{title}',
        'bilibili_nfo_enabled': False, 'bilibili_preferred_audio_qn': 30300, 'bilibili_preferred_codec': 20,
        'bilibili_preferred_qn': 200, 'caption_locale': 'en', 'clip_hotkey_binding': 'CmdOrCtrl+Shift+B',
        'clip_hotkey_enabled': False, 'clipboard_detection': False, 'continuous_lecture_numbers': False,
        'cookie_file': '', 'copy_to_clipboard_on_hotkey': False, 'default_output_dir': '',
        'download_attachments': True, 'download_descriptions': True, 'download_subtitles': False,
        'embed_metadata': False, 'embed_thumbnail': False, 'extra_ytdlp_flags': [],
        'filename_template': '%(title).200s [%(id)s].%(ext)s', 'hotkey_binding': 'CmdOrCtrl+Shift+D',
        'hotkey_enabled': False, 'include_auto_subtitles': False, 'keep_vtt': False, 'live_from_start': False,
        'music_audio_format': 'm4a', 'music_hotkey_binding': 'CmdOrCtrl+Shift+M', 'music_hotkey_enabled': False,
        'organize_by_platform': False, 'saved_output_dirs': [], 'skip_existing': True, 'speed_limit': '',
        'split_by_chapters': False, 'sponsorblock_categories': ['sponsor', 'selfpromo', 'interaction'],
        'sponsorblock_mode': 'remove', 'translate_metadata': False, 'video_quality': '720p',
        'write_nfo_sidecar': False, 'youtube_sponsorblock': False},
    'advanced': {
        'concurrent_fragments': 8, 'cookies_from_browser': '', 'insecure_tls': False, 'max_concurrent_downloads': 8,
        'max_concurrent_segments': 20, 'max_retries': 3, 'prevent_sleep': False, 'stagger_delay_ms': 150,
        'torrent_auto_trackers': False, 'torrent_listen_port': 6881, 'torrent_upnp': False,
        'twitter_manual_cookie': '', 'user_agent': ''},
}
JS = r"""const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const vis=e=>!!(e&&(e.offsetParent||e.getClientRects().length));
const inv=(c,a)=>window.__TAURI_INTERNALS__.invoke(c,a||{});
const txt=()=>(document.querySelector('main')||document.body).innerText;
const byText=(t,root)=>[...(root||document).querySelectorAll('button,a,[role=button],summary')].filter(vis).find(b=>b.innerText.trim()===t);
const type=async(el,v)=>{el.focus();const p=el.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(p,'value').set.call(el,v);el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));await sleep(150);};
"""


# ── small utilities ────────────────────────────────────────────────────

def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def write_private(path, data):
    path = pathlib.Path(path)
    fd = os.open(str(path), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, 'w') as f:
        f.write(data if isinstance(data, str) else json.dumps(data))
    os.chmod(str(path), 0o600)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


class GateFail(Exception):
    pass


class GateSkip(Exception):
    pass


def need(cond, msg):
    if not cond:
        raise GateFail(msg)


def validate(schema, value, path='$'):
    """Small JSON Schema subset validator (the keywords OmniGet's schemas use)."""
    errs = []
    if not isinstance(schema, dict):
        return errs
    if 'anyOf' in schema or 'oneOf' in schema:
        options = schema.get('anyOf') or schema.get('oneOf')
        if not any(not validate(o, value, path) for o in options):
            errs.append(f'{path}: matches none of anyOf/oneOf')
    for sub in schema.get('allOf', []):
        errs += validate(sub, value, path)
    t = schema.get('type')
    if t is not None:
        types = t if isinstance(t, list) else [t]
        ok = False
        for ty in types:
            ok |= (ty == 'object' and isinstance(value, dict)) or (ty == 'array' and isinstance(value, list)) \
                or (ty == 'string' and isinstance(value, str)) or (ty == 'boolean' and isinstance(value, bool)) \
                or (ty == 'null' and value is None) \
                or (ty == 'integer' and isinstance(value, int) and not isinstance(value, bool)) \
                or (ty == 'number' and isinstance(value, (int, float)) and not isinstance(value, bool))
        if not ok:
            return errs + [f'{path}: expected {t}, got {type(value).__name__}']
    if 'const' in schema and value != schema['const']:
        errs.append(f'{path}: const mismatch')
    if 'enum' in schema and value not in schema['enum']:
        errs.append(f'{path}: {value!r} not in enum')
    if isinstance(value, dict):
        props = schema.get('properties', {})
        for k in schema.get('required', []):
            if k not in value:
                errs.append(f'{path}: missing {k}')
        for k, v in value.items():
            if k in props:
                errs += validate(props[k], v, f'{path}.{k}')
            elif schema.get('additionalProperties') is False:
                errs.append(f'{path}: unexpected {k}')
            elif isinstance(schema.get('additionalProperties'), dict):
                errs += validate(schema['additionalProperties'], v, f'{path}.{k}')
    if isinstance(value, list) and isinstance(schema.get('items'), dict):
        for i, v in enumerate(value[:200]):
            errs += validate(schema['items'], v, f'{path}[{i}]')
    return errs


# ── MCP / bridge clients ───────────────────────────────────────────────

class Mcp:
    """HTTP MCP client for one connection token. Responses are kept for the secret scan."""

    def __init__(self, inst, token):
        self.inst, self.token, self.n = inst, token, 0
        self.base = f'http://127.0.0.1:{inst.port}'

    def raw(self, method, path, body=None, headers=None, token='mcp', timeout=60):
        h = {}
        tok = self.token if token == 'mcp' else (self.inst.bridge_token if token == 'ext' else token)
        if tok:
            h['Authorization'] = 'Bearer ' + tok
        if path.startswith('/mcp') and body is not None:
            h.update({'Content-Type': 'application/json', 'Accept': 'application/json, text/event-stream',
                      'MCP-Protocol-Version': '2025-06-18'})
        h.update(headers or {})
        h = {k: v for k, v in h.items() if v is not None}
        data = body if isinstance(body, (bytes, type(None))) else json.dumps(body).encode()
        req = urllib.request.Request(self.base + path, data=data, method=method, headers=h)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                code, raw, hd = r.status, r.read(), dict(r.headers)
        except urllib.error.HTTPError as e:
            code, raw, hd = e.code, e.read(), dict(e.headers)
        self.inst.run.record(raw)
        return code, raw, hd

    def rpc(self, method, params=None, headers=None):
        self.n += 1
        msg = {'jsonrpc': '2.0', 'id': self.n, 'method': method}
        if params is not None:
            msg['params'] = params
        code, raw, _ = self.raw('POST', '/mcp', msg, headers)
        try:
            return code, json.loads(raw)
        except ValueError:
            return code, {'raw': raw[:300].decode('utf-8', 'replace')}

    def call(self, name, args=None):
        code, r = self.rpc('tools/call', {'name': name, 'arguments': args or {}})
        res = r.get('result', {}) if isinstance(r, dict) else {}
        return {'http': code, 'isError': bool(res.get('isError')), 'sc': res.get('structuredContent') or {},
                'text': (res.get('content') or [{}])[0].get('text'), 'error': r.get('error') if isinstance(r, dict) else None}

    def ok(self, name, args=None):
        for _ in range(40):
            r = self.call(name, args)
            code = (r['sc'].get('error') or {}).get('code')
            if code == 'ARTIFACT_BUSY':
                time.sleep(0.5)
                continue
            if code in ('RETRY_COOLDOWN_UNTIL', 'RECONCILE_WRITER_ACTIVE') and name in ('download_retry', 'download_resume'):
                until = int(r['sc']['error']['message'].split(':')[1]) / 1000
                time.sleep(min(90, max(0, until - time.time())) + 0.3)
                continue
            break
        if r['isError'] or r['error']:
            raise GateFail(f"{name} failed: {json.dumps(r['sc'].get('error') or r['error'])[:200]}")
        return r['sc']

    def item_id(self, sc):
        return (sc.get('item') or {}).get('id') or sc.get('download_id')

    def wait(self, did, limit=120):
        t = time.time()
        last = {}
        while time.time() - t < limit:
            r = self.call('download_wait', {'download_id': did, 'timeoutMs': 15000})
            if r['isError'] or r['error']:
                raise GateFail(f"download_wait failed: {json.dumps(r['sc'] or r['error'])[:200]}")
            last = r['sc'].get('item') or {}
            if (last.get('status') or {}).get('type') in ('Complete', 'Error', 'Unknown'):
                return last
        raise GateFail(f'job {did} did not settle in {limit}s (last {json.dumps(last.get("status"))[:120]})')

    def fetch(self, access, rng=None, token=None, if_match=True, limit=None):
        h = {'Authorization': 'Bearer ' + (token or self.token)}
        if if_match:
            h['If-Match'] = '"' + access['digest'] + '"'
        if rng:
            h['Range'] = rng
        req = urllib.request.Request(self.base + access['downloadPath'], headers=h)
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                d = hashlib.sha256()
                n = 0
                while True:
                    ch = r.read(1 << 20)
                    if not ch:
                        break
                    d.update(ch)
                    n += len(ch)
                    if limit and n >= limit:
                        break
                return {'status': r.status, 'bytes': n, 'sha256': d.hexdigest(), 'contentRange': r.headers.get('Content-Range')}
        except urllib.error.HTTPError as e:
            return {'status': e.code}


def stdio_session(run, inst, token, lines, port=None, env_url=None):
    exe = run.mcp_bin
    url = env_url or f'http://127.0.0.1:{port or inst.port}/mcp'
    data = ''.join((l if isinstance(l, str) else json.dumps(l)) + '\n' for l in lines)
    p = subprocess.run([exe], input=data, capture_output=True, text=True, timeout=90,
                       env={'OMNIGET_MCP_URL': url, 'OMNIGET_MCP_TOKEN': token, 'PATH': '/usr/bin:/bin'})
    run.record(p.stdout.encode())
    out, bad = [], []
    for l in p.stdout.splitlines():
        try:
            out.append(json.loads(l))
        except ValueError:
            bad.append(l[:120])
    return {'exit': p.returncode, 'messages': out, 'nonJson': bad, 'stderr': p.stderr[:300]}


# ── app instance ───────────────────────────────────────────────────────

class Instance:
    def __init__(self, run, name):
        self.run, self.name = run, name
        self.dir = run.out / name
        self.profile, self.home = self.dir / 'profile', self.dir / 'home'
        self.media = self.profile / 'media'
        self.port = free_port()
        self.bridge_token = secrets.token_urlsafe(32)
        self.tokens, self.principals = {}, {}
        self.proc = None
        self.pids = []
        self.ui_lock = threading.Lock()
        # The test driver keeps one pending eval: a second concurrent eval makes
        # the first one time out, so every eval of an instance is serialized.
        self.eval_lock = threading.RLock()

    def prepare(self):
        for d in (self.media, self.home, self.profile / 'bin'):
            d.mkdir(parents=True, exist_ok=True)
        s = json.loads(json.dumps(SETTINGS_TEMPLATE))
        s['download']['default_output_dir'] = str(self.media)
        s.update({'bridge': {'enabled': True, 'port': self.port, 'token': self.bridge_token},
                  'onboarding_completed': True, 'legal_acknowledged': True, 'start_minimized': True,
                  'start_with_system': False})
        write_private(self.profile / 'settings.json', {'app_settings': s})
        for tool in ('ffmpeg', 'ffprobe', 'deno'):
            src = shutil.which(tool) or f'/opt/homebrew/bin/{tool}'
            if os.path.exists(src):
                link = self.profile / 'bin' / tool
                if not link.exists():
                    link.symlink_to(os.path.realpath(src))
        if self.run.args.ytdlp:
            onedir = ytdlp_onedir(self.run.args.ytdlp)
            if onedir:
                # Where managed_ytdlp_onedir_exe looks (bin/yt-dlp_onedir/yt-dlp_macos). A link,
                # not a copy: a fresh copy pays the macOS first-run scan of _internal/ (20-40 s)
                # and the worker canonicalizes the path, so Seatbelt grants the real directory.
                link = self.profile / 'bin' / 'yt-dlp_onedir'
                if not link.exists():
                    link.symlink_to(onedir, target_is_directory=True)
            else:
                shutil.copy2(self.run.args.ytdlp, self.profile / 'bin' / 'yt-dlp')

    def start(self):
        env = dict(os.environ, HOME=str(self.home), OMNIGET_DATA_DIR=str(self.profile), OMNIGET_TEST_DRIVER='1')
        env.pop('OMNIGET_PORTABLE', None)
        log = open(self.dir / f'app-{len(self.pids)}.log', 'ab')
        self.proc = subprocess.Popen([self.run.app], cwd=str(self.dir), env=env, stdout=log, stderr=subprocess.STDOUT,
                                     stdin=subprocess.DEVNULL)
        self.pids.append(self.proc.pid)
        self.run.own_pids.add(self.proc.pid)
        self.wait_ready()
        # A reset settings file would move the bridge to a default port.
        cfg = json.loads((self.profile / 'settings.json').read_text())['app_settings']
        need(cfg['bridge']['port'] == self.port, f'{self.name}: the app rewrote settings.json (template no longer matches this build)')

    def wait_ready(self, limit=90):
        t = time.time()
        while time.time() - t < limit:
            if self.proc.poll() is not None:
                raise GateFail(f'{self.name}: app exited with {self.proc.returncode} (see {self.dir}/app-*.log)')
            try:
                with urllib.request.urlopen(f'http://127.0.0.1:{self.port}/v1/health', timeout=2) as r:
                    if r.status == 200:
                        ok = self.eval('return !!(window.__TAURI_INTERNALS__ && document.body)', timeout=10)
                        if ok is True:
                            return
            except Exception:
                pass
            time.sleep(0.5)
        raise GateFail(f'{self.name}: bridge/test driver not ready in {limit}s (debug builds need the Vite dev server)')

    def children(self):
        if not self.proc:
            return []
        out = subprocess.run(['ps', '-A', '-o', 'pid=,ppid='], capture_output=True, text=True).stdout
        return [int(a) for a, b in (l.split() for l in out.splitlines() if l.strip()) if int(b) == self.proc.pid]

    def _reap_children(self, kids, grace=15):
        t = time.time()
        alive = kids
        while alive and time.time() - t < grace:
            time.sleep(0.5)
            alive = [k for k in alive if subprocess.run(['ps', '-p', str(k)], capture_output=True).returncode == 0]
        for k in alive:  # our app's own worker children, by exact PID
            try:
                os.kill(k, signal.SIGKILL)
            except OSError:
                pass
        return alive

    def stop(self):
        if not self.proc or self.proc.poll() is not None:
            return []
        kids = self.children()
        self.proc.terminate()
        try:
            self.proc.wait(15)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait(10)
        self.run.own_pids.discard(self.proc.pid)
        return self._reap_children(kids)

    def kill9(self):
        kids = self.children()
        os.kill(self.proc.pid, signal.SIGKILL)
        self.proc.wait(10)
        self.run.own_pids.discard(self.proc.pid)
        return kids, self._reap_children(kids)

    def eval(self, body, timeout=60):
        with self.eval_lock:
            return self._eval(body, timeout)

    def _eval(self, body, timeout):
        req = urllib.request.Request(f'http://127.0.0.1:{self.port}/v1/debug/eval', data=(JS + body).encode(), method='POST',
                                     headers={'authorization': 'Bearer ' + self.bridge_token, 'x-timeout': str(timeout)})
        with urllib.request.urlopen(req, timeout=timeout + 15) as r:
            return json.loads(r.read())

    def inv(self, cmd, args=None):
        r = self.eval(f'try{{return {{ok:await inv({json.dumps(cmd)},{json.dumps(args or {})})}}}}catch(e){{return {{err:String(e)}}}}')
        if not isinstance(r, dict) or 'err' in r or 'ok' not in r:
            raise GateFail(f'{cmd}: {json.dumps(r)[:200]}')
        return r['ok']

    def client(self, label, scopes, root=False, network=False):
        g = self.inv('tool_mcp_client_create', {'name': f'gates {label}', 'scopes': scopes})
        self.tokens[label] = g['token']
        self.principals[label] = {'id': g['principal']['id']}
        if network:
            self.inv('tool_mcp_network_grant', {'principal': g['principal']['id'], 'endpoints': [f'127.0.0.1:{self.run.fx_port}']})
        if root:
            self.principals[label]['root'] = self.inv('tool_mcp_root_grant', {'principal': g['principal']['id'], 'path': str(self.media)})
        write_private(self.profile / 'tokens.json', self.tokens)
        return Mcp(self, g['token'])


# ── gates ──────────────────────────────────────────────────────────────

def fx(run, name, gate, **q):
    # A unique path segment per job: jobs never share a URL by accident.
    path = f'/{gate}-{run.next()}/{name}'
    return f'http://127.0.0.1:{run.fx_port}{path}' + ('?' + '&'.join(f'{k}={v}' for k, v in q.items()) if q else '')


def enqueue_wait(c, url, key, limit=120, **extra):
    r = c.call('download_enqueue', dict({'url': url, 'idempotencyKey': key}, **extra))
    need(not r['isError'] and not r['error'], f'enqueue {url.split("/")[-1][:40]} refused: {json.dumps(r["sc"] or r["error"])[:160]}')
    did = c.item_id(r['sc'])
    return did, c.wait(did, limit)


def gate_g01(run, A, D):
    c = A.client('G01', BASE_SCOPES + ['transfer', 'local_network'], root=True, network=True)
    other = A.client('G01b', BASE_SCOPES)
    url = fx(run, 'good.mp4', 'g01')
    need(run.mcp_bin, 'omniget-mcp adapter not found next to the app')
    s = stdio_session(run, A, c.token, [
        {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'gates', 'version': '1'}}},
        {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
        {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call', 'params': {'name': 'download_enqueue', 'arguments': {'url': url, 'idempotencyKey': 'g01-stdio'}}}])
    msg = [m for m in s['messages'] if isinstance(m, dict) and m.get('id') == 2]
    need(msg and not msg[0].get('error'), f'stdio enqueue failed: {json.dumps(msg)[:160]}')
    did = c.item_id(msg[0]['result']['structuredContent'])
    item = c.wait(did)
    need(item['status']['type'] == 'Complete', 'fixture job did not complete')
    http_art = c.ok('download_artifacts', {'download_id': did})['artifacts'][0]
    s2 = stdio_session(run, A, c.token, [{'jsonrpc': '2.0', 'id': 3, 'method': 'tools/call', 'params': {'name': 'download_artifacts', 'arguments': {'download_id': did}}},
                                         {'jsonrpc': '2.0', 'id': 4, 'method': 'tools/call', 'params': {'name': 'download_status', 'arguments': {'download_id': did}}}])
    by = {m.get('id'): m for m in s2['messages'] if isinstance(m, dict)}
    st_art = by[3]['result']['structuredContent']['artifacts'][0]
    need(st_art['access']['digest'] == http_art['access']['digest'] == run.good_sha, 'HTTP and stdio artifact digests differ from the fixture')
    need(by[4]['result']['structuredContent']['item']['id'] == did, 'stdio status returned another job')
    got = c.fetch(http_art['access'])
    need(got['status'] == 200 and got['sha256'] == run.good_sha, f'transfer mismatch {got}')
    with A.ui_lock:
        ui = A.eval(f'const h=await inv("get_download_history"); const e=h.find(x=>String(x.id)==="{did}"); const log=await inv("get_download_log",{{downloadId:{did}}}); return {{found:!!e, size:e&&e.file_size_bytes, success:e&&e.success, logLines:log.length}}')
    need(ui.get('found') and ui.get('size') == http_art['bytes'] and ui.get('success'), f'UI history does not show the same job: {ui}')
    need(other.call('download_status', {'download_id': did})['sc'].get('error', {}).get('code') == 'DOWNLOAD_NOT_FOUND', 'another client can see the job')
    return f'job {did}: stdio enqueue, HTTP+stdio status/artifacts, UI history agree (digest = fixture)'


def gate_g03(run, A, D):
    c = A.client('G03', BASE_SCOPES + ['transfer', 'local_network'], root=True, network=True)
    notes = []
    need(not c.call('media_inspect', {'url': fx(run, 'good.mp4', 'g03')})['isError'], 'media_inspect failed')
    pf = c.ok('downloads_preflight', {'urls': [fx(run, 'good.mp4', 'g03'), 'notaurl']})
    need(len(pf['items']) == 2 and pf['items'][1]['valid'] is False, 'preflight did not flag an invalid URL')
    mc = c.call('media_collection_list', {'url': fx(run, 'good.mp4', 'g03'), 'limit': 2})
    if (mc['sc'].get('error') or {}).get('code') == 'WORKER_DEPENDENCY_UNAVAILABLE':
        notes.append('media_collection_list needs yt-dlp (--ytdlp): not exercised')
    else:
        need(not mc['isError'], f'media_collection_list failed: {mc["sc"]}')
    urls = [fx(run, 'good.mp4', 'g03b'), fx(run, 'good.mp4', 'g03b'), 'notaurl']
    b1 = c.ok('downloads_batch_enqueue', {'urls': urls, 'idempotencyKey': 'g03-batch'})
    b2 = c.ok('downloads_batch_enqueue', {'urls': urls, 'idempotencyKey': 'g03-batch'})
    need(json.dumps(b1['children']) == json.dumps(b2['children']), 'batch replay differs')
    need(c.call('downloads_batch_enqueue', {'urls': [fx(run, 'good.mp4', 'g03x') for _ in range(21)], 'idempotencyKey': 'g03-b21'})['error'], 'batch of 21 accepted')
    accepted = [ch for ch in b1['children'] if ch.get('accepted')]
    need(len(accepted) == 2, f'batch accepted {len(accepted)} of 2 valid URLs')
    # multi-file listing: a second produced file in the job folder is its own artifact
    did, item = enqueue_wait(c, fx(run, 'good.mp4', 'g03m'), 'g03-multi')
    need(item['status']['type'] == 'Complete', 'fixture job failed')
    job_dir = pathlib.Path(item['file_path']).parent
    shutil.copy2(job_dir / pathlib.Path(item['file_path']).name, job_dir / 'part2.mp4')
    arts = c.ok('download_artifacts', {'download_id': did})['artifacts']
    need(len(arts) == 2 and all(c.fetch(a['access'])['sha256'] == run.good_sha for a in arts), f'multi-file listing: {len(arts)} artifacts')
    notes.append('multi-file listing ok (controlled; real carousel only with --live)')
    # pause / resume / cancel on a throttled transfer
    r = c.call('download_enqueue', {'url': fx(run, 'slow.mp4', 'g03', bps=400000), 'idempotencyKey': 'g03-slow'})
    sid = c.item_id(r['sc'])
    t = time.time()
    while time.time() - t < 60 and (c.ok('download_status', {'download_id': sid})['item'].get('downloaded_bytes') or 0) < 300000:
        time.sleep(0.5)
    p = c.ok('download_pause', {'download_id': sid, 'idempotencyKey': 'g03-p'})
    need(p['item']['status']['type'] == 'Paused', 'pause did not pause')
    time.sleep(1)
    rs = c.ok('download_resume', {'download_id': sid, 'idempotencyKey': 'g03-r'})
    notes.append('resume=' + ((rs.get('resume') or {}).get('continuation') or 'in-place'))
    time.sleep(3)
    cn = c.ok('download_cancel', {'download_id': sid, 'idempotencyKey': 'g03-c'})
    time.sleep(2)
    st = c.ok('download_status', {'download_id': sid})['item']
    need(st['status'].get('data', {}).get('message') == 'Cancelled' and st.get('phase') == 'cancelled', f'cancel state {st.get("phase")}')
    need(c.ok('download_artifacts', {'download_id': sid})['artifacts'] == [], 'cancelled job lists artifacts')
    leftovers = list((A.media / f'omniget-mcp-{sid}').glob('*')) if (A.media / f'omniget-mcp-{sid}').exists() else []
    need(not leftovers, f'cancel left files: {[x.name for x in leftovers]}')
    # retry after a transient 503
    fid, item = enqueue_wait(c, fx(run, 'flaky.mp4', 'g03', fail=1), 'g03-flaky')
    need(item['status']['type'] == 'Error' and item['status']['data'].get('retryable'), f'503 not retryable: {item["status"]}')
    d = c.ok('download_diagnose', {'download_id': fid})
    nb = (d.get('retry') or {}).get('notBeforeMs') or 0
    time.sleep(max(0, nb / 1000 - time.time()) + 0.5)
    c.ok('download_retry', {'download_id': fid, 'strategyId': 'retry_transient', 'idempotencyKey': 'g03-retry'})
    need(c.wait(fid)['status']['type'] == 'Complete', 'retry after 503 did not complete')
    # logs with a cursor, history
    l1 = c.ok('download_logs', {'download_id': fid, 'limit': 2})
    l2 = c.ok('download_logs', {'download_id': fid, 'limit': 2, 'cursor': l1['nextCursor']})
    need(l1['events'] and l2['events'] and l2['events'][0]['eventId'] > l1['events'][-1]['eventId'], 'log cursor did not advance')
    hist = {x['id'] for x in c.ok('download_history', {'limit': 100})['items']}
    need({did, fid} <= hist, 'history misses finished jobs')
    # >64 MiB transfer
    if not run.big_sha:
        notes.append('big transfer skipped (no ffmpeg big fixture)')
    else:
        bid, item = enqueue_wait(c, fx(run, 'big.mp4', 'g03'), 'g03-big', limit=240)
        need(item['status']['type'] == 'Complete', f'big job failed: {item["status"]}')
        a = c.ok('download_artifacts', {'download_id': bid})['artifacts'][0]['access']
        t0 = time.time()
        got = c.fetch(a)
        need(got['status'] == 200 and got['sha256'] == run.big_sha and got['bytes'] > 64 * 1024 * 1024, f'big transfer {got.get("status")} {got.get("bytes")}')
        rg = c.fetch(a, rng='bytes=100-199')
        need(rg['status'] == 206 and rg['bytes'] == 100, 'Range request failed')
        notes.append(f'{got["bytes"] // (1 << 20)} MiB transfer in {time.time() - t0:.1f}s')
    return '; '.join(notes)


def gate_g04(run, B, D):
    c = B.client('G04', BASE_SCOPES + ['transfer', 'local_network'], root=True, network=True)
    u = fx(run, 'good.mp4', 'g04')
    r1 = c.ok('download_enqueue', {'url': u, 'idempotencyKey': 'g04-k'})
    r2 = c.ok('download_enqueue', {'url': u, 'idempotencyKey': 'g04-k'})
    need(c.item_id(r1) == c.item_id(r2), 'replay created another job')
    for extra in ({'url': fx(run, 'good.mp4', 'g04')}, {'mode': 'audio'}, {'maxHeight': 480}):
        e = c.call('download_enqueue', dict({'url': u, 'idempotencyKey': 'g04-k'}, **extra))
        need(e['sc'].get('error', {}).get('code') == 'IDEMPOTENCY_CONFLICT', f'conflict not detected for {list(extra)}')
    uc = fx(run, 'good.mp4', 'g04c')
    with concurrent.futures.ThreadPoolExecutor(12) as ex:
        res = list(ex.map(lambda _: Mcp(B, c.token).call('download_enqueue', {'url': uc, 'idempotencyKey': 'g04-conc'})['sc'], range(12)))
    ids = [c.item_id(x) for x in res]
    need(len({i for i in ids if i}) == 1, f'concurrent same key -> {len({i for i in ids if i})} jobs: {json.dumps(res[:2])[:200]}')
    pending = [x for x in res if not c.item_id(x)]
    need(all((x.get('error') or {}).get('code') == 'OUTCOME_UNKNOWN' for x in pending), f'unexpected concurrent refusal: {json.dumps(pending[:1])[:160]}')
    time.sleep(1)
    need(c.item_id(c.ok('download_enqueue', {'url': uc, 'idempotencyKey': 'g04-conc'})) in ids, 'replay after concurrent duplicates returned another job')
    conc_note = f'{len(pending)}/12 concurrent duplicates answered OUTCOME_UNKNOWN, replay -> same job' if pending else '12 concurrent duplicates -> same job'
    ud = fx(run, 'good.mp4', 'g04d')
    with concurrent.futures.ThreadPoolExecutor(6) as ex:
        res = list(ex.map(lambda k: Mcp(B, c.token).call('download_enqueue', {'url': ud, 'idempotencyKey': f'g04-d{k}'})['sc'], range(6)))
    made = {c.item_id(x) for x in res if c.item_id(x)}
    need(len(made) == 1 and all(c.item_id(x) or x.get('error', {}).get('code') == 'URL_ALREADY_MANAGED' for x in res), f'same URL, different keys -> {len(made)} jobs')
    # two different URLs that differ only in a query value are two jobs
    base = f'http://127.0.0.1:{run.fx_port}/g04-q{run.next()}/slow.mp4?bps=200000&x='
    q1 = c.call('download_enqueue', {'url': base + 'one', 'idempotencyKey': 'g04-q1'})
    qpath = (base + 'one').split(f':{run.fx_port}', 1)[1]
    t = time.time()
    while time.time() - t < 60 and run.hits(qpath) == 0:  # the first one is really running
        time.sleep(0.3)
    q2 = c.call('download_enqueue', {'url': base + 'two', 'idempotencyKey': 'g04-q2'})
    for q in (q1, q2):
        if c.item_id(q['sc']):
            c.call('download_cancel', {'download_id': c.item_id(q['sc']), 'idempotencyKey': 'g04-qc' + str(c.item_id(q['sc']))})
    soft = []
    if not (c.item_id(q1['sc']) and c.item_id(q2['sc'])):
        soft.append(f'a second URL differing only in a query value was refused ({(q2["sc"].get("error") or {}).get("code")})')
    # lost response: the request is sent and the socket closed before the reply
    ul = fx(run, 'good.mp4', 'g04l')
    body = json.dumps({'jsonrpc': '2.0', 'id': 9, 'method': 'tools/call', 'params': {'name': 'download_enqueue', 'arguments': {'url': ul, 'idempotencyKey': 'g04-lost'}}}).encode()
    s = socket.create_connection(('127.0.0.1', B.port))
    s.sendall(b'POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer ' + c.token.encode() + b'\r\nContent-Type: application/json\r\n'
              b'Accept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2025-06-18\r\nContent-Length: ' + str(len(body)).encode() + b'\r\n\r\n' + body)
    s.close()
    time.sleep(2)
    lost = c.item_id(c.ok('download_enqueue', {'url': ul, 'idempotencyKey': 'g04-lost'}))
    need(lost, 'replay after a lost response returned no job')
    # kill -9 with a job in flight
    hang = fx(run, 'hang.mp4', 'g04')
    hid = c.item_id(c.ok('download_enqueue', {'url': hang, 'idempotencyKey': 'g04-hang'}))
    path = hang.split(f':{run.fx_port}', 1)[1]
    t = time.time()
    while time.time() - t < 90 and run.hits(path) == 0:
        time.sleep(0.5)
    need(run.hits(path) > 0, 'in-flight job never reached the fixture')
    time.sleep(1.5)
    kids, survivors = B.kill9()
    B.start()
    st = c.ok('download_status', {'download_id': hid})['item']
    need(st['status']['type'] == 'Unknown' and st['status']['data'].get('interrupted'), f'after restart: {json.dumps(st["status"])[:120]}')
    need(any(x['id'] == hid for x in c.ok('downloads_queue', {'limit': 100})['items']), 'interrupted job not visible in the queue')
    e = c.call('download_enqueue', {'url': hang, 'idempotencyKey': 'g04-hang-2'})
    need(e['sc'].get('error', {}).get('code') == 'URL_OUTCOME_UNKNOWN', f'new key for an unknown URL: {e["sc"]}')
    need(c.call('download_retry', {'download_id': hid, 'strategyId': 'retry_transient', 'idempotencyKey': 'g04-rt0'})['isError'], 'blind retry allowed before reconcile')
    h0 = run.hits(path)
    rec = c.ok('download_retry', {'download_id': hid, 'strategyId': 'reconcile', 'idempotencyKey': 'g04-rec'})
    rec2 = c.ok('download_retry', {'download_id': hid, 'strategyId': 'reconcile', 'idempotencyKey': 'g04-rec'})
    need(rec.get('item', {}).get('status', {}).get('type') == 'Error', f"reconcile did not settle as Error: outcome={rec.get('outcome')} status={json.dumps(rec.get('item', {}).get('status'))[:160]} files={json.dumps((rec.get('receipt') or {}).get('files'))[:200]}")
    need(json.dumps(rec2) == json.dumps(rec), f'reconcile replay differs: {json.dumps(rec)[:200]} vs {json.dumps(rec2)[:200]}')
    time.sleep(1)
    need(run.hits(path) == h0, 'reconcile contacted the source')
    nb = max([a.get('notBeforeMs') or 0 for a in rec.get('nextActions', [])] or [0])
    time.sleep(max(0, nb / 1000 - time.time()) + 0.5)
    rt = c.ok('download_retry', {'download_id': hid, 'strategyId': 'retry_transient', 'idempotencyKey': 'g04-rt'})
    need(c.item_id(rt) == hid and rt.get('attempt') == 1, 'retry after reconcile created another job')
    time.sleep(2)
    c.ok('download_cancel', {'download_id': hid, 'idempotencyKey': 'g04-cx'})
    q = [x for x in c.ok('downloads_queue', {'limit': 100})['items'] if x['id'] == hid]
    need(len(q) == 1, f'{len(q)} queue rows for the reconciled job')
    need(not soft, '; '.join(soft) + ' [the kill -9/reconcile checks passed]')
    return f'replay/conflict ok; {conc_note}; lost response ok; kill -9 -> Unknown -> reconcile (0 source hits) -> retry same id; {len(kids)} worker(s) at kill, {len(survivors)} outlived 15s'


def gate_g05(run, C, D):
    c = C.client('G05', BASE_SCOPES + ['transfer', 'local_network', 'auth'], root=True, network=True)
    lid, item = enqueue_wait(c, fx(run, 'limited.mp4', 'g05', ra=3), 'g05-lim')
    need(item['status']['type'] == 'Error' and item['status']['data'].get('retryable'), f'429 not retryable: {item["status"]}')
    aid, item = enqueue_wait(c, fx(run, 'secret401.mp4', 'g05'), 'g05-auth')
    d = c.ok('download_diagnose', {'download_id': lid})
    nb = (d.get('retry') or {}).get('notBeforeMs') or 0
    time.sleep(min(90, max(0, nb / 1000 - time.time())) + 0.5)
    c.ok('download_retry', {'download_id': lid, 'strategyId': 'retry_transient', 'idempotencyKey': 'g05-r1'})
    c.wait(lid)
    C.stop()
    C.start()
    d = c.ok('download_diagnose', {'download_id': lid})
    size = len(json.dumps(d))
    need(d['diagnosis']['code'] == 'RATE_LIMITED', f'429 diagnosed as {d["diagnosis"]["code"]}')
    need(len(d.get('attempts', [])) >= 2, f'attempts after restart: {len(d.get("attempts", []))}')
    need(d.get('nextActions'), 'no typed next action')
    need(size <= 8192, f'diagnosis {size} B > 8 KiB')
    da = c.ok('download_diagnose', {'download_id': aid})
    need(da['diagnosis']['code'] == 'AUTH_REQUIRED', f'401 diagnosed as {da["diagnosis"]["code"]}')
    need(any(a.get('tool') == 'auth_connection_request' or a.get('action') in ('local_user_action', 'connect_account') for a in da.get('nextActions', [])), 'no login action for AUTH_REQUIRED')
    b = c.ok('diagnostic_bundle_create', {'download_id': lid, 'idempotencyKey': 'g05-b'})
    need(len(json.dumps(b)) <= 48 * 1024 and b.get('manifest') and b.get('bundleId'), 'bundle too large or incomplete')
    l1 = c.ok('download_logs', {'download_id': lid, 'limit': 3})
    l2 = c.ok('download_logs', {'download_id': lid, 'limit': 3, 'cursor': l1['nextCursor']})
    need(l1['events'] and l2['events'] and l2['events'][0]['eventId'] > l1['events'][-1]['eventId'], 'log cursor did not advance after restart')
    need('priorSessionGaps' in l2, 'log page does not report prior-session gaps')
    return f'after restart: RATE_LIMITED with {len(d["attempts"])} attempts, {len(d["nextActions"])} action(s), {size} B; AUTH_REQUIRED -> login action; bundle {len(json.dumps(b))} B'


def gate_g06_jobs(run, A, D):
    s = A.client('G06', BASE_SCOPES + ['transfer', 'local_network'], root=True, network=True)
    other = A.client('G06b', BASE_SCOPES + ['transfer'])
    S = run.secrets
    urls = {
        'signed': fx(run, 'secret-good.mp4', 'g06', **{'X-Amz-Signature': S['synthsig'], 'access_token': S['synthtok'], 'Policy': S['synthpass']}),
        'auth': fx(run, 'secret401.mp4', 'g06', token=S['synthtok']),
        'forbidden': fx(run, 'secret403.mp4', 'g06', sig=S['synthsig']),
        'unknown-param': fx(run, 'good.mp4', 'g06', igsh=S['synthtok']),
    }
    urls['fragment'] = fx(run, 'good.mp4', 'g06') + '#frag=' + S['synthcookie']
    ids = {}
    for k, u in urls.items():
        r = s.call('download_enqueue', {'url': u, 'idempotencyKey': 'g06-' + k})
        ids[k] = s.item_id(r['sc'])
    for k, did in ids.items():
        if did:
            s.wait(did)
            for tool, extra in (('download_status', {}), ('download_diagnose', {}), ('download_logs', {'limit': 100}),
                                ('download_artifacts', {}), ('download_recovery_options', {})):
                s.call(tool, dict({'download_id': did}, **extra))
            b = s.call('diagnostic_bundle_create', {'download_id': did, 'idempotencyKey': 'g06-b-' + k})
            need(not b['isError'], f'bundle failed for {k}')
    s.call('downloads_queue', {'limit': 100})
    s.call('download_history', {'limit': 100})
    run.g06_ids = [x for x in ids.values() if x]
    target = ids['signed']
    art = s.ok('download_artifacts', {'download_id': target})['artifacts'][0]
    for tool, args in (('download_status', {}), ('download_logs', {}), ('download_diagnose', {}), ('download_artifacts', {}),
                       ('download_cancel', {'idempotencyKey': 'x'}), ('download_retry', {'strategyId': 'reconcile', 'idempotencyKey': 'y'}),
                       ('diagnostic_bundle_create', {'idempotencyKey': 'z'})):
        e = other.call(tool, dict({'download_id': target}, **args))
        need(e['isError'] and e['sc'].get('error', {}).get('code') == 'DOWNLOAD_NOT_FOUND', f'client B reached A via {tool}')
    need(other.call('artifact_access', {'artifactId': art['artifactId'], 'mode': 'transfer'})['isError'], 'client B got artifact access')
    need(not other.ok('downloads_queue', {'limit': 100})['items'] and not other.ok('download_history', {'limit': 100})['items'], 'client B sees A\'s jobs')
    need(other.fetch(art['access'], token=other.token)['status'] == 404, 'client B transferred A\'s file')
    return len(run.g06_ids)


def gate_g06_scan(run, A, D):
    needles = {k: v.split('_', 1)[1].encode() for k, v in run.secrets.items()}

    def hits(b):
        return sorted(k for k, n in needles.items() if n in b)
    found = {}
    files = 0
    for root in (A.profile, A.home):
        for dp, _, fs in os.walk(root):
            for f in fs:
                p = os.path.join(dp, f)
                if f == 'tokens.json' or os.path.islink(p):
                    continue
                files += 1
                try:
                    with open(p, 'rb') as fh:
                        h = hits(fh.read())
                except OSError:
                    continue
                if h:
                    found[os.path.relpath(p, A.dir)] = h
    for p in A.dir.glob('app-*.log'):
        h = hits(p.read_bytes())
        if h:
            found[p.name] = h
    with run.lock:
        blobs = list(run.responses)
    rh = sorted({k for b in blobs for k in hits(b)})
    if rh:
        found['mcp-responses'] = rh
    c = Mcp(A, None)
    for path in ['/v1/queue'] + [f'/v1/log/{i}' for i in run.g06_ids]:
        code, raw, _ = c.raw('GET', path, token='ext')
        h = hits(raw)
        if h:
            found['bridge ' + path.split('/')[2]] = h
    with A.ui_lock:
        ui = A.eval('return {events: JSON.stringify(window.__gatesEvents||[]), n:(window.__gatesEvents||[]).length, hist: JSON.stringify(await inv("get_download_history")), rec: JSON.stringify(await inv("get_recovery_items")), dom: document.body.innerText}', timeout=60)
    for k in ('events', 'hist', 'rec', 'dom'):
        h = hits(ui.get(k, '').encode())
        if h:
            found['ui ' + k] = h
    need(ui.get('n', 0) > 0, 'no window events were captured')
    need(not found, 'synthetic secrets found in: ' + '; '.join(f'{k}={v}' for k, v in found.items()))
    return f'0 hits: {files} profile/home files, app log, {len(blobs)} MCP responses, {ui.get("n")} window events, UI history/recovery/DOM, /v1/queue + {len(run.g06_ids)} logs; client isolation ok'


def gate_g07(run, A, D):
    c = A.client('G07', BASE_SCOPES + ['transfer', 'local_network', 'auth'], root=True, network=True)
    init = lambda v: {'protocolVersion': v, 'capabilities': {}, 'clientInfo': {'name': 'gates', 'version': '1'}}
    for asked, want in (('2025-06-18', '2025-06-18'), ('2025-03-26', '2025-03-26'), ('2099-01-01', '2025-06-18')):
        code, r = c.rpc('initialize', init(asked))
        need(code == 200 and r['result']['protocolVersion'] == want and r['result'].get('serverInfo'), f'initialize {asked}')
    checks = [
        (c.raw('POST', '/mcp', {'jsonrpc': '2.0', 'id': 1, 'method': 'ping'}, {'MCP-Protocol-Version': '2099-01-01'})[0], 400, 'bad revision header'),
        (c.raw('GET', '/mcp')[0], 405, 'GET'), (c.raw('POST', '/mcp', {'jsonrpc': '2.0', 'method': 'notifications/initialized'})[0], 202, 'notification'),
        (c.raw('POST', '/mcp', b'{bad')[0], 400, 'parse error'),
        (c.raw('POST', '/mcp', {'jsonrpc': '2.0', 'id': 1, 'method': 'ping'}, {'Content-Type': 'text/plain'})[0], 415, 'content type'),
        (c.raw('POST', '/mcp', {'jsonrpc': '2.0', 'id': 1, 'method': 'ping'}, {'Accept': 'application/json'})[0], 406, 'accept'),
        (c.raw('POST', '/mcp', b'{"jsonrpc":"2.0","id":1,"method":"ping","params":{"x":"' + b'a' * 70000 + b'"}}')[0], 413, 'body limit')]
    for got, want, what in checks:
        need(got == want, f'{what}: HTTP {got}, expected {want}')
    for method, params, code in (('nope', None, -32601), ('tools/call', {'name': 'agent_delegate', 'arguments': {}}, -32602),
                                 ('tools/call', {'name': 'download_status', 'arguments': {'download_id': 1, 'extra': 1}}, -32602)):
        _, r = c.rpc(method, params)
        need((r.get('error') or {}).get('code') == code, f'{method} -> {r.get("error")}')
    _, r = c.rpc('ping')
    for bad, what in (({'jsonrpc': '1.0', 'id': 1, 'method': 'ping'}, 'jsonrpc 1.0'), ({'jsonrpc': '2.0', 'id': None, 'method': 'ping'}, 'null id')):
        need(json.loads(c.raw('POST', '/mcp', bad)[1]).get('error', {}).get('code') == -32600, what)
    biz = c.call('download_status', {'download_id': 999999999})
    need(biz['isError'] and biz['sc'].get('error', {}).get('code') == 'DOWNLOAD_NOT_FOUND' and biz['text'], 'business error shape')
    # batches
    code, raw, _ = c.raw('POST', '/mcp', [{'jsonrpc': '2.0', 'id': 'a', 'method': 'ping'}, {'jsonrpc': '2.0', 'id': 'b', 'method': 'tools/list'}, {'jsonrpc': '2.0', 'method': 'notifications/x'}])
    need(code == 200 and sorted(m['id'] for m in json.loads(raw)) == ['a', 'b'], 'HTTP batch')
    need(json.loads(c.raw('POST', '/mcp', [])[1])['error']['code'] == -32600, 'empty batch')
    need(json.loads(c.raw('POST', '/mcp', [{'jsonrpc': '2.0', 'id': i, 'method': 'ping'} for i in range(33)])[1])['error']['code'] == -32600, 'oversized batch')
    # tools/list: every tool has input and output schemas
    _, tl = c.rpc('tools/list')
    tools = {t['name']: t for t in tl['result']['tools']}
    missing = [n for n, t in tools.items() if not isinstance(t.get('outputSchema'), dict) or t['inputSchema'].get('type') != 'object']
    need(not missing, f'tools without outputSchema/object inputSchema: {missing}')
    did, item = enqueue_wait(c, fx(run, 'good.mp4', 'g07'), 'g07-e')
    fid, _ = enqueue_wait(c, fx(run, 'missing.mp4', 'g07'), 'g07-f')
    art = c.ok('download_artifacts', {'download_id': did})['artifacts'][0]
    calls = [('omniget_capabilities', {}), ('omniget_health', {}), ('destinations_list', {}), ('downloads_queue', {'limit': 50}),
             ('download_history', {'limit': 50}), ('download_status', {'download_id': did}), ('download_status', {'download_id': fid}),
             ('download_wait', {'download_id': did, 'timeoutMs': 100}), ('download_artifacts', {'download_id': did}),
             ('artifact_access', {'artifactId': art['artifactId'], 'mode': 'metadata'}), ('artifact_access', {'artifactId': art['artifactId'], 'mode': 'transfer'}),
             ('download_logs', {'download_id': fid, 'limit': 5}), ('download_diagnose', {'download_id': fid}), ('download_diagnose', {'download_id': did}),
             ('download_recovery_options', {'download_id': fid}), ('diagnostic_bundle_create', {'download_id': fid, 'idempotencyKey': 'g07-b'}),
             ('media_inspect', {'url': fx(run, 'good.mp4', 'g07')}), ('downloads_preflight', {'urls': [fx(run, 'good.mp4', 'g07'), 'x']}),
             ('media_collection_list', {'url': fx(run, 'good.mp4', 'g07'), 'limit': 2}), ('download_enqueue', {'url': item['url'], 'idempotencyKey': 'g07-e'}),
             ('downloads_batch_enqueue', {'urls': [fx(run, 'good.mp4', 'g07')], 'idempotencyKey': 'g07-bb'})]
    if 'auth_connection_status' in tools:
        calls.append(('auth_connection_status', {'platform': 'youtube'}))
    validated = 0
    for name, args in calls:
        if name not in tools:
            continue
        r = c.call(name, args)
        if (r['sc'].get('error') or {}).get('code') == 'WORKER_DEPENDENCY_UNAVAILABLE':
            continue
        need(not r['isError'] and not r['error'], f'{name} failed: {json.dumps(r["sc"] or r["error"])[:150]}')
        errs = validate(tools[name]['outputSchema'], r['sc'])
        need(not errs, f'{name} structuredContent violates outputSchema: {errs[:3]}')
        need(json.loads(r['text']) == r['sc'], f'{name}: text differs from structuredContent')
        validated += 1
    # stdio adapter: batch, clean stdout, offline behaviour, bad endpoint
    notes = [f'{len(tools)} tools with outputSchema, {validated} live results validated']
    if run.mcp_bin:
        s = stdio_session(run, A, c.token, [{'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': init('2025-03-26')}, 'NOT JSON',
                                            [{'jsonrpc': '2.0', 'id': 'b1', 'method': 'ping'}, {'jsonrpc': '2.0', 'id': 'b2', 'method': 'ping'}],
                                            {'jsonrpc': '2.0', 'id': 3, 'result': {}}, {'jsonrpc': '2.0', 'id': 4, 'method': 'nope'}])
        ids = [m.get('id') if isinstance(m, dict) else sorted(x.get('id') for x in m) for m in s['messages']]
        need(not s['nonJson'] and ['b1', 'b2'] in ids and 3 not in ids and 4 in ids and None in ids, f'stdio session ids {ids}')
        off = stdio_session(run, A, c.token, [{'jsonrpc': '2.0', 'id': 1, 'method': 'ping'}, {'jsonrpc': '2.0', 'id': 2, 'result': {}}], port=free_port())
        need([m.get('id') for m in off['messages']] == [1] and off['messages'][0]['error']['message'] == 'APP_NOT_RUNNING', f'offline adapter: {off["messages"]}')
        bad = stdio_session(run, A, c.token, [], env_url='http://evil.example/mcp')
        need(bad['exit'] != 0 and not bad['messages'], 'adapter accepted a non-loopback endpoint')
        notes.append('stdio batch/offline/endpoint ok')
    sdk = pathlib.Path('/tmp/omniget-mcp-sdk/node_modules')
    if sdk.exists() and shutil.which('node') and run.mcp_bin:
        work = run.out / 'sdk'
        work.mkdir(exist_ok=True)
        if not (work / 'node_modules').exists():
            (work / 'node_modules').symlink_to(sdk)
        (work / 'check.mjs').write_text(SDK_CHECK)
        results = []
        for mode in ('http', 'stdio'):
            p = subprocess.run(['node', str(work / 'check.mjs'), mode, f'http://127.0.0.1:{A.port}/mcp', run.mcp_bin, str(did)],
                               capture_output=True, text=True, timeout=120, env=dict(os.environ, GATES_TOKEN=c.token))
            try:
                results.append(json.loads(p.stdout.strip().splitlines()[-1]))
            except (ValueError, IndexError):
                results.append({'mode': mode, 'error': (p.stderr or p.stdout)[:200]})
        bad = [r for r in results if r.get('error') or r.get('failures')]
        need(not bad, f'official SDK: {json.dumps(bad)[:300]}')
        notes.append('SDK ' + '/'.join(f'{r["mode"]}:{r["calls"]} calls' for r in results))
    return '; '.join(notes)


SPA_SERVER = r"""import http.server, os, sys
root = os.path.abspath(sys.argv[1])
class H(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k): super().__init__(*a, directory=root, **k)
    def log_message(self, *a): pass
    def send_head(self):
        p = self.translate_path(self.path)
        if not os.path.exists(p): self.path = '/index.html'
        return super().send_head()
http.server.ThreadingHTTPServer(('127.0.0.1', 1420), H).serve_forever()
"""

SDK_CHECK = r"""import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
const [mode, url, bin, did] = process.argv.slice(2); const tok = process.env.GATES_TOKEN;
const t = mode === "http" ? new StreamableHTTPClientTransport(new URL(url), { requestInit: { headers: { Authorization: `Bearer ${tok}` } } })
  : new StdioClientTransport({ command: bin, args: [], env: { OMNIGET_MCP_URL: url, OMNIGET_MCP_TOKEN: tok, PATH: "/usr/bin:/bin" } });
const c = new Client({ name: "gates-sdk-" + mode, version: "1" }); const out = { mode, calls: 0, failures: [] };
try { await c.connect(t); const tools = (await c.listTools()).tools;
  for (const [n, a] of [["omniget_health", {}], ["omniget_capabilities", {}], ["downloads_queue", { limit: 10 }], ["download_status", { download_id: Number(did) }], ["download_artifacts", { download_id: Number(did) }]]) {
    try { const r = await c.callTool({ name: n, arguments: a }); out.calls++; if (r.isError) out.failures.push(n); } catch (e) { out.failures.push(n + ": " + String(e).slice(0, 120)); } }
  out.tools = tools.length; } catch (e) { out.error = String(e).slice(0, 200); }
await c.close().catch(() => {}); console.log(JSON.stringify(out));
"""


def gate_g09(run, A, D):
    c = A.client('G09', BASE_SCOPES)
    g = A.client('G09n', BASE_SCOPES + ['local_network'], network=True)
    routes = [('GET', '/v1/pair', None, (403,)), ('POST', '/v1/enqueue', {'url': 'https://example.com/a.mp4'}, (401,)),
              ('GET', '/v1/queue', None, (401,)), ('GET', '/v1/log/1', None, (401,)), ('POST', '/v1/cookies', {'domain': 'x'}, (401,)),
              ('POST', '/v1/debug/eval', b'return 1', (401,)), ('POST', '/mcp/assist', {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/list'}, (401,)),
              ('GET', '/v1/models', None, (401,)), ('GET', '/v1/jobs', None, (401,)), ('GET', '/v1/agents', None, (401,)), ('GET', '/v1/loops', None, (401,)),
              ('POST', '/v1/agent/run', {'agent_id': 'omni', 'prompt': 'noop'}, (401,)), ('POST', '/v1/hooks/x', {}, (401,))]
    for method, path, body, want in routes:
        code, _, _ = c.raw(method, path, body, {'Content-Type': 'application/json'} if body is not None and not path.startswith('/mcp') else None)
        need(code in want, f'{method} {path} with an MCP token -> {code}')
    ping = {'jsonrpc': '2.0', 'id': 1, 'method': 'ping'}
    need(c.raw('POST', '/mcp', ping, token='ext')[0] == 401, 'extension token accepted on /mcp')
    for name, hdr in (('origin', {'Origin': 'https://evil.example'}), ('origin-null', {'Origin': 'null'}), ('origin-local', {'Origin': 'http://localhost:1420'}),
                      ('host', {'Host': 'evil.example'}), ('rebind', {'Host': f'127.0.0.1.nip.io:{A.port}'}), ('userinfo', {'Host': 'a@127.0.0.1'}),
                      ('any', {'Host': f'0.0.0.0:{A.port}'})):
        need(c.raw('POST', '/mcp', ping, hdr)[0] == 403, f'{name} header not refused on /mcp')
        need(c.raw('GET', '/mcp/artifacts/x', None, hdr)[0] == 403, f'{name} header not refused on artifacts')
    for u in (f'http://127.0.0.1:{run.fx_port}/g09a/good.mp4', f'http://[::1]:{run.fx_port}/good.mp4', 'http://169.254.169.254/latest/meta-data/',
              f'http://10.0.0.1:{run.fx_port}/x.mp4', f'http://2130706433:{run.fx_port}/x.mp4', f'http://localtest.me:{run.fx_port}/x.mp4', 'file:///etc/passwd'):
        e = c.call('download_enqueue', {'url': u, 'idempotencyKey': 'g09-' + hashlib.sha1(u.encode()).hexdigest()[:10]})
        need(e['isError'] and e['sc'].get('error', {}).get('code') in ('LOCAL_NETWORK_NOT_GRANTED', 'INVALID_URL', 'DNS_FAILURE', 'UNSUPPORTED_URL'),
             f'private target {u} accepted: {e["sc"]}')
    blocked = 0
    for name in ('redirect-canary.mp4', 'redirect-meta.mp4', 'hls.m3u8'):
        did, item = enqueue_wait(g, fx(run, name, 'g09'), 'g09-' + name)
        need(item['status']['type'] == 'Error', f'{name} completed through the egress policy')
        blocked += 'EGRESS_BLOCKED' in json.dumps(item['status'])
    did, item = enqueue_wait(g, f'http://127.0.0.1:{run.canary_port}/g09/good.mp4', 'g09-canary')
    need(item['status']['type'] == 'Error', 'direct ungranted port completed')
    did, item = enqueue_wait(g, fx(run, 'redirect-ok.mp4', 'g09'), 'g09-ok')
    need(item['status']['type'] == 'Complete', 'same-origin redirect failed')
    need(run.hits(port='canary') == 0, f'canary reached {run.hits(port="canary")} time(s)')
    return f'{len(routes)} bridge routes refused, Origin/Host refused, private targets refused, egress blocked ({blocked + 1}/4 labelled EGRESS_BLOCKED), canary 0 hits'


def gate_g10(run, A, D):
    f = A.client('G10', BASE_SCOPES + ['transfer', 'local_network'], root=True, network=True)
    need(f.call('download_enqueue', {'url': fx(run, 'good.mp4', 'g10'), 'idempotencyKey': 'g10-d', 'destinationId': '/tmp'})['error'], 'foreign destination accepted')
    stamp = secrets.token_hex(4)
    for name in ('cd-traversal.mp4', 'cd-abs.mp4', f'a/b/../../gv-escape-{stamp}.mp4', f'x/..%2F..%2Fgv-escape-{stamp}.mp4'):
        did, item = enqueue_wait(f, fx(run, name, 'g10'), 'g10-' + hashlib.sha1(name.encode()).hexdigest()[:8])
        if item['status']['type'] == 'Complete':
            need(pathlib.Path(item['file_path']).parent == A.media / f'omniget-mcp-{did}', f'{name} written outside its job folder')
            need(f.ok('download_artifacts', {'download_id': did})['artifacts'], f'{name}: completed without an artifact')
    escaped = [p for p in list(pathlib.Path('/tmp').glob('gv-*escape*')) + [p for p in A.profile.rglob('gv-*') if '/omniget-mcp-' not in str(p)]]
    need(not escaped, f'files escaped: {escaped}')
    did, item = enqueue_wait(f, fx(run, 'good.mp4', 'g10s'), 'g10-sym')
    a = f.ok('download_artifacts', {'download_id': did})['artifacts'][0]['access']
    target = pathlib.Path(item['file_path'])
    outside = run.out / 'outside.txt'
    outside.write_text('outside secret file')
    target.rename(str(target) + '.orig')
    target.symlink_to(outside)
    need(f.fetch(a)['status'] in (404, 412), 'symlink swapped artifact was served')
    target.unlink()
    pathlib.Path(str(target) + '.orig').rename(target)
    b = bytearray(target.read_bytes())
    b[1000] ^= 0xFF
    target.write_bytes(bytes(b))
    need(f.fetch(a)['status'] == 412, 'modified artifact was served')
    need(f.fetch(a, if_match=False)['status'] == 428, 'transfer without If-Match accepted')
    fd = A.client('G10d', BASE_SCOPES + ['transfer', 'local_network'], network=True)
    fe = A.client('G10e', BASE_SCOPES + ['local_network'], root=True, network=True)
    for cl, what in ((fd, 'transfer scope without folder grant'), (fe, 'folder grant without transfer scope')):
        did, _ = enqueue_wait(cl, fx(run, 'good.mp4', 'g10g'), 'g10-g')
        arts = cl.ok('download_artifacts', {'download_id': did})['artifacts']
        need(arts and not arts[0].get('transferAllowed') and 'access' not in arts[0], f'{what} still offered a transfer')
    fr = A.client('G10r', BASE_SCOPES + ['transfer', 'local_network'], root=True, network=True)
    src = 'big.mp4' if run.big_sha else 'good.mp4'
    did, _ = enqueue_wait(fr, fx(run, src, 'g10r'), 'g10-r', limit=240)
    a = fr.ok('download_artifacts', {'download_id': did})['artifacts'][0]['access']
    req = urllib.request.Request(fr.base + a['downloadPath'], headers={'Authorization': 'Bearer ' + fr.token, 'If-Match': '"' + a['digest'] + '"'})
    n = 0
    with urllib.request.urlopen(req, timeout=60) as r:
        n += len(r.read(1 << 20))
        A.inv('tool_mcp_root_revoke', {'id': A.principals['G10r']['root']})
        try:
            while True:
                ch = r.read(1 << 16)
                if not ch:
                    break
                n += len(ch)
        except Exception:
            pass
    need(n < a['bytes'] or not run.big_sha, f'stream continued after root revocation ({n} of {a["bytes"]})')
    need(fr.fetch(a)['status'] == 404, 'transfer allowed after root revocation')
    A.inv('tool_mcp_client_revoke', {'id': A.principals['G10r']['id']})
    need(fr.fetch(a)['status'] == 401 and fr.rpc('tools/list')[0] == 401, 'revoked client still authenticated')
    return f'destination/traversal/symlink/modify/If-Match/grant checks ok; root revoke cut stream at {n // 1024} KiB of {a["bytes"] // 1024}; client revoke -> 401'


def gate_g11(run, A, D):
    # Keep every eval short: while /v1/debug/eval runs, the bridge answers nothing else.
    nav = """const s=byText('Skip setup'); if(s){s.click(); await sleep(600);}
const home=[...document.querySelectorAll('a')].find(x=>x.getAttribute('href')==='/llm'); if(!home) return {err:'no /llm link'}; home.click(); await sleep(1000);"""
    nav += """
for (const t of ['Configure','MCP','Your endpoint']) { let b=null; for(let i=0;i<40&&!b;i++){ b=byText(t); if(!b) await sleep(500); } if(!b) return {err:'no '+t}; b.click(); await sleep(800); }
return {ok:true};"""
    t0 = time.time()
    while time.time() - t0 < 120:
        if A.eval("return !![...document.querySelectorAll('a')].find(x=>x.getAttribute('href')==='/llm')", timeout=5) is True:
            break
        time.sleep(1)
    with A.ui_lock:
        r = A.eval(nav, timeout=90)
        need(r.get('ok'), f'navigation: {r}')
        created = A.eval("""const m=document.querySelector('main'); const inp=[...m.querySelectorAll('input')].find(i=>(i.closest('label')?.textContent||'').trim().startsWith('Client name'));
if(!inp) return {err:'no client name field'}; await type(inp,'gates UI client'); const b=byText('Create connection',m); if(!b||b.disabled) return {err:'create disabled'}; b.click(); await sleep(2500);
const sel=[...m.querySelectorAll('select')].find(s=>[...s.options].some(o=>o.text==='Codex')); if(sel){sel.value=[...sel.options].find(o=>o.text==='Codex').value; sel.dispatchEvent(new Event('change',{bubbles:true})); await sleep(300);}
const show=byText('Show',m); if(show){show.click(); await sleep(500);} const snip=[...m.querySelectorAll('pre,code,textarea')].map(e=>e.innerText||e.value).join('\\n'); const hide=byText('Hide',m); if(hide) hide.click();
return {msg:(txt().match(/[^\\n]*Connection created[^\\n]*/)||[''])[0], snip};""", timeout=40)
        need('Connection created' in created.get('msg', ''), f'create: {created.get("err") or created.get("msg")}')
        m = re.search(r'Bearer ([A-Za-z0-9_\-]{30,})', created.get('snip', ''))
        need(m, 'token not shown after "Show"')
        created['snip'] = '<redacted>'
        tok = m.group(1)
        c = Mcp(A, tok)
        need(c.rpc('tools/list')[0] == 200, 'UI-created token does not authenticate')
        test = A.eval("""const m=document.querySelector('main'); byText('Test',m).click(); await sleep(4000); return (txt().match(/20\\d\\d-\\d\\d-\\d\\d · \\d+ tools · http[^\\n]*/)||[''])[0];""", timeout=30)
        need(test, 'Test button showed no result')
        rv = A.eval("""const m=document.querySelector('main');
const revokes=()=>[...m.querySelectorAll('button')].filter(x=>x.innerText.trim()==='Revoke');
const row=b=>{let p=b; while(p.parentElement && [...p.parentElement.querySelectorAll('button')].filter(x=>x.innerText.trim()==='Revoke').length===1) p=p.parentElement; return p;};
const mine=()=>revokes().filter(b=>row(b).innerText.includes('gates UI client'));
const b=mine()[0]; if(!b) return {err:'no revoke row'}; b.click(); await sleep(2500);
return {msg:(txt().match(/Connection revoked\\./)||[''])[0], still: mine().length>0};""", timeout=30)
    need(rv.get('msg') and not rv.get('still'), f'revoke: {rv}')
    need(c.rpc('tools/list')[0] == 401, 'revoked UI token still works')
    return f'UI create -> token works; Test: "{test}"; Revoke -> "Connection revoked." and 401'


# ── orchestration ──────────────────────────────────────────────────────

class Run:
    def __init__(self, args):
        self.args = args
        self.out = pathlib.Path(args.out or tempfile.mkdtemp(prefix='omniget-gates-')).resolve()
        self.out.mkdir(parents=True, exist_ok=True)
        os.chmod(str(self.out), 0o700)
        self.lock = threading.Lock()
        self.responses = []
        self.own_pids = set()
        self.counter = 0
        self.fixture = None
        self.results = {}
        self.g06_ids = []
        app = pathlib.Path(args.app).resolve()
        wdir = pathlib.Path(args.worker_dir).resolve() if args.worker_dir else app.parent
        if not (app.parent / 'omniget-worker').exists():
            if not (wdir / 'omniget-worker').exists():
                raise SystemExit(f'omniget-worker not found next to {app} or in {wdir}')
            bindir = self.out / 'bin'
            bindir.mkdir(exist_ok=True)
            for f in [app] + [wdir / n for n in ('omniget-worker', 'omniget-mcp') if (wdir / n).exists()]:
                shutil.copy2(f, bindir / f.name)
            app = bindir / app.name
        self.app = str(app)
        mcp = app.parent / 'omniget-mcp'
        self.mcp_bin = str(mcp) if mcp.exists() else (str(wdir / 'omniget-mcp') if (wdir / 'omniget-mcp').exists() else None)
        self.secrets = {k: f'{k.upper()}_{secrets.token_hex(12)}' for k in ('synthtok', 'synthcookie', 'synthsig', 'synthpass', 'synthbody', 'synthhdr')}

    def next(self):
        with self.lock:
            self.counter += 1
            return self.counter

    def record(self, blob):
        with self.lock:
            self.responses.append(bytes(blob))

    def hits(self, path=None, port=None):
        try:
            lines = [json.loads(l) for l in open(self.hits_file)]
        except OSError:
            return 0
        return sum(1 for h in lines if (path is None or h['path'] == path) and (port is None or h['port'] == port))

    def start_frontend(self):
        mode = self.args.frontend
        try:
            with socket.create_connection(('127.0.0.1', 1420), timeout=1):
                return 'already running on :1420'
        except OSError:
            pass
        if mode == 'none':
            return 'none'
        repo = HERE.parent.parent
        log = open(self.out / 'frontend.log', 'ab')
        if mode == 'auto':
            vite = repo / 'node_modules' / '.bin' / 'vite'
            if not vite.exists():
                raise SystemExit('nothing on :1420 and no node_modules/.bin/vite; pass --frontend <built dir>')
            self.frontend = subprocess.Popen([str(vite), 'dev', '--port', '1420', '--strictPort'], cwd=str(repo), stdout=log, stderr=subprocess.STDOUT)
        else:
            self.frontend = subprocess.Popen([sys.executable, '-c', SPA_SERVER, mode], stdout=log, stderr=subprocess.STDOUT)
        t = time.time()
        while time.time() - t < 90:
            try:
                with urllib.request.urlopen('http://localhost:1420/', timeout=2) as r:
                    if r.status == 200:
                        return f'started ({"vite dev" if mode == "auto" else mode}, pid {self.frontend.pid})'
            except Exception:
                time.sleep(0.5)
        raise SystemExit('frontend on :1420 did not come up (see frontend.log)')

    def start_fixture(self):
        d = self.out / 'fixtures'
        d.mkdir(exist_ok=True)
        ff = shutil.which('ffmpeg') or '/opt/homebrew/bin/ffmpeg'
        if not os.path.exists(ff):
            raise SystemExit('ffmpeg is required to build the fixture media')
        subprocess.run([ff, '-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=size=640x360:rate=25', '-f', 'lavfi', '-i', 'sine=frequency=440',
                        '-t', '2', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-c:a', 'aac', '-movflags', '+faststart', str(d / 'good.mp4')], check=True)
        self.big_sha = None
        big_needed = set(self.gates) & {'G03', 'G10'}
        if big_needed:
            subprocess.run([ff, '-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=size=1280x720:rate=30', '-f', 'lavfi', '-i', 'sine=frequency=330',
                            '-t', '20', '-vf', 'noise=alls=40:allf=t', '-c:v', 'libx264', '-preset', 'ultrafast', '-b:v', '40M', '-maxrate', '40M', '-bufsize', '4M',
                            '-pix_fmt', 'yuv420p', '-c:a', 'aac', '-movflags', '+faststart', str(d / 'big.mp4')], check=True)
            if (d / 'big.mp4').stat().st_size > 64 * 1024 * 1024:
                self.big_sha = sha256_file(d / 'big.mp4')
        self.good_sha = sha256_file(d / 'good.mp4')
        write_private(d / 'secrets.json', self.secrets)
        self.fx_port, self.canary_port = free_port(), free_port()
        self.hits_file = str(self.out / 'fixture-hits.jsonl')
        self.fixture = subprocess.Popen([sys.executable, str(HERE / 'fixture_server.py'), '--directory', str(d), '--port', str(self.fx_port),
                                         '--canary-port', str(self.canary_port), '--hits-file', self.hits_file],
                                        stdout=open(self.out / 'fixture.log', 'ab'), stderr=subprocess.STDOUT)
        self.own_pids.add(self.fixture.pid)
        t = time.time()
        while time.time() - t < 10:
            try:
                urllib.request.urlopen(f'http://127.0.0.1:{self.fx_port}/good.mp4', timeout=1).read(10)
                break
            except Exception:
                time.sleep(0.2)
        os.remove(self.hits_file) if os.path.exists(self.hits_file) else None

    def cleanup(self):
        for inst in getattr(self, 'instances', []):
            try:
                inst.stop()
            except Exception:
                pass
        fe = getattr(self, 'frontend', None)
        if fe and fe.poll() is None:
            fe.terminate()
            try:
                fe.wait(10)
            except subprocess.TimeoutExpired:
                fe.kill()
        if self.fixture and self.fixture.poll() is None:
            self.fixture.terminate()
            try:
                self.fixture.wait(5)
            except subprocess.TimeoutExpired:
                self.fixture.kill()
        for inst in getattr(self, 'instances', []):
            if inst.proc and inst.proc.poll() is None:  # only our own live Popen objects
                inst.proc.kill()


def timed(run, gate, fn, *a):
    t = time.time()
    try:
        reason = fn(run, *a)
        res = {'status': 'pass', 'reason': str(reason)}
    except GateSkip as e:
        res = {'status': 'skip', 'reason': str(e)}
    except GateFail as e:
        res = {'status': 'fail', 'reason': str(e)}
    except Exception as e:
        res = {'status': 'fail', 'reason': f'{type(e).__name__}: {e}', 'trace': traceback.format_exc()[-1500:]}
    res['seconds'] = round(time.time() - t, 1)
    with run.lock:
        run.results[gate] = res
    print(f'  {gate} {res["status"]} ({res["seconds"]}s)', file=sys.stderr, flush=True)
    return res


def live(run, A):
    if not run.args.corpus:
        raise GateSkip('--live needs --corpus')
    A.client('A', BASE_SCOPES + ['transfer'], root=True)
    A.client('B', BASE_SCOPES)
    cases = json.loads(pathlib.Path(run.args.corpus).read_text())
    if run.args.live_limit:
        cases = cases[:run.args.live_limit]
    corpus = run.out / 'live-cases.json'
    corpus.write_text(json.dumps(cases, indent=1))
    outdir = run.out / 'live'
    p = subprocess.run([sys.executable, str(HERE / 'desktop_benchmark.py'), '--profile', str(A.profile), '--cases', str(corpus), '--output', str(outdir),
                        '--case-timeout', str(run.args.case_timeout)],
                       capture_output=True, text=True, timeout=3600,
                       env=dict(os.environ, PATH=f'{A.profile / "bin"}:{os.environ.get("PATH", "")}'))
    (run.out / 'live.log').write_text(p.stdout + p.stderr)
    need(p.returncode == 0, f'benchmark exited {p.returncode}: {p.stderr.strip().splitlines()[-1:] if p.stderr else ""}')
    summary = json.loads((outdir / 'summary.json').read_text())
    need(summary['controlledPassed'] == summary['controlledTotal'], f'controlled checks {summary["controlledPassed"]}/{summary["controlledTotal"]}')
    no_conforming = [s for s, v in summary['sites'].items() if not v.get('conforming')]
    failures = sum(v.get('product-failure', 0) for v in summary['sites'].values())
    text = '; '.join(f'{s}: ' + ','.join(f'{k}={n}' for k, n in v.items() if k != 'cases') for s, v in sorted(summary['sites'].items()))
    need(not failures, f'{failures} product failure(s): {text}')
    if no_conforming:
        return f'no conforming download for {no_conforming} (see classification); {text}'
    return text


ONEDIR_EXE = 'yt-dlp_macos'


def ytdlp_onedir(path):
    """Directory of a yt-dlp onedir build (yt-dlp_macos + _internal/), or None for a single file."""
    exe = os.path.realpath(path)
    root = os.path.dirname(exe)
    if os.path.basename(exe) == ONEDIR_EXE and os.path.isdir(os.path.join(root, '_internal')):
        return root
    return None


def warm_ytdlp(path):
    """First run of a yt-dlp build pays the macOS scan (onedir: 20-40 s); keep it out of case timings."""
    t = time.time()
    r = subprocess.run([os.path.realpath(path), '--version'], capture_output=True, text=True, timeout=300)
    need(r.returncode == 0, f'--ytdlp does not run: {r.stderr.strip()[:200]}')
    kind = 'onedir' if ytdlp_onedir(path) else 'single file'
    print(f'yt-dlp {r.stdout.strip()} ({kind}) ready in {time.time() - t:.1f}s', file=sys.stderr, flush=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--app', required=True, help='omniget desktop binary (debug builds need the Vite dev server)')
    ap.add_argument('--worker-dir', help='directory with omniget-worker/omniget-mcp when they are not next to --app')
    ap.add_argument('--gates', default=','.join(ALL_GATES), help='comma list, default all offline gates')
    ap.add_argument('--live', action='store_true', help='also run desktop_benchmark.py against --corpus (network)')
    ap.add_argument('--corpus', help='cases JSON for --live (corpus_from_ytdlp.py format)')
    ap.add_argument('--live-limit', type=int, default=0, help='use only the first N corpus cases')
    ap.add_argument('--case-timeout', type=int, default=300)
    ap.add_argument('--ytdlp', help='yt-dlp binary copied into each profile (needed for yt-dlp sites in --live); '
                    'the yt-dlp_macos of an onedir build links its whole directory as bin/yt-dlp_onedir')
    ap.add_argument('--out', help='output dir (default: new dir under $TMPDIR)')
    ap.add_argument('--keep', action='store_true', help='keep profiles and fixtures after the run')
    ap.add_argument('--frontend', default='auto', help="auto (default), none, or a built frontend dir to serve on :1420")
    args = ap.parse_args()
    run = Run(args)
    run.gates = [g.strip().upper() for g in args.gates.split(',') if g.strip()]
    unknown = set(run.gates) - set(ALL_GATES)
    if unknown:
        raise SystemExit(f'unknown gates: {sorted(unknown)}')
    started = time.time()
    print(f'gates run in {run.out}', file=sys.stderr, flush=True)
    signal.signal(signal.SIGTERM, lambda *_: (run.cleanup(), sys.exit(2)))
    try:
        if args.ytdlp:
            warm_ytdlp(args.ytdlp)
        print('frontend: ' + run.start_frontend(), file=sys.stderr, flush=True)
        run.start_fixture()
        A = Instance(run, 'main')
        B = Instance(run, 'g04') if 'G04' in run.gates else None
        C = Instance(run, 'g05') if 'G05' in run.gates else None
        run.instances = [i for i in (A, B, C) if i]
        with concurrent.futures.ThreadPoolExecutor(3) as ex:
            list(ex.map(lambda i: (i.prepare(), i.start(), i.inv('tool_mcp_set_enabled', {'enabled': True})), run.instances))
        A.eval("""window.__gatesEvents=[]; const I=window.__TAURI_INTERNALS__;
for (const e of ['queue-state-update','queue-item-progress','external-url','media-info-preview','generic-download-progress','generic-download-complete','download-progress','download-complete','mcp-auth-request']) {
 const cb=I.transformCallback((x)=>{window.__gatesEvents.push(JSON.stringify(x.payload).slice(0,20000)); if(window.__gatesEvents.length>5000) window.__gatesEvents.shift();});
 await I.invoke('plugin:event|listen',{event:e,target:{kind:'Any'},handler:cb}); } return true""")
        plan = {'G01': (gate_g01, A), 'G03': (gate_g03, A), 'G04': (gate_g04, B), 'G05': (gate_g05, C),
                'G06': (gate_g06_jobs, A), 'G07': (gate_g07, A), 'G09': (gate_g09, A), 'G10': (gate_g10, A), 'G11': (gate_g11, A)}
        with concurrent.futures.ThreadPoolExecutor(len(plan)) as ex:
            futs = [ex.submit(timed, run, g, plan[g][0], plan[g][1], None) for g in run.gates]
            concurrent.futures.wait(futs)
        if 'G06' in run.gates and run.results['G06']['status'] == 'pass':
            jobs = run.results['G06']
            res = timed(run, 'G06', gate_g06_scan, A, None)
            res['seconds'] = round(res['seconds'] + jobs['seconds'], 1)
        if args.live:
            timed(run, 'LIVE', live, A)
    except SystemExit:
        raise
    except Exception as e:
        run.results['SETUP'] = {'status': 'fail', 'reason': f'{type(e).__name__}: {e}', 'seconds': round(time.time() - started, 1)}
    finally:
        run.cleanup()
    total = round(time.time() - started, 1)
    for r in run.results.values():
        r.pop('trace', None) if r.get('status') == 'pass' else None
    report = {'app': run.app, 'out': str(run.out), 'seconds': total, 'gates': run.results}
    (run.out / 'gates.json').write_text(json.dumps(report, indent=2))
    print(f'{"gate":6} {"result":6} {"secs":>6}  reason')
    for g in [x for x in ['SETUP'] + ALL_GATES + ['LIVE'] if x in run.results]:
        r = run.results[g]
        print(f'{g:6} {r["status"]:6} {r["seconds"]:>6}  {r["reason"][:220]}')
    print(f'total {total}s · {run.out}/gates.json')
    if not args.keep:
        for inst in getattr(run, 'instances', []):
            shutil.rmtree(inst.dir, ignore_errors=True)
        shutil.rmtree(run.out / 'fixtures', ignore_errors=True)
        shutil.rmtree(run.out / 'bin', ignore_errors=True)
        for media in (run.out / 'live').glob('*.media'):
            media.unlink()
    sys.exit(1 if any(r['status'] == 'fail' for r in run.results.values()) else 0)


if __name__ == '__main__':
    main()
