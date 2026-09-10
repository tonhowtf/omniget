# Installing the OmniGet skill

A Claude Code plugin that downloads media, captions, and transcripts from a pasted URL.
It reuses OmniGet's own tools when present and installs what's missing.

## What it needs

| Purpose | Tool | Required? |
|---|---|---|
| Download / metadata / captions | `yt-dlp` + `ffmpeg` | yes |
| Reliable Instagram / X / Bilibili / Threads | `omniget-cli` (prebuilt) | recommended |
| Local, offline transcription | `whisper-cli` (whisper.cpp) or `mlx_whisper` + a model | optional |
| Cloud transcription | OpenAI **or** Gemini API key | optional |

You do not need all of these. Captions alone need only `yt-dlp` + `ffmpeg`. Any one
transcription option (captions, local model, or a cloud key) is enough.

## 1. Install the plugin

In a Claude Code session:

```
/plugin marketplace add /path/to/omniget/claude-plugin
/plugin install omniget
```

## 2. Install the tools — one step

```
/omniget:setup
```

`/omniget:setup` detects your OS and:
- installs `yt-dlp` and `ffmpeg` if missing (after one confirmation),
- downloads the prebuilt `omniget-cli` for your OS/arch (native extractors for
  Instagram/X/Bilibili/Threads),
- reports which API keys work and how to add one.

Flags: `--yes` (no prompt), `--local` (also set up on-device Whisper + a model),
`--no-cli` (skip omniget-cli), `--update` (refresh yt-dlp + omniget-cli).

You can also run it straight from a terminal:

```bash
bash claude-plugin/omniget/scripts/setup.sh --yes
```

If the OmniGet **desktop app** is installed, `yt-dlp` and `ffmpeg` are already managed by
it and setup will find them.

### Per-OS

- **macOS** — needs [Homebrew](https://brew.sh). Installs `yt-dlp`, `ffmpeg`, and (with
  `--local`) `whisper-cpp` via brew. `omniget-cli` is downloaded for Apple Silicon or Intel.
- **Windows** — needs winget (App Installer from the Microsoft Store) or scoop. Installs
  `yt-dlp` and `ffmpeg`; `omniget-cli` (x64) is downloaded. For local Whisper, prefer a
  cloud key.
- **Linux** — setup runs `apt`/`dnf`/`pacman`/`zypper` (needs `sudo`, so run it in a
  terminal) or prints the commands. `omniget-cli` is downloaded for x86_64 (no prebuilt
  arm64 CLI — those machines use `yt-dlp` for every site). Distro `yt-dlp` is often stale;
  `pipx install yt-dlp` (what setup uses on apt) stays current.

## 3. Add a transcription key (optional)

Captions and local Whisper need no key. For cloud transcription, add an OpenAI or Gemini
key **in your own terminal** — the input is hidden and the key never passes through Claude:

```bash
bash claude-plugin/omniget/scripts/keys.sh set
```

Confirm what works:

```bash
bash claude-plugin/omniget/scripts/keys.sh check
```

> Only OpenAI and Gemini transcribe. OpenRouter has no speech-to-text endpoint and is not
> supported here.

## Keeping it working

Sites change and extractors break — this is normal for every downloader. When something
that used to work stops:

```bash
bash claude-plugin/omniget/scripts/setup.sh --update
```

That upgrades `yt-dlp` and re-downloads the latest `omniget-cli`.

## Private and rate-limited content (Instagram, X)

No downloader "always works" for Instagram/X. They rate-limit by IP and gate content
behind login, and even OmniGet depends on a signed-in session for much of it. If a fetch
returns *"empty media response"* or *HTTP 400/429*:

- It's usually a **temporary rate-limit** — wait a few minutes and retry.
- If the post is **private or login-gated**, the skill **retries automatically** using the
  browser you're logged into (auto-detected). On macOS the first such read raises a one-time
  Keychain prompt — allow it. If your browser has several profiles and the default isn't the
  logged-in one, pin it: `OMNIGET_COOKIES_FROM_BROWSER=chrome:"Profile 3"`. Logging into the
  site in the OmniGet app also works — those cookies are reused first, before any browser.

Public content on YouTube, TikTok, Reddit, Vimeo, Twitch, and the ~1,800 yt-dlp sites is
reliable and needs none of this.

## Troubleshooting

- `/omniget:doctor` — full status: tools, models, and (with the command) live key checks.
- `yt-dlp not found` → run `/omniget:setup`.
- A site suddenly fails → `setup.sh --update`.
- Everything green but one URL fails → check the hint the script prints; it distinguishes
  rate-limits, login-required, and DRM.

## Uninstall

```
/plugin uninstall omniget
```

Downloaded tools/models live in `~/.cache/omniget-skill/` (and, if you used brew/winget,
in the usual system locations); remove them by hand if you want the space back.
