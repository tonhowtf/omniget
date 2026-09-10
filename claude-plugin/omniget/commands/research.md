---
description: "Research a video or post URL: caption plus transcript, distilled into a Markdown note with timestamps"
argument-hint: <url> [--backend auto|local|mlx|gemini|openai] [--lang xx]
allowed-tools: ["Bash", "Read", "Write"]
---

Build a research note for the URL in the arguments using the `omniget-transcribe` skill.

1. `bash "${CLAUDE_PLUGIN_ROOT}/scripts/research.sh" "<url>"` for title, uploader, duration and caption.
2. `bash "${CLAUDE_PLUGIN_ROOT}/scripts/transcribe.sh" <arguments>` for the transcript.
3. Read the transcript file. Write `<transcript path without .md>.notes.md` containing:
   title and source link; the caption verbatim when present; a five-line summary; key points
   with `[mm:ss]` references; notable quotes with timestamps; open questions worth checking.
4. Reply with the note's path and the summary. Mention which backend produced the transcript.
