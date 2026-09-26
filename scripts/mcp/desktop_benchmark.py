#!/usr/bin/env python3
"""Real desktop MCP benchmark using an explicitly isolated profile.
Run only against a profile created for this benchmark, never a user's tokens.

  python3 scripts/mcp/desktop_benchmark.py --profile <isolated profile> \
      --cases cases.json --output <dir> [--run-id ID] [--case-timeout 300]

The profile needs settings.json (bridge port) and a private tokens.json with
connections A (transfer + a granted output folder) and B (any scopes).
Cases: [{caseId, platform, url, mode?|maxHeight?, expected?: {ext, duration}}]
(scripts/mcp/corpus_from_ytdlp.py writes this format).

Every case is classified:
  conforming           valid media (digest, ffprobe, decode), policy met, duration
                       within +-5% (+1 s, upstream durations are rounded) of
                       expected.duration when known; the extension is only reported
  valid-nonconforming  valid media that misses the policy or the expected duration
  correct-refusal      FORMAT_UNAVAILABLE: nothing within the requested options, or
                       LIMIT_BYTES/LIMIT_TIME/LIMIT_CONNECTIONS: OmniGet's own
                       per-download egress budget (the media is too big/long)
  external             content gone, login/age/geo, IP block, rate limit, bot check
  product-failure      anything else (engine errors, invalid artifacts, timeouts)
Outputs: results.jsonl, summary.csv, summary.json (per site), controlled-http.json.
A correct refusal is never counted as a download.
"""
import argparse
import csv
import datetime
import hashlib
import json
import pathlib
import re
import subprocess
import time
import urllib.error
import urllib.request
import uuid

EXTERNAL_CODES = {'NOT_FOUND', 'AUTH_REQUIRED', 'AUTH_EXPIRED', 'ACCESS_DENIED', 'GEO_RESTRICTED',
                  'RATE_LIMITED', 'BOT_CHALLENGE', 'BLOCKED_BY_PLATFORM', 'SOURCE_RESTRICTED', 'DRM_UNSUPPORTED',
                  'CONTENT_REMOVED'}
EXTERNAL_PREFIXES = ('HTTP 404', 'HTTP 401', 'HTTP 403', 'HTTP 429', 'BLOCKED_BY_PLATFORM', 'BOT_CHALLENGE')
# FORMAT_UNAVAILABLE: nothing within the requested options. LIMIT_*: OmniGet's
# per-download egress budget (src-tauri/omniget-core/src/core/egress.rs).
REFUSAL_CODES = {'FORMAT_UNAVAILABLE', 'LIMIT_BYTES', 'LIMIT_TIME', 'LIMIT_CONNECTIONS'}


def leading_code(message):
    """The bare machine code a message starts with ('AUTH_REQUIRED' or
    'CONTENT_REMOVED: ...'), or None. Only the leading token counts: our own
    codes (ENGINE_FAILED, EGRESS_FAILED...) wrapping a platform word stay ours."""
    m = re.match(r'([A-Z][A-Z0-9_]+)(?::|\s|$)', message or '')
    return m.group(1) if m else None


def classify(status, diagnosis_code, artifacts, case):
    if any(a['valid'] and a.get('durationOk', True) for a in artifacts):
        return 'conforming'
    if any(a['mediaValid'] for a in artifacts):
        return 'valid-nonconforming'
    message = ((status or {}).get('data') or {}).get('message', '') if isinstance(status, dict) else ''
    code = leading_code(message)
    if code in REFUSAL_CODES or diagnosis_code in REFUSAL_CODES:
        return 'correct-refusal'
    if diagnosis_code in EXTERNAL_CODES or code in EXTERNAL_CODES or message.startswith(EXTERNAL_PREFIXES):
        return 'external'
    return 'product-failure'


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--profile', required=True)
    p.add_argument('--cases', required=True)
    p.add_argument('--output', required=True)
    p.add_argument('--run-id', default=uuid.uuid4().hex[:12])
    p.add_argument('--case-timeout', type=int, default=300)
    a = p.parse_args()
    root = pathlib.Path(a.profile)
    out = pathlib.Path(a.output)
    out.mkdir(parents=True, exist_ok=True)
    tokens = json.loads((root / 'tokens.json').read_text())
    settings = json.loads((root / 'settings.json').read_text())['app_settings']
    url = f"http://127.0.0.1:{settings['bridge']['port']}/mcp"
    seq = calls = response_bytes = 0

    def rpc(method, params=None, client='A', headers=None):
        nonlocal seq, calls, response_bytes
        seq += 1
        calls += 1
        req = urllib.request.Request(url, data=json.dumps({'jsonrpc': '2.0', 'id': seq, 'method': method, 'params': params or {}}).encode(),
                                     headers={'Authorization': 'Bearer ' + tokens[client], 'Content-Type': 'application/json',
                                              'Accept': 'application/json, text/event-stream', 'MCP-Protocol-Version': '2025-06-18', **(headers or {})})
        with urllib.request.urlopen(req, timeout=35) as r:
            body = r.read(1048576)
            response_bytes += len(body)
            return json.loads(body)

    def tool(name, args=None, client='A'):
        r = rpc('tools/call', {'name': name, 'arguments': args or {}}, client)
        return r.get('result', r)

    def sc(result):
        return result.get('structuredContent') or {}

    init = rpc('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'omniget-independent-python-harness', 'version': '1'}})
    inventory = rpc('tools/list')
    (out / 'initialize.json').write_text(json.dumps(init, indent=2))
    (out / 'tools.json').write_text(json.dumps(inventory, indent=2))
    controlled = []

    def check(name, ok):
        controlled.append({'case': name, 'passed': bool(ok)})

    # An unknown tool is a protocol error (-32602), or an isError result on older builds.
    denied = tool('agent_delegate', {'task': 'test'})
    check('unknown-tool-denied', denied.get('isError') or (denied.get('error') or {}).get('code') == -32602)
    for name, headers, expected in [('extension-token-denied', {'Authorization': 'Bearer isolated-extension-token'}, 401),
                                    ('origin-denied', {'Origin': 'https://attacker.invalid'}, 403),
                                    ('host-denied', {'Host': 'attacker.invalid'}, 403),
                                    ('unsupported-revision', {'MCP-Protocol-Version': '2099-01-01'}, 400)]:
        try:
            rpc('ping', headers=headers)
            check(name, False)
        except urllib.error.HTTPError as e:
            check(name, e.code == expected)
    def order(c):
        tail = c['caseId'].rsplit('-', 1)[-1]
        return (int(tail) if tail.isdigit() else 0, c['platform'])
    cases = sorted(json.loads(pathlib.Path(a.cases).read_text()), key=order)
    results = []
    round_started = time.monotonic()
    round_bytes = 0
    for c in cases:
        if time.monotonic() - round_started >= 2700 or round_bytes >= 2 * 1024 ** 3:
            (out / 'budget-stop.json').write_text(json.dumps({'reason': 'round_time_or_bytes_limit', 'remainingCase': c['caseId']}))
            break
        started = time.monotonic()
        call_start = calls
        bytes_start = response_bytes
        args = {'url': c['url'], 'idempotencyKey': 'bench-' + a.run_id + '-' + c['caseId']}
        if c.get('mode') == 'audio':
            args['mode'] = 'audio'
        else:
            args['maxHeight'] = c.get('maxHeight', (c.get('policy') or {}).get('maxHeight', 720))
        enqueue = tool('download_enqueue', args)
        value = sc(enqueue)
        id = (value.get('item') or {}).get('id') or value.get('download_id')
        state = None
        diagnosis = None
        artifacts = []
        if id:
            replay = tool('download_enqueue', args)
            check(c['caseId'] + '-idempotency', ((sc(replay).get('item') or {}).get('id') or sc(replay).get('download_id')) == id)
            check(c['caseId'] + '-owner-isolation', tool('download_status', {'download_id': id}, 'B').get('isError'))
            conflict = tool('download_enqueue', {**args, 'mode': 'video' if args.get('mode') == 'audio' else 'audio'})
            check(c['caseId'] + '-conflict', conflict.get('isError'))
            deadline = time.monotonic() + a.case_timeout
            while time.monotonic() < deadline:
                state = sc(tool('download_status', {'download_id': id})).get('item', {})
                status = state.get('status')
                tag = status if isinstance(status, str) else (status or {}).get('type')
                if state.get('downloaded_bytes', 0) > 150 * 1024 ** 2:
                    tool('download_cancel', {'download_id': id, 'idempotencyKey': 'byte-limit-' + a.run_id + '-' + c['caseId']})
                    break
                if str(tag).lower() in ('complete', 'error', 'unknown'):
                    break
                time.sleep(1)
            else:
                tool('download_cancel', {'download_id': id, 'idempotencyKey': 'timeout-' + a.run_id + '-' + c['caseId']})
            diagnosis = tool('download_diagnose', {'download_id': id})
            catalog = sc(tool('download_artifacts', {'download_id': id})).get('artifacts', [])
            for index, artifact in enumerate(catalog):
                access = artifact.get('access', {})
                if not artifact.get('transferAllowed') or not access.get('downloadPath'):
                    continue
                expected = access['digest']
                route = access['downloadPath']
                if not route.startswith('/mcp/artifacts/') or '?' in route or '..' in route:
                    raise ValueError('Unexpected artifact route')
                transfer = urllib.request.Request(url[:-len('/mcp')] + route, headers={'Authorization': 'Bearer ' + tokens['A'], 'If-Match': '"' + expected + '"'})
                # One file per artifact: a carousel produces several for one job.
                target = out / (c['caseId'] + '-' + str(id) + '-' + str(index) + '.media')
                digest = hashlib.sha256()
                size = 0
                with urllib.request.urlopen(transfer, timeout=35) as r, target.open('wb') as f:
                    while True:
                        chunk = r.read(65536)
                        if not chunk:
                            break
                        size += len(chunk)
                        if size > 150 * 1024 ** 2:
                            raise ValueError('Artifact byte limit')
                        digest.update(chunk)
                        f.write(chunk)
                digest_valid = digest.hexdigest() == expected and size == access['bytes']
                # Pipe-only input prevents playlist/protocol references from fetching
                # other local files or network destinations during independent decode.
                with target.open('rb') as f:
                    probe = subprocess.run(['ffprobe', '-v', 'error', '-protocol_whitelist', 'pipe', '-show_streams', '-show_format', '-of', 'json', 'pipe:0'],
                                           stdin=f, capture_output=True, text=True, timeout=20)
                try:
                    data = json.loads(probe.stdout)
                except ValueError:
                    data = {}
                with target.open('rb') as f:
                    decode = subprocess.run(['ffmpeg', '-v', 'error', '-protocol_whitelist', 'pipe', '-i', 'pipe:0', '-t', '2', '-f', 'null', '-'],
                                            stdin=f, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=20)
                streams = data.get('streams', [])
                media_valid = digest_valid and probe.returncode == 0 and decode.returncode == 0 and bool(streams)
                heights = [s.get('height', 0) for s in streams if s.get('codec_type') == 'video' and not s.get('disposition', {}).get('attached_pic')]
                if c.get('mode') == 'audio':
                    policy_valid = any(s.get('codec_type') == 'audio' for s in streams) and not heights
                else:
                    # Same rule as the app's check_height: video within the
                    # ceiling, or audio alone (an audio-only source has no video
                    # to exceed it).
                    has_audio = any(s.get('codec_type') == 'audio' for s in streams)
                    policy_valid = (bool(heights) and all(0 < h <= args['maxHeight'] for h in heights)) or (not heights and has_audio)
                exp = c.get('expected') or {}
                try:
                    duration = float((data.get('format') or {}).get('duration') or 0)
                except ValueError:
                    duration = 0.0
                if duration <= 0 and media_valid:
                    # Pipe input cannot seek: a header-less MP3 reports no
                    # duration. Decode it fully and take the last timestamp.
                    with target.open('rb') as f:
                        full = subprocess.run(['ffmpeg', '-v', 'info', '-nostats', '-protocol_whitelist', 'pipe', '-i', 'pipe:0', '-f', 'null', '-'],
                                              stdin=f, capture_output=True, text=True, timeout=60)
                    stamps = re.findall(r'time=(\d+):(\d+):(\d+(?:\.\d+)?)', full.stderr)
                    if stamps:
                        hh, mm, ss = stamps[-1]
                        duration = int(hh) * 3600 + int(mm) * 60 + float(ss)
                duration_ok = True
                if exp.get('duration') and index == 0:
                    duration_ok = duration > 0 and abs(duration - float(exp['duration'])) <= 0.05 * float(exp['duration']) + 1.0
                name = artifact.get('name') or ''
                ext = name.rsplit('.', 1)[-1].lower() if '.' in name else ''
                artifacts.append({'artifactId': access['artifactId'], 'name': name, 'digest': expected, 'digestValid': digest_valid, 'bytes': size,
                                  'probe': data, 'mediaValid': media_valid, 'requestedPolicySatisfied': policy_valid,
                                  'durationSeconds': duration, 'durationOk': duration_ok,
                                  'extension': ext, 'extensionMatchesExpected': (ext == exp.get('ext')) if exp.get('ext') else None,
                                  'valid': media_valid and policy_valid})
        round_bytes += max((state or {}).get('downloaded_bytes', 0), sum(x['bytes'] for x in artifacts))
        diag_code = ((sc(diagnosis or {}).get('diagnosis') or {}).get('code')) if diagnosis else None
        classification = classify((state or {}).get('status'), diag_code, artifacts, c) if id else (
            'correct-refusal' if 'FORMAT_UNAVAILABLE' in json.dumps(enqueue) else 'product-failure')
        row = {**c, 'benchmarkRunId': a.run_id, 'path': 'mcp_http_real_desktop', 'enqueue': enqueue, 'downloadId': id, 'lastStatus': state,
               'diagnosis': diagnosis, 'diagnosisCode': diag_code, 'artifactValidation': artifacts,
               'outcome': 'success' if classification == 'conforming' else 'failed', 'classification': classification,
               'totalSeconds': time.monotonic() - started, 'mcpCalls': calls - call_start, 'responseBytes': response_bytes - bytes_start,
               'timestamp': datetime.datetime.now(datetime.timezone.utc).isoformat()}
        results.append(row)
        with (out / 'results.jsonl').open('a') as f:
            f.write(json.dumps(row) + '\n')
        print(c['caseId'], classification, round(row['totalSeconds'], 2), flush=True)
    (out / 'controlled-http.json').write_text(json.dumps(controlled, indent=2))
    with (out / 'summary.csv').open('w') as f:
        fields = ['caseId', 'platform', 'path', 'classification', 'outcome', 'totalSeconds', 'mcpCalls', 'responseBytes']
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        w.writerows({k: r[k] for k in fields} for r in results)
    sites = {}
    for r in results:
        s = sites.setdefault(r['platform'], {'cases': 0})
        s['cases'] += 1
        s[r['classification']] = s.get(r['classification'], 0) + 1
    summary = {'runId': a.run_id, 'cases': len(results), 'sites': sites,
               'controlledPassed': sum(x['passed'] for x in controlled), 'controlledTotal': len(controlled)}
    (out / 'summary.json').write_text(json.dumps(summary, indent=2))
    for site, s in sorted(sites.items()):
        print(f"{site:12} " + ' '.join(f'{k}={v}' for k, v in s.items()), flush=True)


if __name__ == '__main__':
    main()
