---
description: "Minutes and action items of your OmniGet meetings: the latest one, one by name, or everything still open"
argument-hint: [meeting name | "todo" | owner]
allowed-tools: ["Bash", "Read"]
---

Use the `omniget-meetings` skill.

- With no arguments: `omniget meetings list --json`, take the newest meeting that is `ready`, and show
  `omniget meetings show <id> --no-transcript`. Answer with a five-line summary, the decisions and the
  action items (owner, due).
- With `todo` (or a person's name): `omniget meetings todo --json [--owner "<name>"]` and group the open
  items by owner, oldest meeting first, with the meeting each came from.
- With anything else: `omniget meetings search "<arguments>" --json`, pick the best matching meeting and
  summarize it as above, quoting the transcript lines that answer the question with their `[mm:ss]`.

Do not mark items done, post anything or send bots unless the user asks.
