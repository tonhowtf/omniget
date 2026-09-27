#!/usr/bin/env python3
"""Fake `claude -p --output-format stream-json` for the watchdog, approval,
parallel-jobs and job-diff benchmarks (BN1, BN3, BN2, BN4). No network, no
account, deterministic.

It accepts (and ignores) the real argv of a Claude turn, reads the prompt
from stdin like the real CLI, and writes stream-json lines shaped like the
fixtures in `src-tauri/omniget-core/tests/cli_fixtures/claude-2.1.276-*.jsonl`,
so `ClaudeParser` reads it exactly as it reads Claude.

The behaviour comes from the environment, because the argv belongs to the
turn and must stay the real one:

  FAKE_CLAUDE_MODE     ok | stall-after-init | silent-start | tool-long |
                       ask-pending | grandchild | grandchild-setsid | write |
                       sleep | apply
                       (default: ok)
  FAKE_CLAUDE_HOLD_S   seconds of silence in the modes that go quiet
                       (default 3600: "forever" for a benchmark)
  FAKE_CLAUDE_PIDFILE  where `grandchild`/`grandchild-setsid` write the pid of
                       their child
  FAKE_CLAUDE_TAG      `write`: this job's name (its line in the ledger)
  FAKE_CLAUDE_LEDGER   `write`: file every job appends its line to
  FAKE_CLAUDE_FILE     `write`: file only this job writes
  FAKE_CLAUDE_TRACE    `write`/`sleep`: file that gets one JSON line
                       {tag, start, end, cwd} per run (wall-clock seconds)
  FAKE_CLAUDE_CHANGES  `apply`: JSON file {"write": {path: text},
                       "write_b64": {path: base64 bytes}, "delete": [path]},
                       paths relative to the working folder

Modes:
  ok                 init, a text reply, result; exit 0.
  stall-after-init   init, then silence for HOLD (a hung turn).
  silent-start       nothing at all for HOLD (the prompt never took).
  tool-long          init, a Bash tool_use, silence for HOLD (the tool runs),
                     then the tool_result, a reply and the result.
  ask-pending        like tool-long: the real CLI is silent while its
                     permission prompt tool waits for a person; the pause
                     itself comes from OmniGet's broker, not from here.
  grandchild         init, starts `sleep` as a child (same process group, like
                     a git or an MCP server Claude would start), writes its pid,
                     then silence for HOLD.
  grandchild-setsid  like grandchild, but the child calls setsid() first (a
                     new session and group, like an MCP server, `nohup` or a
                     daemonizing tool): `killpg` of the CLI's group misses it.
  write              init, reads LEDGER, waits HOLD, writes it back with its own
                     line appended (read-modify-write, like an agent editing a
                     file: two of these at once lose a line), writes FILE,
                     then a reply and the result.
  sleep              init, HOLD seconds of work (silent), a reply, the result:
                     a read-only job of known length.
  apply              init, a Bash tool_use, applies FAKE_CLAUDE_CHANGES to the
                     working folder byte for byte (a known edit, as an agent's
                     shell would make it: nothing goes through OmniGet's
                     tools), the tool_result, a reply and the result.

`--help` prints a short help (the capability probe never hangs on it).
"""
import base64
import json
import os
import subprocess
import sys
import time
import uuid

SESSION = "5e0f8a3c-1d2b-4a55-9c77-0b1f2e3d4c5a"


def emit(obj):
    obj.setdefault("session_id", SESSION)
    obj.setdefault("uuid", str(uuid.uuid4()))
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def init():
    emit({"type": "system", "subtype": "init", "cwd": os.getcwd(),
          "claude_code_version": "2.1.276-fake", "apiKeySource": "none",
          "model": "claude-sonnet-4-5", "permissionMode": "default",
          "tools": ["Bash", "Read", "Edit"], "mcp_servers": []})


def text_reply(text):
    emit({"type": "stream_event", "event": {"type": "message_start", "message": {
        "id": "msg_fake", "type": "message", "role": "assistant", "content": [],
        "model": "claude-sonnet-4-5", "usage": {"input_tokens": 4, "output_tokens": 1}}},
        "parent_tool_use_id": None})
    emit({"type": "stream_event", "event": {"type": "content_block_start", "index": 0,
                                            "content_block": {"type": "text", "text": ""}},
          "parent_tool_use_id": None})
    emit({"type": "stream_event", "event": {"type": "content_block_delta", "index": 0,
                                            "delta": {"type": "text_delta", "text": text}},
          "parent_tool_use_id": None})
    emit({"type": "stream_event", "event": {"type": "content_block_stop", "index": 0},
          "parent_tool_use_id": None})
    emit({"type": "assistant", "message": {
        "id": "msg_fake", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": text}], "stop_reason": "end_turn",
        "usage": {"input_tokens": 4, "output_tokens": 2, "cache_creation_input_tokens": 0,
                  "cache_read_input_tokens": 12040}}, "parent_tool_use_id": None})


def tool_use(tool_id, command):
    # With `--include-partial-messages` (what OmniGet passes) the tool call
    # streams first, block by block, as in claude-2.1.276-tool-use.jsonl; the
    # whole `assistant` message follows.
    emit({"type": "stream_event", "event": {"type": "message_start", "message": {
        "id": "msg_tool", "type": "message", "role": "assistant", "content": [],
        "model": "claude-sonnet-4-5", "usage": {"input_tokens": 4, "output_tokens": 1}}},
        "parent_tool_use_id": None})
    emit({"type": "stream_event", "event": {"type": "content_block_start", "index": 0,
                                            "content_block": {"type": "tool_use", "id": tool_id,
                                                              "name": "Bash", "input": {}}},
          "parent_tool_use_id": None})
    emit({"type": "stream_event", "event": {"type": "content_block_delta", "index": 0,
                                            "delta": {"type": "input_json_delta",
                                                      "partial_json": json.dumps({"command": command})}},
          "parent_tool_use_id": None})
    emit({"type": "stream_event", "event": {"type": "content_block_stop", "index": 0},
          "parent_tool_use_id": None})
    emit({"type": "assistant", "message": {
        "id": "msg_tool", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
        "content": [{"type": "tool_use", "id": tool_id, "name": "Bash",
                     "input": {"command": command, "description": "fake"}}],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 4, "output_tokens": 9}}, "parent_tool_use_id": None})


def tool_result(tool_id, content):
    emit({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": tool_id, "content": content}]},
        "parent_tool_use_id": None})


def result(text):
    emit({"type": "result", "subtype": "success", "duration_ms": 10, "duration_api_ms": 5,
          "is_error": False, "num_turns": 1, "result": text, "stop_reason": "end_turn",
          "total_cost_usd": 0.0,
          "usage": {"input_tokens": 4, "output_tokens": 2, "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 12040}})


def main():
    if "--help" in sys.argv[1:] or "-h" in sys.argv[1:]:
        print("Usage: claude [options] (fake for OmniGet benchmarks)\n  -p, --print\n  --output-format <format>")
        return 0
    if "--version" in sys.argv[1:]:
        print("2.1.276-fake (Claude Code)")
        return 0
    mode = os.environ.get("FAKE_CLAUDE_MODE", "ok")
    hold = float(os.environ.get("FAKE_CLAUDE_HOLD_S", "3600"))
    # The prompt arrives on stdin and stdin is closed after it, like a turn.
    try:
        sys.stdin.read()
    except Exception:  # noqa: BLE001 - a closed or missing stdin is fine
        pass

    if mode == "silent-start":
        time.sleep(hold)
        return 0
    init()
    if mode == "ok":
        text_reply("ok")
        result("ok")
        return 0
    if mode == "stall-after-init":
        time.sleep(hold)
        return 0
    if mode in ("tool-long", "ask-pending"):
        tool_use("toolu_fake_1", "sleep 300 && echo done")
        time.sleep(hold)
        tool_result("toolu_fake_1", "done")
        text_reply("the tool finished")
        result("the tool finished")
        return 0
    if mode in ("write", "sleep"):
        start = time.time()
        tag = os.environ.get("FAKE_CLAUDE_TAG", "job")
        if mode == "write":
            ledger = os.environ["FAKE_CLAUDE_LEDGER"]
            try:
                with open(ledger, encoding="utf-8") as fh:
                    before = fh.read()
            except FileNotFoundError:
                before = ""
            time.sleep(hold)
            with open(ledger, "w", encoding="utf-8") as fh:
                fh.write(before + tag + "\n")
            own = os.environ.get("FAKE_CLAUDE_FILE")
            if own:
                with open(own, "w", encoding="utf-8") as fh:
                    fh.write(f"written by {tag}\n")
        else:
            time.sleep(hold)
        end = time.time()
        trace = os.environ.get("FAKE_CLAUDE_TRACE")
        if trace:
            with open(trace, "a", encoding="utf-8") as fh:
                fh.write(json.dumps({"tag": tag, "start": start, "end": end,
                                     "cwd": os.path.realpath(os.getcwd())}) + "\n")
        text_reply(f"{tag} done")
        result(f"{tag} done")
        return 0
    if mode == "apply":
        with open(os.environ["FAKE_CLAUDE_CHANGES"], encoding="utf-8") as fh:
            changes = json.load(fh)
        tool_use("toolu_fake_apply", "apply the change")
        root = os.getcwd()

        def target(rel):
            path = os.path.join(root, rel)
            os.makedirs(os.path.dirname(path) or root, exist_ok=True)
            return path

        for rel, text in changes.get("write", {}).items():
            # newline="": the bytes on disk are exactly the text given.
            with open(target(rel), "w", encoding="utf-8", newline="") as fh:
                fh.write(text)
        for rel, data in changes.get("write_b64", {}).items():
            with open(target(rel), "wb") as fh:
                fh.write(base64.b64decode(data))
        for rel in changes.get("delete", []):
            os.remove(os.path.join(root, rel))
        n = len(changes.get("write", {})) + len(changes.get("write_b64", {})) + len(changes.get("delete", []))
        tool_result("toolu_fake_apply", f"{n} files changed")
        text_reply("applied the change")
        result("applied the change")
        return 0
    if mode in ("grandchild", "grandchild-setsid"):
        # start_new_session=True is setsid() in the child: portable, where a
        # `setsid` binary exists on Linux only.
        child = subprocess.Popen(["sleep", str(int(hold) + 60)],
                                 start_new_session=(mode == "grandchild-setsid"))
        pidfile = os.environ.get("FAKE_CLAUDE_PIDFILE")
        if pidfile:
            with open(pidfile, "w", encoding="utf-8") as fh:
                fh.write(str(child.pid))
        text_reply("started a helper")
        time.sleep(hold)
        return 0
    sys.stderr.write(f"fake_claude: unknown FAKE_CLAUDE_MODE {mode!r}\n")
    return 2


if __name__ == "__main__":
    sys.exit(main())
