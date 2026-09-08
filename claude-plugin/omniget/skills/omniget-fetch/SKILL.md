---
name: omniget-fetch
description: This skill should be used when the user pastes a video, audio or social-post URL (YouTube, TikTok, Instagram, X/Twitter, Reddit, Vimeo, Twitch, Bilibili, Threads, Pinterest, direct .mp4/.m3u8 links) and asks to "download this", "get this video", "save the audio", "grab this reel", "fetch this clip", "pull the mp4", or otherwise wants the media file on disk. Also covers "what tools do I have for downloading" and "why did the download fail".
version: 0.1.0
---

# Fetching media with OmniGet's tooling

Download media through the plugin's scripts instead of hand-writing yt-dlp
invocations. The scripts reuse whatever the user already has, in this order:
`omniget-cli` on PATH, the binaries OmniGet manages inside its app-data folder,
then system `yt-dlp`/`ffmpeg`. Every script prints one JSON line on success.

Scripts live in `${CLAUDE_PLUGIN_ROOT}/scripts/`.

## Workflow

1. Run the download. Quote the URL.
   ```bash
   bash "${CLAUDE_PLUGIN_ROOT}/scripts/fetch.sh" "<url>" [--audio] [--quality 720] [--out DIR]
   ```
   Output: `{"file","title","duration","size","platform","engine","id"}`.
   - Default output directory is `~/Downloads/omniget` (override with `OMNIGET_DIR` or `--out`).
   - `--audio` keeps only the audio track as `.m4a`. `--quality N` caps the video height.
   - Instagram, Threads, X and Bilibili links go through `omniget-cli` when it is installed,
     which brings OmniGet's native extractors and cookie accounts.
2. Report the file path, size and duration in one or two sentences. Do not paste the JSON.
3. If the user only wants to know what the media says, use the `omniget-transcribe` skill
   instead of downloading video.

## When something fails

Run the doctor first, then act on what it reports:
```bash
bash "${CLAUDE_PLUGIN_ROOT}/scripts/doctor.sh"
```

| Symptom in stderr | Meaning | Action |
|---|---|---|
| `yt-dlp not found` | no engine available | show the doctor's install lines and ask before installing anything |
| `Sign in to confirm`, `login required`, `Private video`, HTTP 401/403 | platform wants a session | use OmniGet's cookies (automatic when the app has an account for that site) or set `OMNIGET_COOKIES_FROM_BROWSER=chrome` and retry |
| HTTP 429, `rate-limit` | too many requests | wait 30 s and retry once; then stop and tell the user |
| `DRM`, `SAMPLE-AES`, `Widevine` | protected stream | stop; explain that OmniGet does not bypass DRM |
| `Unsupported URL` | no extractor | say so; suggest the page's direct media link if the user has one |
| `Requested format is not available` | quality cap too strict | retry without `--quality` |

## Rules

- Downloads happen only when the user asked for the media. A bare URL with a question
  about its content is a transcription request, not a download request.
- Never run an install command (brew, winget, apt, pip, uv) without the user's explicit yes;
  the doctor prints the commands, the user decides.
- Cookies files and API keys are never printed, copied or committed.
- Prefer `--audio` for podcasts, talks and anything the user will only listen to or transcribe.

## Environment variables

`OMNIGET_DIR` (output), `OMNIGET_DATA_DIR` (OmniGet app data, auto-detected per OS),
`OMNIGET_TOOL_YT_DLP` / `OMNIGET_TOOL_FFMPEG` (explicit binaries),
`OMNIGET_COOKIES_FROM_BROWSER` (chrome, firefox, safari, edge).
