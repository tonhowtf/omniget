# OmniGet — Claude Code plugin

Paste a video, audio, or social-post URL into Claude Code and get the media file,
its caption, and a transcript, ready to summarize, quote, or research.

It reuses whatever OmniGet already installed. Tool lookup order, mirroring the
desktop app's own resolver: `omniget-cli` on `PATH` → the binaries OmniGet
manages in its app-data folder → the system `PATH`. Nothing here requires the
desktop app; with `yt-dlp` and `ffmpeg` on `PATH` it works on its own.

> Full install guide, per-OS notes, updating, and private-content handling: **[INSTALL.md](INSTALL.md)**.

## Setup

Three steps. The first is the only one that's always required.

**1. Install the plugin** (in a Claude Code session):

```
/plugin marketplace add /path/to/omniget/claude-plugin
/plugin install omniget
```

**2. Install the tools** — one step, on any OS:

```
/omniget:setup
```

It checks for `yt-dlp` and `ffmpeg`, installs whatever is missing after a single
confirmation (Mac via Homebrew, Windows via winget/scoop), and downloads the prebuilt
`omniget-cli` for your OS/arch — OmniGet's native extractors for Instagram/X/Bilibili/Threads,
which are more reliable than raw yt-dlp for those sites. Add `--local` to also set up an
on-device Whisper engine and model, `--no-cli` to skip omniget-cli, or `--update` to refresh
`yt-dlp` and `omniget-cli` when a site breaks. You can also run it from a terminal:
`bash claude-plugin/omniget/scripts/setup.sh` (add `--yes` to skip the prompt).

If the OmniGet desktop app is installed, `yt-dlp` and `ffmpeg` are already managed by it,
so this step finds them and installs nothing.

**3. Add a transcription key (optional).** Captions and local Whisper need no key. For
cloud transcription (more accurate, no model download), add an OpenAI or Gemini key **in
your own terminal** — the input is hidden and the key never passes through Claude:

```bash
bash claude-plugin/omniget/scripts/keys.sh set
```

Then confirm what works:

```bash
bash claude-plugin/omniget/scripts/keys.sh check
```

> Only OpenAI and Gemini are supported for transcription. OpenRouter has no
> speech-to-text endpoint, so it is not offered here.

### Per-OS notes

- **macOS** — needs [Homebrew](https://brew.sh). `/omniget:setup` then installs
  `yt-dlp`, `ffmpeg`, and (with `--local`) `whisper-cpp` automatically.
- **Windows** — needs winget (App Installer, from the Microsoft Store) or scoop.
  `/omniget:setup` installs `yt-dlp` and `ffmpeg`. For local transcription, prefer a
  cloud key or install whisper.cpp manually; captions and cloud both work out of the box.
- **Linux** — `/omniget:setup` detects `apt`/`dnf`/`pacman`/`zypper` and runs it (with
  `sudo`, so it needs a terminal), or prints the exact commands if it can't. Distro
  `yt-dlp` is often outdated — `pipx install yt-dlp` (what setup uses on apt) stays current.
  For local Whisper, install `whisper.cpp` from your distro or the upstream release and
  fetch a model with `scripts/get-model.sh`.

## Commands

| Command | Does |
|---|---|
| `/omniget:fetch <url> [--audio] [--quality N]` | Download the media to `~/Downloads/omniget` |
| `/omniget:transcribe <url\|file> [--backend B] [--lang xx] [--summarize]` | Transcribe and optionally summarize |
| `/omniget:research <url> [--backend B]` | Caption + transcript distilled into a Markdown note with `[mm:ss]` references |
| `/omniget:setup` | Install missing tools + `omniget-cli` (confirm once), check API keys; `--update` refreshes them |
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
`setup.sh` (cross-OS installer), `keys.sh` (add/test keys), `doctor.sh`,
`resolve-tools.sh` (shared lookup, sourced by the rest), `fetch.sh`, `research.sh`,
`transcribe.sh`, `get-model.sh`, plus `srt_tools.py`, `gemini_transcribe.py`,
`openai_transcribe.sh`. Run the tests with `python3 -m unittest discover -s tests`
and `bash tests/test_keys.sh`.

## Scope

Downloads only what the user asks for. Never bypasses DRM or paywalls; it uses the
session the user already has (via OmniGet cookie accounts or `--cookies-from-browser`).
Captions cover the post description; comments are out of scope.
