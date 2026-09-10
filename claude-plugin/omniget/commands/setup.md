---
description: "Set up the omniget skill: install missing tools for this OS (confirm once) and check API keys"
argument-hint: "[--yes] [--local]"
allowed-tools: ["Bash"]
---

Set up everything the skill needs, using the `omniget-fetch` / `omniget-transcribe` skills' scripts.

1. Run the setup script with any arguments the user passed:
   ```bash
   bash "${CLAUDE_PLUGIN_ROOT}/scripts/setup.sh" $ARGUMENTS
   ```
   - It checks for `yt-dlp` and `ffmpeg`, lists anything missing, installs it after one
     confirmation (Mac/Windows), and downloads `omniget-cli` (native Instagram/X/Bilibili
     extractors). `--yes` skips the prompt; `--local` adds a local Whisper engine + model;
     `--no-cli` skips omniget-cli; `--update` refreshes yt-dlp and omniget-cli.
   - It cannot type the user's API keys, and neither can you. It reports which keys work
     and prints the exact command for the user to run in their own terminal to add one.
2. Read the output and tell the user, in plain words: what is now installed, which
   transcription options are available to them (captions always; local if a model is
   present; Gemini/OpenAI if a key tested "working"), and — only if they have no working
   key and no local model — that to add a key they should run, in their own terminal:
   `bash "${CLAUDE_PLUGIN_ROOT}/scripts/keys.sh" set`
3. Do not ask the user to paste a key to you, and never run `keys.sh set` yourself — it
   needs their terminal and their keys must not pass through this conversation.
