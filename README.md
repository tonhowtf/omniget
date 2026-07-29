<!--
SEO / discovery: OmniGet is a free open source downloader for Windows, macOS and Linux.
GitHub caps topics at 20, so this list is exactly 20 — adding one means removing one.
Chosen for rankability: OmniGet is #1 by stars in course-downloader, udemy-downloader,
hotmart-downloader, bilibili-downloader, tiktok-downloader, twitter-downloader,
reddit-downloader and media-downloader. Broad topics (rust, svelte, tauri, desktop-app,
open-source) were dropped on purpose: the field there is 10k-110k repos deep and this repo
cannot rank, so the slots are worth more elsewhere.
downloader, download-manager, media-downloader, video-downloader, youtube-downloader,
yt-dlp, yt-dlp-gui, course-downloader, udemy-downloader, hotmart-downloader,
bilibili-downloader, tiktok-downloader, instagram-downloader, twitter-downloader,
reddit-downloader, telegram-downloader, twitch-downloader, subtitle-downloader,
epub-reader, spaced-repetition
-->

<p align="center">
  <img src="static/loop.png" alt="Loop, the OmniGet mascot" width="120" />
</p>

<h1 align="center">OmniGet</h1>

<h3 align="center">Download Udemy courses, YouTube, music, books, and 1,800+ sites in one app. No terminal.</h3>

<p align="center">
  <b>English</b>
  | <a href="README_zh_CN.md">中文</a>
  | <a href="README.ru.md">Русский</a>
</p>

<p align="center">
  <a href="https://github.com/tonhowtf/omniget/releases/latest"><img src="https://img.shields.io/github/v/release/tonhowtf/omniget?style=for-the-badge&label=release" alt="Latest Release" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-green?style=for-the-badge" alt="License GPL-3.0" /></a>
  <a href="https://github.com/tonhowtf/omniget/stargazers"><img src="https://img.shields.io/github/stars/tonhowtf/omniget?style=for-the-badge" alt="GitHub stars" /></a>
  <a href="https://github.com/tonhowtf/omniget/releases"><img src="https://img.shields.io/github/downloads/tonhowtf/omniget/total?style=for-the-badge&label=downloads" alt="Total downloads" /></a>
  <a href="https://hosted.weblate.org/engage/omniget/"><img src="https://hosted.weblate.org/widget/omniget/frontend-json/svg-badge.svg" alt="Translation status" /></a>
</p>

<p align="center">
  <b>OmniGet is a free, open source desktop application for Windows, macOS, and Linux.</b> It downloads online courses (Udemy, Hotmart, Kiwify, Skool, Teachable, and more), video and audio from YouTube, TikTok, Instagram, Twitter/X, Reddit, and 1,800+ other sites, plus music and books. Everything plays inside the app. No command line, no Python, no setup, and your files stay on your computer. Released under the GPL-3.0 license.
</p>

<p align="center">
  <a href="#download-and-install"><b>Download for Windows, macOS, or Linux</b></a>
  &nbsp;·&nbsp;
  <a href="#one-keypress-and-it-is-downloading"><b>See the one-click hotkey</b></a>
</p>

<p align="center">
  <img src="assets/readme/en/home-hero.png" alt="OmniGet home screen, a free downloader for courses, videos, music and books on Windows, macOS and Linux" width="880" />
</p>

---

## Download and install

Pick your system, download the latest release, and open it. There is no installer to click through and no admin rights are needed.

<table>
  <tr>
    <th>Platform</th>
    <th>How to install</th>
  </tr>
  <tr>
    <td><strong>Windows</strong></td>
    <td>
      <a href="https://github.com/tonhowtf/omniget/releases/latest"><img alt="Download OmniGet for Windows" src="https://img.shields.io/badge/Windows-Portable_EXE-0078D6?style=for-the-badge&logo=windows&logoColor=white" height="38"></a>
      <br/>
      <sub>Download the <code>.exe</code> from Releases and double click it. It is portable, so it runs from anywhere. There is also an <code>.msi</code> installer, and <code>winget install -e --id tonhowtf.OmniGet</code> if you prefer the command line.</sub>
    </td>
  </tr>
  <tr>
    <td><strong>macOS</strong></td>
    <td>
      <a href="https://github.com/tonhowtf/omniget/releases/latest"><img alt="Download OmniGet for macOS" src="https://img.shields.io/badge/macOS-DMG-000000?style=for-the-badge&logo=apple&logoColor=white" height="38"></a>
      <br/>
      <sub>Open the <code>.dmg</code> and drag OmniGet into your Applications folder. Read the first launch note below.</sub>
    </td>
  </tr>
  <tr>
    <td><strong>Linux</strong></td>
    <td>
      <a href="https://github.com/tonhowtf/omniget/releases/latest"><img alt="Download OmniGet for Linux as deb, rpm or AppImage" src="https://img.shields.io/badge/Linux-deb_·_rpm_·_AppImage-FFAA33?style=for-the-badge&logo=linux&logoColor=white" height="38"></a>
      <br/>
      <sub>Debian and Ubuntu: download the <code>.deb</code>. Fedora and openSUSE: the <code>.rpm</code>. Everything else: the <code>.AppImage</code>. x86_64 and ARM64 builds are both published.</sub>
    </td>
  </tr>
</table>

<sub><strong>AppImage on Debian 12+ or Ubuntu 24.04+:</strong> those releases ship without FUSE 2, which AppImage needs. If <code>./omniget.AppImage</code> fails with a libfuse error, run <code>sudo apt install libfuse2</code>, or start it with <code>./omniget.AppImage --appimage-extract-and-run</code>. The <code>.deb</code> avoids this entirely.</sub>

### ⚠️ Please read this before the first launch

OmniGet is open source and is not signed with a paid certificate, so the first time you open it your system may warn you. This is expected, and the steps below clear it for good. Your files stay local either way.

**macOS (this is the big one, the app will not open on the first try).** macOS Gatekeeper blocks unsigned apps. After you move OmniGet to Applications, open Terminal and run these two lines:

```bash
xattr -cr /Applications/omniget.app
codesign --force --deep --sign - /Applications/omniget.app
```

Then open OmniGet normally. You only do this once.

**Windows.** SmartScreen may show a blue warning on the first run. Click **More info**, then **Run anyway**. This is standard for open source apps without a paid code signing certificate.

### Portable mode, for a USB stick or a locked-down PC

Create an empty file named `portable.txt` (or `.portable`) next to the `.exe` and relaunch. OmniGet then keeps settings, the database, cookies, plugins, caches, and the bundled yt-dlp and FFmpeg in a `data` folder beside the executable. Nothing is written to `AppData\Roaming` or any other user folder, so the whole install travels on the stick. Without that file, OmniGet uses the standard per-user data directory.

Free and open source under GPL-3.0. Updates run quietly in the background. The bundled tools (yt-dlp and FFmpeg) install themselves, and yt-dlp is verified by SHA256 before it runs. Plugins install on first launch and update themselves too, with nothing for you to configure.

---

## One keypress, and it is downloading

This is the part people fall in love with. Copy any link, a YouTube video, a tweet, a Discord message, a track, a magnet, then press the global hotkey **`Ctrl+Shift+D`** (**`Cmd+Shift+D`** on macOS). OmniGet reads your clipboard and downloads it in the background. You do not even open the window.

<p align="center">
  <img src="assets/readme/global-hotkey.png" alt="OmniGet global download hotkey, press the shortcut and the link in your clipboard downloads into your folder" width="760" />
</p>

It works from anywhere on your system. Browsing, chatting, reading, it does not matter which app is in front. Copy, press, done. The file lands in your folder and the queue handles the rest. If you would rather see a preview first, just paste the link into the omnibox on the home screen, glance at the quality options, and click download.

---

## The problem this solves

You already have yt-dlp open in a terminal. You found a course downloader script that breaks on every site update. You have a separate app for music, and none of them talk to each other. Every download becomes three tools and a copy paste.

OmniGet does all of it in one window. Paste a course link, a YouTube link, a TikTok, a magnet, a podcast, and it figures out the rest. The file lands in your folder, and it plays right there in the app.

It is the only open source app that downloads a full Udemy or Hotmart course, video and audio from 1,800+ sites, and your music library, in one place, without the command line. It earned thousands of GitHub stars in its first months because that combination did not exist anywhere else.

---

## How OmniGet compares to yt-dlp and other downloaders

Different tools do different jobs. This is where OmniGet fits, not a scoreboard.

### OmniGet vs yt-dlp

[yt-dlp](https://github.com/yt-dlp/yt-dlp) is the extraction engine, and OmniGet uses it. yt-dlp is a command-line tool: you install Python, you learn the flags, you write the format selector, and you get an unmatched amount of control over 1,800+ sites. It is excellent at that, and OmniGet would not exist without it.

OmniGet is the app around it. You install one file, paste a link, and see a preview with quality options. yt-dlp and FFmpeg install and update themselves, and yt-dlp is verified by SHA256 before it runs. On top of that sit a queue that retries and resumes, a download history, a course player, a reader, and a music library. If you are comfortable in a terminal and only want files, use yt-dlp directly. If you want a library you can actually watch and read, use OmniGet.

### OmniGet vs single-purpose downloaders

Most downloaders do one site well. OmniGet covers courses, video, audio, images, torrents, and Telegram in one queue, and then plays the result. The trade-off is honest: a dedicated tool for one platform will sometimes support an edge case first.

### OmniGet vs paid course downloaders

Paid course downloaders typically charge a subscription for something you already own access to. OmniGet is GPL-3.0, has no account, no ads, and no paid tier, and it downloads only what your own logged-in session can already reach.

---

## What OmniGet downloads

Paste a link. OmniGet detects the site, shows a preview with quality options, and downloads. If [yt-dlp](https://github.com/yt-dlp/yt-dlp) supports a site, OmniGet downloads from it, which is roughly a thousand more than the table below.

| Category | Platforms |
|----------|-----------|
| Online courses | Hotmart, Udemy, Kiwify, Gumroad, Teachable, Kajabi, Skool, Wondrium, Thinkific, Rocketseat |
| Video and audio | YouTube, Instagram, TikTok, Twitter/X, Reddit, Twitch, Pinterest, Vimeo, Bluesky, Bilibili |
| Bilibili (deep) | Sign in for 4K, HDR, Dolby Vision, Hi-Res lossless, Dolby Atmos. Danmaku (XML/ASS/JSON), NFO for Kodi and Jellyfin, 11 URL types (UGC, 番剧, 课程, 收藏夹, UP主, 每周必看, 稍后再看, 历史记录, b23.tv) |
| Asian platforms | Douyin (抖音), Xiaohongshu (小红书), Kuaishou (快手), Youku (优酷), iQiyi (爱奇艺), Tencent Video, Mango TV |
| Image galleries | DeviantArt, Pixiv, ArtStation, Flickr, Tumblr, Imgur albums, Kemono, Newgrounds, image boards |
| Bulk profiles | Whole subreddits and their sort pages, Reddit user profiles, and X/Twitter profiles |
| Files and transfer | `.torrent` and magnet links, plus direct P2P transfer between two computers with a short code |

Things people search for, and OmniGet does:

- **Download a full online course**, every lesson and attached PDF, then watch it inside the app and resume where you stopped.
- **Download a YouTube video or whole playlist**, pick the quality, or grab audio only as MP3, M4A, Opus, FLAC, or WAV.
- **Download TikTok, Instagram, Twitter/X, Reddit** posts, reels, stories, carousels, and galleries.
- **Batch download** a list of links from a text file, an entire subreddit, a Reddit user profile, or an X/Twitter profile, all in one go.
- **Download only part of a video** by setting a start and end time.
- **Download subtitles** in any language, embed them, or generate them with Whisper when none exist.
- **Skip sponsors** with SponsorBlock, and auto embed metadata and thumbnails.
- **Follow a channel** and auto download new uploads, with a tray notification.
- **Download Bilibili at maximum quality**, sign in once and unlock 4K, HDR, Hi-Res lossless audio and Dolby Atmos.

Downloads are reliable, not a guessing game. Speed and ETA come straight from the downloader instead of being faked from a percentage, so they stay correct even when the file size is unknown or the stream is live. A stall is shown as a stall, not a frozen "3 seconds left". The queue resumes interrupted downloads and retries with backoff.

---

## It also plays everything inside

This is the part people do not expect. OmniGet is not just where you download. It is where you watch, read, and listen.

### Open a course and actually watch it

Download the whole course (Hotmart, Udemy, Kiwify, Skool, Teachable, Kajabi, Wondrium, Thinkific) and watch it without leaving the app. Resume at the second you stopped. Take notes that jump to that moment when you click them. Read the attached PDFs side by side.

<p align="center">
  <img src="assets/screenshot-courses.png" alt="OmniGet course player with timestamped notes and PDF attachments" width="760" />
  <br/>
  <em>Course player, notes pinned to timestamps, attachments in the same window.</em>
</p>

### Read books, real ones

Drop a folder of PDFs and EPUBs. OmniGet pulls covers from them, fetches titles and authors, and opens each one in a built-in reader with highlights, bookmarks, a focus mode, and a paper feel theme for the eyes. CBZ comics and TXT or HTML too.

<p align="center">
  <img src="assets/screenshot-reader.png" alt="OmniGet built-in EPUB and PDF reader with highlights and focus mode" width="760" />
  <br/>
  <em>Reader with highlights, notes panel, and focus mode.</em>
</p>

### Music, the way you remember it

Point OmniGet at your music folder and it shows your tracks the way iTunes used to: albums with covers, artists with discographies, a queue that behaves.

- Plays MP3, FLAC, M4A, OGG, Opus, anything you already have.
- Pulls **synced lyrics** so they scroll along with the song.
- Connects to **Spotify, SoundCloud, YouTube Music, Qobuz, and Last.fm**, so your playlists and likes sit next to your local files.
- **Equalizer** with presets, dark theme variants per album cover, an activity dashboard with your top tracks, and a Discord presence that shows what you are playing.

<p align="center">
  <img src="assets/screenshot-music.png" alt="OmniGet music player with album view, synced lyrics and streaming sources" width="820" />
  <br/>
  <em>Local library, synced lyrics, streaming sources, one player.</em>
</p>

---

## Settings that stay out of your way

Settings are grouped and quiet. Common choices are right there, the deep options live one tap away, and a search box finds anything across every category and highlights it for you.

<p align="center">
  <img src="assets/readme/en/settings-drill.png" alt="OmniGet settings with a grouped sidebar and clean drill down sections" width="820" />
  <br/>
  <em>Grouped sidebar, one clear list, each section opens its own page.</em>
</p>

<p align="center">
  <img src="assets/readme/en/settings-output.png" alt="OmniGet download settings, output folder, organize by platform, filename template, skip existing files" width="820" />
  <br/>
  <em>Output, quality, subtitles, and the rest, with a short hint under every control.</em>
</p>

---

## For League of Legends players, if you want it

OmniGet has a League of Legends menu built in. It ships **off**. Nothing connects, nothing watches, and the menu does not even appear in the sidebar until you switch it on in **Settings → Advanced → League of Legends**. Leave it off and OmniGet behaves exactly as it always did.

Turn it on and it reads your running League client the same way the client reads itself, with no account, no login, and no third-party build site in the loop.

- **Match scouting** for both teams. Rank, recent form, KDA, the champions each player actually plays, and short notes it works out on its own: hot streak, one-trick, low win rate. Write your own note on a player and it comes back the next time you meet them.
- **Win probability** that does the statistics properly. Win rates are shrunk toward the baseline by sample size, so fifty percent over a thousand games and fifty percent over ten are not treated as the same evidence, and the result always carries a range instead of a fake decimal. Matchmaking aims for even games, so the honest answer usually sits near even, and the app says so rather than pretending otherwise.
- **Live economy** while you play. Gold, CS and level for all ten players, and the gap against the opponent in your own lane.
- **Goals per role** you can edit. A support is not judged on CS and a jungler is not judged like a marksman.
- **Runes and summoner spells** recommended by the game client itself, applied in one click. It only ever replaces the page OmniGet created, never yours.
- **Champion tiers** by role with win, pick and ban rates.
- **Player search** for any Riot ID, with rank, champion record and mastery.
- **Automation**, all opt-in: accept matches, pick and ban from your own priority list, and grab a champion off the ARAM bench.

<p align="center">
  <em>Everything above is off by default and every automation has its own switch.</em>
</p>

---

## Plugins that install themselves

OmniGet ships with its full set of plugins (courses, study, Telegram, convert, and more) and they set themselves up on first launch. They also update on their own when a new version is released, so you never chase a download. Turn any of them on or off from the sidebar, and uninstall the ones you do not want. What you remove stays removed.

<p align="center">
  <img src="assets/readme/en/plugins.png" alt="OmniGet plugins and dependencies, browser extension pairing and managed tools as a table" width="820" />
  <br/>
  <em>Plugins and bundled tools, managed for you, shown as a clear table.</em>
</p>

---

## The small things that add up

Quietly there when you need them.

- **Subtitle Workshop** that opens SRT, VTT, and ASS, with timing tools, two point sync, find and replace, a one click auto fix, AI translate and AI grammar fix, and a waveform with shot change markers.
- **Pomodoro focus timer** that pauses your video when the session ends.
- **Notes app** with bidirectional links, a daily journal, and a knowledge graph.
- **Progress dashboard** with a streak counter, daily goals, and a year style heatmap.
- **FFmpeg converter** for local files, no internet required.
- **Telegram chat browser** that lets you save photos, videos, and files from any chat.
- **Browser extension** (Chrome and Firefox) that hands the current page to OmniGet with one click.
- **Global hotkey** (`Ctrl+Shift+D`, or `Cmd+Shift+D` on macOS) that downloads whatever URL is in your clipboard.
- **9 languages** and **14 themes**, including Catppuccin, Dracula, One Dark Pro, and three e-ink variants.

---

## Frequently asked questions

**Is OmniGet free?**
Yes. OmniGet is free and open source under GPL-3.0, with no account, no ads, and no paid tier.

**Do I need an account to use OmniGet?**
No. OmniGet needs no account and no sign-up of its own. You only log in to a platform when the content itself requires it, such as a paid course you bought or a Bilibili premium stream, and that session stays on your computer.

**Can OmniGet download a YouTube playlist or an entire channel?**
Yes. Paste a playlist URL and OmniGet queues every video in it. You can also follow a channel, and OmniGet downloads new uploads automatically and shows a tray notification.

**Does OmniGet resume an interrupted download?**
Yes. The queue keeps partially downloaded files and continues from where it stopped instead of starting over, and it retries with backoff when a site rate-limits you. Closing the app or losing your connection does not throw away progress.

**What file formats can OmniGet save?**
Video as MP4, MKV, or WebM, and audio as MP3, M4A, Opus, FLAC, or WAV. Subtitles save as SRT, VTT, or ASS, and can be embedded into the video file. Books open as PDF, EPUB, CBZ, TXT, and HTML.

**Can OmniGet download paid content I already bought?**
OmniGet downloads what your own logged-in session can already open, such as a Udemy or Hotmart course you paid for. It does not bypass DRM, break paywalls, or share credentials. Content you have no access to stays inaccessible.

**Do I need the terminal or Python to use OmniGet?**
No. OmniGet is a normal desktop app. Download it, open it, paste a link. yt-dlp and FFmpeg are bundled and update themselves. The only time you may touch Terminal is the one time macOS first launch step above.

**OmniGet will not open on macOS, what do I do?**
Run the two Terminal commands in the [first launch note](#️-please-read-this-before-the-first-launch). Gatekeeper blocks unsigned open source apps, and those lines clear the flag. You do it once.

**Is OmniGet just a yt-dlp GUI?**
OmniGet uses yt-dlp under the hood for the 1,800+ generic sites, with native extractors for the big platforms, plus a real interface, a queue, a library, and built-in players on top. So yes, and a lot more than a GUI.

**Can OmniGet download a full Udemy or Hotmart course?**
Yes. You log in once on the platform, pick the course, and OmniGet downloads every lesson and attachment, then plays them back with timestamped notes.

**Which sites does OmniGet support?**
Online courses, YouTube, TikTok, Instagram, Twitter/X, Reddit, Twitch, Vimeo, Bilibili, Pinterest, Bluesky, major Asian platforms, image galleries, torrents and magnets, plus around 1,800 more through yt-dlp.

**Does OmniGet work on Windows, macOS, and Linux?**
Yes, all three, on both x86_64 and ARM64. Windows ships as a portable `.exe`, an `.msi` installer, and a winget package. macOS ships as a `.dmg` for Apple Silicon and Intel. Linux ships as `.deb`, `.rpm`, and `.AppImage`.

**Which Linux distributions are supported?**
OmniGet runs on any modern desktop Linux. Debian and Ubuntu users should take the `.deb`, Fedora and openSUSE the `.rpm`, and anything else the `.AppImage`. On Debian 12+ and Ubuntu 24.04+ the AppImage needs `sudo apt install libfuse2`, because those releases dropped FUSE 2; the `.deb` has no such requirement.

**Can OmniGet run fully portable from a USB stick?**
Yes. Create an empty file named `portable.txt` (or `.portable`) next to the `.exe` and relaunch. OmniGet then stores settings, the database, cookies, plugins, caches, and the bundled yt-dlp/FFmpeg in a `data` folder beside the executable — nothing is written to `AppData\Roaming` or other user folders. Without that file, the app uses the standard per-user data directory.

**Can OmniGet download audio only, or just a clip?**
Yes. OmniGet extracts audio as MP3, M4A, Opus, FLAC, or WAV, or set a start and end time to download only the part you need.

**Are my OmniGet downloads private?**
Yes. Everything in OmniGet runs locally and your files never leave your computer. There is no telemetry on what you download.

**Can OmniGet download Bilibili in 4K, HDR, or Hi-Res lossless?**
Yes, with a Bilibili account signed in. OmniGet talks to the official Bilibili API and respects exactly what your 大会员 (premium) subscription unlocks. Without signing in, downloads still work through yt-dlp at standard quality.

---

## Build from source

For developers. If you just want to use OmniGet, [grab a release](#download-and-install).

```bash
git clone https://github.com/tonhowtf/omniget.git
cd omniget
pnpm install
pnpm tauri dev
```

Requires [Rust](https://rustup.rs/), [Node.js](https://nodejs.org/) 18+, and [pnpm](https://pnpm.io/).

<details>
<summary>Linux build dependencies</summary>

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

</details>

Production build: `pnpm tauri build`.

### Command line interface (build it yourself)

The repository also contains `omniget-cli`, a small Rust binary that makes OmniGet scriptable. **It is not included in the release downloads yet** — you build it from this repository:

```bash
cargo build --release -p omniget-cli
```

```bash
omniget info <url>                     # preview title, formats and size, download nothing
omniget download <url> -q 1080 -o ~/Videos
omniget download <url> --audio-only --subs en,pt
omniget batch links.txt -m 3           # one URL per line, 3 at a time
omniget import-cookies cookies.txt     # Netscape format
```

The desktop app never needs this. It exists for cron jobs, dotfiles, and scripts.

---

## Contribute

**Community.** Questions, help and release chatter happen on [Discord](https://discord.gg/jgdxyPy7Vn).

Found a bug or have a feature idea? [Open an issue](https://github.com/tonhowtf/omniget/issues). Pull requests are welcome, see [CONTRIBUTING.md](CONTRIBUTING.md).

OmniGet is translated on [Weblate](https://hosted.weblate.org/engage/omniget/). Pick a language, translate in your browser, and Weblate opens a pull request automatically.

### Developing plugins

OmniGet's Courses, Telegram, and Convert features are all plugins — Rust dynamic libraries built on [`omniget-plugin-sdk`](src-tauri/omniget-plugin-sdk) — and third-party plugins are welcome. The [Plugin Development Guide](docs/plugin-development.md) covers the architecture, a quick start from the [plugin template](https://github.com/tonhowtf/omniget-plugin-template), the manifest and host API, honest notes on ABI stability, and how to get listed in the [plugin registry](https://github.com/tonhowtf/omniget-plugins).

## Notice to platform owners

If you represent a listed platform and have concerns, email **tonhowtf@gmail.com** from a company address. The platform comes off the list right away.

## Legal

OmniGet is meant for personal use. Respect copyright and each platform's terms of service. You are responsible for what you download.

## License

[GPL-3.0](LICENSE). The OmniGet name, logo, and Loop mascot are project trademarks not covered by the code license.
