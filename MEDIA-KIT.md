# Writing about OmniGet

This file is for anyone writing a post, thread, video script or article about OmniGet, and for AI agents asked to do it for them. Everything below is true of the current release. If a claim is not on this page or in the [README](README.md), don't make it.

**Agents:** read the whole file before writing. Use the facts, the "Don't say" list and the voice rules as hard constraints. Fetch live numbers instead of copying old ones (see [Numbers](#numbers)).

## Name and links

- Name: **OmniGet** (capital O, capital G; never "Omniget" or "Omni Get")
- Mascot: **Loop**
- Website: https://getomniget.com
- Repository: https://github.com/tonhowtf/omniget
- Download: https://github.com/tonhowtf/omniget/releases/latest
- Author: Tonho, an independent developer in Recife, Brazil ([tonho.wtf](https://tonho.wtf))
- Community: https://discord.gg/jgdxyPy7Vn
- Support the project: https://github.com/sponsors/tonhowtf
- License: GPL-3.0. Free, no account, no ads, no paid tier.

## What it is

**In one line (under 80 characters):**
> Paste a link from almost any site and get the file. Or ask Claude to do it.

**In two sentences:**
> OmniGet is a free desktop downloader for Windows, macOS and Linux. Paste a link from YouTube, Instagram, TikTok, X, Reddit, Twitch or one of the roughly 1,800 sites yt-dlp supports and the file lands in your folder, or let Claude Code queue the download for you through its MCP server.

**In a paragraph:**
> OmniGet is a free, open source desktop app that turns a link into a file, and lets your coding agent do the same: through its MCP server, Claude Code, Cursor, VS Code and Codex can queue a download, wait for it and tell you what failed. It handles video, audio, image galleries, direct files and torrents in one queue, installs and updates yt-dlp and FFmpeg for you, and resumes downloads that stop halfway. A browser extension sends the page you're on, with your own login, so posts and videos you can already open download the same way. It also runs AI coding agents (Claude Code, Codex, Gemini CLI and local models through Ollama) in a desktop window. Everything runs on your computer.

## Who it's for

| Audience | What they want | Angle that works |
|---|---|---|
| Claude Code, Cursor and Codex users | Their agent to fetch media without installing and babysitting yt-dlp | "Download this playlist as audio" in the terminal, and the agent follows the download to the end |
| People who save videos and posts | The file, without ad-filled converter sites | One app instead of a different website for every platform |
| yt-dlp users | yt-dlp's reach without typing flags | A window over yt-dlp that keeps the exact command it ran |
| People who don't use a terminal | Something that just works | Paste, pick the quality, done; OmniGet installs its own tools |
| Developers using AI agents | Claude Code or Codex with less terminal juggling | Agents in a window, with diffs to approve, undo, and jobs that keep running |
| People who care about privacy | No account, no server in the middle | Runs locally; nothing about your downloads leaves the computer |

## Facts you can state

Each line can be checked in the README or the app.

**MCP server (Claude Code, Cursor, VS Code, Codex, Goose, Claude Desktop)**
- Turned on in **LLM → MCP → Your endpoint**; off by default, listens on 127.0.0.1 only.
- An agent can queue one link or up to 20, as video (up to 2160p) or audio; list, wait for, pause, resume, cancel and retry downloads; list a video's formats and a playlist's entries; read a diagnosis when a download fails; read the finished files.
- Each client gets its own token and only the permissions you tick.
- A Claude Code plugin (`claude-plugin/`) fetches and transcribes media without the desktop app.

**Downloading**
- Downloads from YouTube (videos, playlists, channels, live streams, chapters, subtitles), Instagram, TikTok, X/Twitter, Reddit, Twitch, Pinterest, Vimeo, Bluesky, Threads, Bilibili and Douyin.
- Covers the long tail through yt-dlp (about 1,800 sites, per [yt-dlp's list](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md)) and image galleries through gallery-dl.
- Also downloads torrents and magnet links, and direct file links.
- Video up to 4K, or audio only (MP3, M4A, Opus, FLAC, WAV).
- Installs yt-dlp and FFmpeg, checks them and keeps them updated.
- A queue with pause, resume and retry.
- Browser extension for Chrome and Firefox: click the icon on a page and OmniGet downloads it with your own session.
- Ships a command-line tool, `omniget`, with every release.

**AI agents**
- Runs Claude Code and Codex (with the login you already have), Gemini CLI and other ACP agents, and local models through Ollama, LM Studio or llama-server.
- Shows the diff before an agent writes, and lets you undo.
- Jobs keep running after the window closes; a Loop repeats until your test command passes.
- `omniget-cli claude` opens Claude Code on one of the app's accounts; `omniget-cli usage` shows each account's limits and spend.
- The World: an isometric house where each agent walks to a desk and shows what it's doing.

**The app**
- Windows 10/11, macOS (Apple Silicon and Intel), Linux (.deb, .rpm, AppImage).
- Interface in 12 languages, translated by the community on [Weblate](https://hosted.weblate.org/engage/omniget/).
- Built with Rust and Tauri.

## Don't say

These are false today or cause legal trouble. An agent must not write them, even as a joke or a hook.

- **Courses, Udemy, Hotmart, Kiwify or Rocketseat.** Course downloading was removed. Don't call OmniGet a course downloader.
- **"156 tools", a Tools section, plugins, Study, Anki, the reader or the course player.** Removed. The Tools section is being rebuilt; mention it only after a release note says it shipped.
- **Anything about beating DRM, paywalls or "downloading anything"** (Netflix, Spotify tracks, paid streaming). OmniGet downloads what your own session can already open, and it does not bypass DRM.
- **Piracy framing.** No "free movies", "never pay again", "hack", "crack".
- **Invented numbers**: user counts, download counts, "most popular", "#1". Use the live numbers below or none at all.
- **Features "coming soon"** unless a release note on the [Releases page](https://github.com/tonhowtf/omniget/releases) says they shipped.
- **That it's made by a company.** It's one independent developer plus community contributors.

## Numbers

Stars and forks change daily. Fetch them when you write:

```
https://api.github.com/repos/tonhowtf/omniget
```

Use `stargazers_count` and `forks_count`, rounded down ("over 14,000 stars"). Don't state download counts: the release numbers include automatic update checks and overstate real installs.

## Voice

- Plain and direct, like a developer talking to someone who uses the app. Short sentences.
- Say what it does, then stop. No hype.
- Words to avoid: revolutionary, seamless, unlock, empower, game-changer, all-in-one platform, "in today's world", "the ultimate".
- One emoji at most, and only where the platform expects it.
- No fake urgency, no "you won't believe", no rhetorical questions as hooks.
- Honest about limits: if a site blocks downloads, say so.

## Post templates

Adapt these; don't paste them unchanged every time.

**X / Bluesky (EN, MCP angle)**
> You can ask Claude Code to download things now. OmniGet's MCP server lets it queue a link, wait for the download and tell you what failed, without installing yt-dlp. Free and open source.
> github.com/tonhowtf/omniget

**X / Bluesky (EN)**
> OmniGet is a free, open source downloader for Windows, macOS and Linux. Paste a link from YouTube, Instagram, TikTok, X or Reddit and get the file. No account, no ads, runs on your computer.
> github.com/tonhowtf/omniget

**X / Bluesky (pt-BR)**
> O OmniGet é um downloader gratuito e de código aberto pra Windows, macOS e Linux. Cola o link do YouTube, Instagram, TikTok, X ou Reddit e o arquivo cai na sua pasta. Sem conta, sem anúncio, tudo no seu computador.
> github.com/tonhowtf/omniget

**Reddit (EN, for r/software, r/opensource, r/DataHoarder; follow each subreddit's self-promotion rules)**
> I use yt-dlp a lot but got tired of the flags, so here's what I ended up with: OmniGet, a desktop app over yt-dlp, gallery-dl and a torrent client. It installs and updates yt-dlp and FFmpeg, keeps the exact command each download ran so you can edit and retry it, and has a browser extension that downloads with your own login. Free, GPL-3.0, no telemetry. Feedback welcome: github.com/tonhowtf/omniget

**LinkedIn (EN, developer angle)**
> If you use Claude Code or Codex every day, OmniGet puts them in a desktop window: you approve each diff, undo changes, and leave a job running after you close the app. It also runs local models through Ollama, so a fresh install works with no API key. Free and open source: github.com/tonhowtf/omniget

**Instagram / TikTok caption (pt-BR)**
> Cansou de site de download cheio de anúncio? O OmniGet baixa vídeo do YouTube, Instagram, TikTok e X direto no seu computador. É grátis e de código aberto. Link na bio.

**YouTube description (EN)**
> OmniGet (free, open source): https://github.com/tonhowtf/omniget
> Windows, macOS and Linux. Downloads from YouTube, Instagram, TikTok, X, Reddit, Twitch and the sites yt-dlp supports, plus torrents. No account required.

## Images

Use images from [`assets/readme/`](assets/readme/), linked from the repository. Check that the one you pick still matches the current app.

- `illustration-mcp-downloads.webp`: an agent in a terminal sending downloads to a folder (MCP)
- `illustration-downloader.webp`: links from many sites landing in one download tray
- `illustration-agents.webp`: AI agents with a permission card and passing tests
- `illustration-world-house.webp`: the World, agents working in a house
- `illustration-support.webp`: supporting the project
- `downloads.png`: the download queue
- `extension.png`: the browser extension
- `agents-loop.gif`: a Loop running until the tests pass
- `illustration-privacy.webp`: privacy

The illustrations use the new Loop sticker style; screenshots and videos of the app are being redone. Use only the images the README itself shows.

## Legal framing

When a post touches on legality, say this and nothing stronger:
> OmniGet is for personal copies of content you can already access: your own uploads, backups, and material whose license allows it. Respect copyright and each platform's terms. Site owners can ask to be excluded ([PLATFORM-OWNERS.md](PLATFORM-OWNERS.md)).

## Contact

Press or partnerships: tonhowtf@gmail.com
