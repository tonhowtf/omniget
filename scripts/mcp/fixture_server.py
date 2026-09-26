#!/usr/bin/env python3
"""Loopback-only fixtures for the real desktop queue. No accounts or Internet.

Create good.mp4 with a two-second synthetic ffmpeg test pattern, then pass its
directory. Grant local_network only to a disposable isolated-profile client.

Optional files in --directory: vertical.mp4, big.mp4 (served for the >64 MiB
transfer check) and secrets.json (synthetic secrets echoed by the secret*
routes, for leak scans). --canary-port opens a second listener that must never
be reached through the worker's egress (redirect/manifest targets point at it);
every request to either port is appended to --hits-file as one JSON line.

Routes (file name only; the query string is free for uniqueness):
  good.mp4 vertical.mp4 big.mp4 secret-good.mp4 *escape*.mp4   200 media (Range)
  redirect.mp4 redirect-ok.mp4       302 -> /good.mp4
  auth.mp4 forbidden.mp4 missing.mp4 401 / 403 / 404
  limited.mp4?ra=N                   429 with Retry-After N (default 60)
  html.mp4                           200 HTML sent as video/mp4
  slow.mp4?bps=N                     big.mp4 (or good.mp4) throttled to N bytes/s
  hang.mp4                           headers, then no body (in-flight job)
  flaky.mp4?fail=N                   503 for the first N requests of this exact path+query
  secret401.mp4 secret403.mp4        errors whose body/headers carry the synthetic secrets
  redirect-canary.mp4 redirect-meta.mp4 hls.m3u8   targets outside the grant
  cd-traversal.mp4 cd-abs.mp4        Content-Disposition with ../ and absolute names
"""
import argparse
import http.server
import json
import pathlib
import threading
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--directory', required=True)
    parser.add_argument('--port', type=int, default=47831)
    parser.add_argument('--canary-port', type=int, default=0)
    parser.add_argument('--hits-file', default='')
    args = parser.parse_args()
    root = pathlib.Path(args.directory)
    media = (root / 'good.mp4').read_bytes()
    vertical_path = root / 'vertical.mp4'
    vertical = vertical_path.read_bytes() if vertical_path.exists() else media
    big_path = root / 'big.mp4'
    big = big_path.read_bytes() if big_path.exists() else None
    secrets_path = root / 'secrets.json'
    secrets = json.loads(secrets_path.read_text()) if secrets_path.exists() else {}
    counts = {}
    lock = threading.Lock()

    def record(tag, handler):
        if not args.hits_file:
            return
        line = json.dumps({'t': time.time(), 'port': tag, 'method': handler.command,
                           'path': handler.path, 'range': handler.headers.get('Range')})
        with lock, open(args.hits_file, 'a') as f:
            f.write(line + '\n')

    def make(tag):
        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *_):
                pass

            def do_HEAD(self):
                self.serve(False)

            def do_GET(self):
                self.serve(True)

            def query(self):
                q = {}
                if '?' in self.path:
                    for part in self.path.split('?', 1)[1].split('&'):
                        k, _, v = part.partition('=')
                        q[k] = v
                return q

            def send(self, status, data=b'', ctype='video/mp4', extra=None, body=True,
                     throttle=None, hang=False):
                start, end = 0, len(data) - 1
                if status == 200 and data and self.headers.get('Range', '').startswith('bytes='):
                    span = self.headers['Range'][6:].split('-', 1)
                    start = int(span[0] or 0)
                    end = min(int(span[1]) if span[1] else end, end)
                    status = 206
                self.send_response(status)
                self.send_header('Content-Type', ctype)
                self.send_header('Accept-Ranges', 'bytes')
                for k, v in (extra or {}).items():
                    self.send_header(k, v)
                self.send_header('Content-Length', str(max(0, end - start + 1)))
                if status == 206:
                    self.send_header('Content-Range', f'bytes {start}-{end}/{len(data)}')
                self.end_headers()
                if hang:
                    time.sleep(3600)
                    return
                if not body:
                    return
                try:
                    chunk = data[start:end + 1]
                    if throttle:
                        for i in range(0, len(chunk), 65536):
                            self.wfile.write(chunk[i:i + 65536])
                            self.wfile.flush()
                            time.sleep(65536 / throttle)
                    else:
                        self.wfile.write(chunk)
                except (BrokenPipeError, ConnectionResetError):
                    pass

            def serve(self, body):
                record(tag, self)
                if tag == 'canary':
                    return self.send(200, media, body=body)
                name = self.path.split('?', 1)[0].split('/')[-1]
                q = self.query()
                with lock:
                    counts[self.path] = counts.get(self.path, 0) + 1
                    seen = counts[self.path]
                canary = f'http://127.0.0.1:{args.canary_port}'
                s = lambda k: secrets.get(k, 'SYNTHETIC')
                if name in ('good.mp4', 'secret-good.mp4') or 'escape' in name:
                    return self.send(200, media, body=body)
                if name == 'vertical.mp4':
                    return self.send(200, vertical, body=body)
                if name == 'big.mp4':
                    return self.send(200, big, body=body) if big else self.send(404, b'no big.mp4', 'text/plain', body=body)
                if name == 'slow.mp4':
                    return self.send(200, big or media, body=body, throttle=int(q.get('bps') or 1_000_000))
                if name == 'hang.mp4':
                    return self.send(200, big or media, body=body, hang=body)
                if name in ('redirect.mp4', 'redirect-ok.mp4'):
                    return self.send(302, extra={'Location': '/good.mp4'}, body=body)
                if name == 'redirect-canary.mp4':
                    return self.send(302, extra={'Location': canary + '/good.mp4'}, body=body)
                if name == 'redirect-meta.mp4':
                    return self.send(302, extra={'Location': 'http://169.254.169.254/latest/meta-data/'}, body=body)
                if name == 'hls.m3u8':
                    m3u8 = f'#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\n{canary}/seg.ts\n#EXT-X-ENDLIST\n'
                    return self.send(200, m3u8.encode(), 'application/vnd.apple.mpegurl', body=body)
                if name == 'cd-traversal.mp4':
                    return self.send(200, media, extra={'Content-Disposition': 'attachment; filename="../../../../gv-escape-cd.mp4"'}, body=body)
                if name == 'cd-abs.mp4':
                    return self.send(200, media, extra={'Content-Disposition': 'attachment; filename="/tmp/gv-abs-escape.mp4"'}, body=body)
                if name == 'secret401.mp4':
                    page = f'<html>denied Authorization: Bearer {s("synthbody")} cookie={s("synthcookie")}</html>'
                    return self.send(401, page.encode(), 'text/html', {'Set-Cookie': f'sessionid={s("synthcookie")}; Path=/',
                                                                       'WWW-Authenticate': f'Bearer realm="{s("synthhdr")}"'}, body=body)
                if name == 'secret403.mp4':
                    return self.send(403, f'forbidden token={s("synthbody")}'.encode(), 'text/plain', {'X-Debug-Token': s('synthhdr')}, body=body)
                if name == 'flaky.mp4':
                    if seen <= int(q.get('fail') or 1):
                        return self.send(503, b'try later', 'text/plain', body=body)
                    return self.send(200, media, body=body)
                status = {'auth.mp4': 401, 'forbidden.mp4': 403, 'missing.mp4': 404, 'limited.mp4': 429}.get(name)
                if status == 429:
                    return self.send(429, b'slow down', 'text/plain', {'Retry-After': q.get('ra') or '60'}, body=body)
                if status:
                    return self.send(status, b'synthetic error', 'text/plain', body=body)
                if name == 'html.mp4':
                    return self.send(200, b'<html>synthetic error, not media</html>', 'video/mp4', body=body)
                return self.send(404, b'unknown fixture', 'text/plain', body=body)
        return Handler

    servers = [('fixture', args.port)] + ([('canary', args.canary_port)] if args.canary_port else [])
    for tag, port in servers:
        server = http.server.ThreadingHTTPServer(('127.0.0.1', port), make(tag))
        server.daemon_threads = True
        threading.Thread(target=server.serve_forever, daemon=True).start()
    print(f'fixtures listening on 127.0.0.1:{args.port}' + (f' (canary {args.canary_port})' if args.canary_port else ''), flush=True)
    while True:
        time.sleep(3600)


if __name__ == '__main__':
    main()
