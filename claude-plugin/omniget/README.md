# OmniGet — Claude Code plugin

Paste a video, audio, or social-post URL into Claude Code and get the media file,
its caption, and a transcript, ready to summarize, quote, or research.

It reuses whatever OmniGet already installed. Tool lookup order, mirroring the
desktop app's own resolver: `omniget-cli` on `PATH` → the binaries OmniGet
manages in its app-data folder → the system `PATH`. Nothing here requires the
desktop app; with `yt-dlp` and `ffmpeg` on `PATH` it works on its own.

## Install

```
/plugin marketplace add /path/to/omniget/claude-plugin
/plugin install omniget
```

Then run `/omniget:doctor` to see what is available and what to add.

## Commands

| Command | Does |
|---|---|
| `/omniget:fetch <url> [--audio] [--quality N]` | Download the media to `~/Downloads/omniget` |
| `/omniget:transcribe <url\|file> [--backend B] [--lang xx] [--summarize]` | Transcribe and optionally summarize |
| `/omniget:research <url> [--backend B]` | Caption + transcript distilled into a Markdown note with `[mm:ss]` references |
| `/omniget:doctor` | Report available tools, models, and keys, plus how to add the missing ones |

The two skills (`omniget-fetch`, `omniget-transcribe`) trigger on their own when a
media URL is pasted with a request, so the slash commands are optional.

## Transcription backends

`--backend auto` (the default) walks this ladder and uses the first that works:

1. **captions** — platform subtitles via `yt-dlp`. Free, instant.
2. **local** — `whisper-cli` (whisper.cpp) with a ggml model. Free, offline, private.
3. **mlx** — `mlx_whisper` on Apple Silicon. Free, offline, faster than whisper-cli.
4. **gemini** — Gemini 2.5 Flash, needs `GEMINI_API_KEY`. Cents per hour, audio-native.
5. **openai** — needs `OPENAI_API_KEY`. `whisper-1` returns timestamps; `--model gpt-4o-transcribe` is more accurate but text-only.

Force one with `--backend local|mlx|gemini|openai|captions`. API keys are read from
the environment or `~/.config/ai-keys.env` and never printed.

## Getting the pieces

Nothing is installed for you; `/omniget:doctor` prints the commands and you decide.

- **yt-dlp + ffmpeg**: `brew install yt-dlp ffmpeg` (macOS), `winget install yt-dlp.yt-dlp Gyan.FFmpeg` (Windows), your distro's packages (Linux).
- **local Whisper**: `brew install whisper-cpp`, or the release bundle from `ggml-org/whisper.cpp`. On Apple Silicon `uv tool install mlx-whisper` is faster.
- **a model**: `scripts/get-model.sh large-v3-turbo-q5_0` (547 MB, SHA-256 verified from `ggerganov/whisper.cpp`). Smaller: `base` (141 MB), `small` (465 MB).
- **cloud keys**: `export GEMINI_API_KEY=...` / `export OPENAI_API_KEY=...`, or put them in `~/.config/ai-keys.env`.

## Environment variables

| Variable | Meaning |
|---|---|
| `OMNIGET_DIR` | Output directory (default `~/Downloads/omniget`) |
| `OMNIGET_DATA_DIR` | OmniGet app-data dir (auto-detected per OS) |
| `OMNIGET_TOOL_YT_DLP`, `OMNIGET_TOOL_FFMPEG`, ... | Explicit binary paths |
| `OMNIGET_WHISPER_MODEL` | Explicit ggml model file for the local backend |
| `OMNIGET_COOKIES_FROM_BROWSER` | `chrome` / `firefox` / `safari` / `edge` for login-gated media |
| `OMNIGET_KEYS_FILE` | Alternative to `~/.config/ai-keys.env` |

## Scripts

The skills call `scripts/`; each prints one JSON line on success, progress on stderr:
`doctor.sh`, `resolve-tools.sh` (shared lookup, sourced by the rest), `fetch.sh`,
`research.sh`, `transcribe.sh`, `get-model.sh`, plus `srt_tools.py`,
`gemini_transcribe.py`, `openai_transcribe.sh`. Run the tests with
`python3 -m unittest discover -s tests`.

## Scope

Downloads only what the user asks for. Never bypasses DRM or paywalls; it uses the
session the user already has (via OmniGet cookie accounts or `--cookies-from-browser`).
Captions cover the post description; comments are out of scope.
