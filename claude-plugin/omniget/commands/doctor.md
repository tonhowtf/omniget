---
description: Check which OmniGet tools, Whisper models and speech API keys this machine has, and how to add missing ones
allowed-tools: ["Bash"]
---

Run `bash "${CLAUDE_PLUGIN_ROOT}/scripts/doctor.sh"` and explain the result in plain words:
what works today (captions, local Whisper, Gemini, OpenAI), what is missing, and the exact
command that would add each missing piece. Do not run any install command unless the user
answers yes.
