---
description: Download the media at a URL with OmniGet's tooling (yt-dlp, ffmpeg, omniget-cli)
argument-hint: <url> [--audio] [--quality N] [--out DIR]
allowed-tools: ["Bash", "Read"]
---

Download the media for the arguments below using the `omniget-fetch` skill.

Run:
```bash
bash "${CLAUDE_PLUGIN_ROOT}/scripts/fetch.sh" $ARGUMENTS
```

Then report the saved file path, size and duration in one or two sentences. If the script
fails, run `bash "${CLAUDE_PLUGIN_ROOT}/scripts/doctor.sh"`, apply the error table from the
`omniget-fetch` skill, and ask before installing anything.
