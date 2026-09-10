---
name: omniget-transcribe
description: This skill should be used when the user shares a video, audio, podcast or social-post URL or a local media file and asks "what does this video say", "transcribe this", "summarize this video", "get the transcript", "what is this reel about", "research this post", "pull the caption", "turn this talk into notes", "quote the part where", "translate what they say", or wants to analyze spoken content from a link. Covers YouTube, TikTok, Instagram, X/Twitter, Reddit, Vimeo, Twitch, Bilibili, Threads and any local .mp4/.mp3/.wav/.m4a.
version: 0.1.0
---

# Transcribing and researching media

Turn a link or file into text with `${CLAUDE_PLUGIN_ROOT}/scripts/`, then read the
transcript and answer. Claude cannot hear audio, so a speech-to-text step always runs first.
Each script prints one JSON line on success; progress goes to stderr.

## Pipeline

1. **Describe the source** (URL only, no download):
   ```bash
   bash "${CLAUDE_PLUGIN_ROOT}/scripts/research.sh" "<url>"
   ```
   Returns `title`, `platform`, `uploader`, `duration`, `has_captions`, and `caption`
   (the post text or description) when the platform provides one. Skip this step for local files.
2. **Get the transcript**:
   ```bash
   bash "${CLAUDE_PLUGIN_ROOT}/scripts/transcribe.sh" "<url-or-file>" [--backend auto|captions|local|mlx|gemini|openai] [--model NAME] [--lang xx] [--out DIR]
   ```
   Returns `transcript` (Markdown with `[mm:ss]` markers), `srt` when timestamps exist,
   `backend`, `model`, `duration`, `chars`. Files land in `~/Downloads/omniget/transcripts`
   unless `--out` or `OMNIGET_DIR` says otherwise.
3. **Read the `.md` file** and do what was asked: summarize, extract key points, quote with
   timestamps, translate, or write a research note that combines the caption and the transcript.
   Cite moments as `[mm:ss]`. Do not paste the whole transcript back unless asked.

## Backend ladder (`--backend auto`)

| Order | Backend | Needs | Cost | Notes |
|---|---|---|---|---|
| 1 | `captions` | platform subtitles | free, seconds | tried first for every URL; auto-captions are rough but usable |
| 2 | `local` | `whisper-cli` + a ggml model | free, private | CPU/Metal; `large-v3-turbo-q5_0` is the default model choice |
| 3 | `mlx` | `mlx_whisper` (Apple Silicon) | free, private | faster than whisper-cli on M-series Macs |
| 4 | `gemini` | `GEMINI_API_KEY` | cents per hour | audio-native; good for long recordings |
| 5 | `openai` | `OPENAI_API_KEY` | about $0.006 per minute | `whisper-1` returns timestamps; `--model gpt-4o-transcribe` is more accurate but text-only |

Choose deliberately when the user states a preference:
- "most accurate" with an OpenAI key: `--backend openai --model gpt-4o-transcribe`.
- "free" or "offline" or "private": `--backend local` (or `mlx` on a Mac).
- Known language: pass `--lang en` (or `pt`, `es`, ...) to every backend; it speeds up and sharpens results.
- Recording longer than two hours: say so and confirm before a cloud backend runs.

Keys are read from the environment or `~/.config/ai-keys.env`. Never print them. To add one, the user runs `bash "${CLAUDE_PLUGIN_ROOT}/scripts/keys.sh" set` in their own terminal (hidden input); never ask them to paste a key here. `keys.sh check` (or `doctor.sh --check-keys`) reports which keys work.

## Exit codes and what to do

| Exit | Meaning | Action |
|---|---|---|
| 3 | `--backend captions` and the platform has none | rerun with `--backend auto` |
| 4 | no local engine and no key | offer `setup.sh` (installs tools, confirm once) and tell the user to add a key with `keys.sh set` in their own terminal; `get-model.sh large-v3-turbo-q5_0` fetches a local model (547 MB) |
| 1 | tool missing or download failed | read the `-> hint` the script printed (rate-limit / login / DRM); run `doctor.sh`, or `setup.sh --update` if a site broke; for login errors see the `omniget-fetch` error table |

## Rules

- Never start an install or a model download without the user's explicit yes.
- Summaries state which backend produced the transcript when accuracy matters (auto-captions vs. Whisper).
- Keep `.srt` and `.md` next to each other; the user may open the SRT in OmniGet's Subtitle Workshop.
