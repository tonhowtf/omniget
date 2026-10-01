---
description: Connect Claude Code to the OmniGet desktop app on this computer (downloads, your AI agents as background jobs, missions) with one click in OmniGet
allowed-tools: ["Bash"]
---

Connect this Claude Code to OmniGet's local MCP server.

1. Get the current steps from the OmniGet app on this computer. It listens on one of the
   ports 47720 to 47729; use the first answer that starts with `# Connect`:

   for p in 47720 47721 47722 47723 47724 47725 47726 47727 47728 47729; do r=$(curl -s -m 2 "http://127.0.0.1:$p/connect?client=claude"); case "$r" in "# Connect"*) echo "$r"; break;; esac; done

   (Windows PowerShell: try `curl.exe -s "http://127.0.0.1:<port>/connect?client=claude"` on each port.)
   If no port answers that way, OmniGet is not open or is older than this feature: ask the
   person to open or update it (https://getomniget.com/download) and try again.
2. Follow the steps it returns. The user clicks Allow in the OmniGet window; OmniGet
   writes the `omniget` server into Claude Code's config itself, so never ask for a token.
