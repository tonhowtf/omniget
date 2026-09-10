---
description: Transcribe a video/audio URL or local file (captions, local Whisper, Gemini or OpenAI) and summarize on request
argument-hint: <url|file> [--backend auto|local|mlx|gemini|openai] [--lang xx] [--summarize]
allowed-tools: ["Bash", "Read", "Write"]
---

Transcribe the media in the arguments below using the `omniget-transcribe` skill.

1. If `--summarize` is present, remove it from the arguments and remember to summarize.
2. For a URL, first run `bash "${CLAUDE_PLUGIN_ROOT}/scripts/research.sh" "<url>"` to get the
   title and caption.
3. Run `bash "${CLAUDE_PLUGIN_ROOT}/scripts/transcribe.sh" <remaining arguments>`.
4. Read the transcript file named in the JSON output.
5. Reply with the transcript path, the backend used, and, when asked, a summary with
   `[mm:ss]` references. Otherwise give a two-sentence description of the content.
