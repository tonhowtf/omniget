#!/usr/bin/env python3
"""FAKE `codex app-server` for tests and BN10 --fake (item 3.6). Not a model.

Speaks the JSON-RPC-over-stdio subset OmniGet uses (codex 0.158 schema,
`estudos/91-onda3/codex-app-server-schema/`): `initialize`, `initialized`,
`thread/start`, `thread/resume`, `turn/start`, `turn/interrupt`, and sends
`item/commandExecution/requestApproval` back when asked to. Stdlib only.

Invoked as `<this> app-server` (the argv OmniGet builds). State lives in
$FAKE_CODEX_DIR:
  count                  calls so far
  call-N.args|.env|.pwd  argv, CODEX_HOME, cwd of call N
  call-N.in.jsonl        every line OmniGet wrote on stdin
  threads/<id>.json      the history of a thread (what `thread/resume` finds)
  mode                   chat (default) | approve | hang | deaf
  decision-N             the approval answer of call N (mode approve)
  interrupt-N            written when `turn/interrupt` arrives (mode hang)

Usage model (the fake's, documented, not Codex's): a turn's input is a fixed
1000-token prelude (Codex's own instructions) plus the thread history plus
the new text, at 4 chars per token. A resumed thread's history is a cached
prefix; a fresh thread caches only the prelude.
"""
import json
import os
import pathlib
import sys
import threading
import time

PRELUDE = 1000
DIR = pathlib.Path(os.environ.get('FAKE_CODEX_DIR', '.'))
LOCK = threading.Lock()

def tokens(text):
    return (len(text) + 3) // 4

def main():
    DIR.mkdir(parents=True, exist_ok=True)
    (DIR / 'threads').mkdir(exist_ok=True)
    n = int((DIR / 'count').read_text()) + 1 if (DIR / 'count').exists() else 1
    (DIR / 'count').write_text(str(n))
    (DIR / f'call-{n}.args').write_text('\n'.join(sys.argv[1:]))
    (DIR / f'call-{n}.env').write_text(os.environ.get('CODEX_HOME', ''))
    (DIR / f'call-{n}.pwd').write_text(os.getcwd())
    mode = (DIR / 'mode').read_text().strip() if (DIR / 'mode').exists() else 'chat'
    log = open(DIR / f'call-{n}.in.jsonl', 'a')

    def send(obj):
        with LOCK:
            sys.stdout.write(json.dumps(obj) + '\n')
            sys.stdout.flush()

    state = {'thread': None, 'history': [], 'resumed': False, 'turn': None, 'waiting': None,
             'interrupted': threading.Event(), 'answer': threading.Event(), 'decision': None}

    def save():
        (DIR / 'threads' / f"{state['thread']}.json").write_text(json.dumps(state['history']))

    def run_turn(text, turn_id):
        send({'method': 'turn/started', 'params': {'threadId': state['thread'], 'turn': {'id': turn_id, 'status': 'inProgress', 'items': []}}})
        history_chars = sum(len(h) for h in state['history'])
        if mode in ('hang', 'deaf'):
            send({'method': 'item/agentMessage/delta', 'params': {'itemId': 'm1', 'delta': 'working'}})
            if mode == 'hang':
                state['interrupted'].wait(300)
                (DIR / f'interrupt-{n}').write_text(str(time.time()))
                send({'method': 'turn/completed', 'params': {'threadId': state['thread'], 'turn': {'id': turn_id, 'status': 'interrupted', 'items': []}}})
            else:
                time.sleep(300)
            return
        if mode == 'approve':
            send({'method': 'item/started', 'params': {'item': {'type': 'commandExecution', 'id': 'c1', 'command': 'touch made.txt', 'cwd': os.getcwd(), 'status': 'inProgress'}}})
            send({'id': 100, 'method': 'item/commandExecution/requestApproval',
                  'params': {'threadId': state['thread'], 'turnId': turn_id, 'itemId': 'c1', 'command': 'touch made.txt', 'reason': 'create a file'}})
            state['answer'].wait(300)
            decision = state['decision']
            (DIR / f'decision-{n}').write_text(str(decision))
            ok = decision in ('accept', 'acceptForSession')
            if ok:
                pathlib.Path('made.txt').write_text('made\n')
            send({'method': 'item/completed', 'params': {'item': {'type': 'commandExecution', 'id': 'c1', 'command': 'touch made.txt',
                                                                  'status': 'completed' if ok else 'declined', 'aggregatedOutput': '' if ok else None}}})
        reply = f"ok {len(state['history']) // 2 + 1}"
        for part in (reply[:2], reply[2:]):
            send({'method': 'item/agentMessage/delta', 'params': {'itemId': 'm1', 'delta': part}})
        send({'method': 'item/completed', 'params': {'item': {'type': 'agentMessage', 'id': 'm1', 'text': reply}}})
        cached = PRELUDE + (tokens('x' * history_chars) if state['resumed'] else 0)
        usage = {'inputTokens': PRELUDE + tokens('x' * history_chars) + tokens(text), 'cachedInputTokens': cached,
                 'outputTokens': tokens(reply), 'reasoningOutputTokens': 0, 'totalTokens': 0}
        send({'method': 'thread/tokenUsage/updated', 'params': {'threadId': state['thread'], 'turnId': turn_id,
                                                                'tokenUsage': {'last': usage, 'total': usage}}})
        state['history'] += [text, reply]
        save()
        send({'method': 'turn/completed', 'params': {'threadId': state['thread'], 'turn': {'id': turn_id, 'status': 'completed', 'items': []}}})

    for line in sys.stdin:
        log.write(line)
        log.flush()
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        method, mid, params = msg.get('method'), msg.get('id'), msg.get('params') or {}
        if method is None and mid is not None:  # an answer to our request
            state['decision'] = (msg.get('result') or {}).get('decision')
            state['answer'].set()
            continue
        if method == 'initialize':
            send({'id': mid, 'result': {'userAgent': 'fake-codex/0.158.0', 'codexHome': os.environ.get('CODEX_HOME', '')}})
        elif method == 'thread/start':
            state['thread'] = f'th-{n}'
            state['history'] = []
            state['resumed'] = False
            save()
            send({'id': mid, 'result': {'thread': {'id': state['thread']}, 'model': params.get('model') or 'fake'}})
        elif method == 'thread/resume':
            tid = params.get('threadId')
            path = DIR / 'threads' / f'{tid}.json'
            if not path.exists():
                send({'id': mid, 'error': {'code': -32600, 'message': f'no rollout found for thread id {tid}'}})
                continue
            state['thread'] = tid
            state['history'] = json.loads(path.read_text())
            state['resumed'] = True
            send({'id': mid, 'result': {'thread': {'id': tid}, 'model': params.get('model') or 'fake'}})
        elif method == 'turn/start':
            text = ''.join(i.get('text', '') for i in params.get('input', []))
            turn_id = f'tu-{n}'
            state['turn'] = turn_id
            send({'id': mid, 'result': {'turn': {'id': turn_id, 'status': 'inProgress', 'items': []}}})
            threading.Thread(target=run_turn, args=(text, turn_id), daemon=True).start()
        elif method == 'turn/interrupt':
            send({'id': mid, 'result': {}})
            state['interrupted'].set()
        elif mid is not None:
            send({'id': mid, 'error': {'code': -32601, 'message': f'unknown method {method}'}})
    return 0

if __name__ == '__main__':
    sys.exit(main())
