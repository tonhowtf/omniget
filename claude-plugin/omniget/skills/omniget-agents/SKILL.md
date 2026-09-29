---
name: omniget-agents
description: Use only when the user explicitly asks to delegate, hand off or run a task in the background with one of their OmniGet agents ("have my OmniGet agent do X", "delegate this to codex in OmniGet", "run this as an OmniGet job", "omniget agent run ..."). Do not use for ordinary coding requests, for downloading media, or when the environment variable OMNIGET_JOB_ID is set.
version: 0.2.0
---

# Delegating a task to an OmniGet agent

OmniGet (the desktop app) runs the user's agents as background jobs. This skill
starts one, waits for it and reports what it did. It is a narrow tool: when the
user did not ask for delegation, do the work yourself.

## First, check where you are

```bash
printenv OMNIGET_JOB_ID
```

If it prints anything, **you are already an OmniGet job. Never delegate from
here** (the depth is 1): do the task yourself, in this turn. `omniget agent run`
refuses inside a job and the app refuses too, so trying only wastes a turn.

## Delegate

The full reference is one command away; read it before the first delegation:

```bash
omniget guide agents
```

The usual flow:

1. Look for the project's contract: `omniget agent workflow show`.
   - If the repository has `.omniget/workflow.md` in effect, it already picks
     the agent, run mode, isolation, deadline and prompt template: pass only
     the task, no `--agent`, `--mode` or `--isolate`.
   - Without one, pick the agent: `omniget agent agents` lists the roster ids.
   - If it says "not trusted", its hooks and checks are skipped. Tell the user;
     only they trust it (in OmniGet or with `omniget agent workflow trust`).
     Never run `workflow trust` yourself.
2. Start and wait, from the project folder:
   ```bash
   omniget agent run [--agent <id>] --title "<short name>" --command-id <unique-id> "<task>"
   ```
   - `--command-id` makes a retry safe: the same id returns the same job.
   - Write the task so it stands alone: the agent does not see this chat.
   - `--detach` returns the job id at once; follow later with `omniget agent jobs <id>`.
3. If the job stops at `waiting_approval`, tell the user and show
   `omniget agent approvals --job <id>`. Answer (`omniget agent approve <approval id>`)
   only when the user tells you to: approving runs the agent's command.
4. Report from one call, `omniget agent report <id>` (the MCP tool `job_report`
   when you have the OmniGet MCP connection with the agents_run grant). It gives
   the verdict, the agent's `OUTCOME:` line, the host's checks, the files changed
   and a `next` hint. Tell the user in two or three sentences:
   - `needs_review`: the job changed files; `omniget agent diff <id>` shows them.
   - `unreported`: the agent ended without saying how it went; read the result
     before relying on it.
   - `checks_failed` or `failed_by_agent`: say it did not work, with the reason.

Exit code 1 of `agent run` means the job failed, was cancelled, or the agent said
`OUTCOME: failed`.
