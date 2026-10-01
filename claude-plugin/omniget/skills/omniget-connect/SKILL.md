---
name: omniget-connect
description: This skill should be used when the user asks to "connect to OmniGet", "set up OmniGet", "configure OmniGet for me", "use OmniGet from Claude", or wants Claude Code to run their OmniGet agents, missions or downloads through OmniGet's MCP server, and the omniget MCP tools are not available yet.
version: 0.1.0
---

# Connecting Claude Code to OmniGet

OmniGet (the desktop app) serves an MCP server on this computer. Connecting is a
one-click approval in the OmniGet window; OmniGet then writes the `omniget` server
into Claude Code's own config. The token never passes through this conversation.

1. Get the current steps from the OmniGet app on this computer. It listens on one of the
   ports 47720 to 47729; use the first answer that starts with `# Connect`:

   for p in 47720 47721 47722 47723 47724 47725 47726 47727 47728 47729; do r=$(curl -s -m 2 "http://127.0.0.1:$p/connect?client=claude"); case "$r" in "# Connect"*) echo "$r"; break;; esac; done

   (Windows PowerShell: try `curl.exe -s "http://127.0.0.1:<port>/connect?client=claude"` on each port.)
   If no port answers that way, OmniGet is not open or is older than this feature: ask the
   person to open or update it (https://getomniget.com/download) and try again.
2. Follow the steps it returns, then tell the user to start a new Claude Code session
   so the `omniget` tools load.
