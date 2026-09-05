<!--
Search keywords (kept here so GitHub search, Google and AI assistants can find the project):
OmniGet is a free open source downloader and media toolbox for Windows, macOS and Linux.
udemy downloader, hotmart downloader, kiwify downloader, course downloader, youtube downloader,
yt-dlp gui, instagram downloader, instagram story downloader, reels downloader, twitter downloader,
x video downloader, pinterest downloader, pinterest board backup, tiktok downloader, reddit downloader,
twitch vod downloader, bilibili downloader, telegram downloader, torrent client, magnet downloader,
subtitle downloader, whisper transcription, text to speech, epub reader, pdf reader, anki flashcards,
spaced repetition, music player, spicetify, media downloader, download manager, tauri, rust, svelte.

GitHub allows 20 topics. The repository uses exactly these 20:
downloader, download-manager, media-downloader, video-downloader, youtube-downloader, yt-dlp,
yt-dlp-gui, course-downloader, udemy-downloader, hotmart-downloader, bilibili-downloader,
tiktok-downloader, instagram-downloader, twitter-downloader, reddit-downloader, telegram-downloader,
twitch-downloader, subtitle-downloader, epub-reader, spaced-repetition
-->

<p align="center">
  <img src="assets/readme/hero.svg" alt="OmniGet: paste a link, get the file. Downloads, tools and a study library in one desktop app for Windows, macOS and Linux." width="100%" />
</p>

<h1 align="center">OmniGet</h1>

<p align="center">
  <b>English</b>
  · <a href="README_pt_br.md">Português (BR)</a>
  · <a href="README.ru.md">Русский</a>
  · <a href="README_zh_CN.md">简体中文</a>
</p>

<p align="center">
  <b>Download Udemy and Hotmart courses, YouTube, Instagram, X, Pinterest, TikTok and 1,800+ other sites.<br/>Then transcribe, convert, read and study what you saved. One free desktop app, no terminal.</b>
</p>

<p align="center">
  <a href="https://github.com/tonhowtf/omniget/releases/latest"><img src="https://img.shields.io/github/v/release/tonhowtf/omniget?style=for-the-badge&label=release&color=F28500" alt="Latest release" /></a>
  <a href="https://github.com/tonhowtf/omniget/releases"><img src="https://img.shields.io/github/downloads/tonhowtf/omniget/total?style=for-the-badge&label=downloads&color=1E6FE8" alt="Total downloads" /></a>
  <a href="https://github.com/tonhowtf/omniget/stargazers"><img src="https://img.shields.io/github/stars/tonhowtf/omniget?style=for-the-badge&color=FFD426" alt="GitHub stars" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-2AA845?style=for-the-badge" alt="License GPL-3.0" /></a>
  <a href="https://discord.gg/jgdxyPy7Vn"><img src="https://img.shields.io/badge/Discord-community-5865F2?style=for-the-badge&logo=discord&logoColor=white" alt="Discord community" /></a>
  <a href="https://hosted.weblate.org/engage/omniget/"><img src="https://hosted.weblate.org/widget/omniget/frontend-json/svg-badge.svg" alt="Translation status" /></a>
</p>

<p align="center">
  <a href="#download-and-install"><img src="https://img.shields.io/badge/Download_for_Windows,_macOS_or_Linux-→-F28500?style=for-the-badge" alt="Download OmniGet" height="40" /></a>
  &nbsp;
  <a href="#the-tools-section-108-tools-in-16-categories"><img src="https://img.shields.io/badge/See_the_108_tools-→-3D5BF0?style=for-the-badge" alt="See the Tools section" height="40" /></a>
</p>

<p align="center">
  <sub>Free. Open source under GPL-3.0. No account, no ads, no telemetry on what you download. Your files stay on your computer.</sub><br/>
  <sub>9,500+ GitHub stars. The most starred repository in the <a href="https://github.com/topics/udemy-downloader">udemy-downloader</a>, <a href="https://github.com/topics/hotmart-downloader">hotmart-downloader</a> and <a href="https://github.com/topics/course-downloader">course-downloader</a> topics.</sub>
</p>

<p align="center">
  <img src="assets/readme/home.png" alt="OmniGet home screen: paste a URL, magnet link or .torrent and the file lands in your folder" width="900" />
</p>

---

## Contents

- [Why OmniGet](#why-omniget)
- [Download and install](#download-and-install)
- [Your first download in one minute](#your-first-download-in-one-minute)
- [What OmniGet downloads](#what-omniget-downloads)
- [The browser extension, step by step](#the-browser-extension-step-by-step)
- [The Tools section: 108 tools in 16 categories](#the-tools-section-108-tools-in-16-categories)
- [Plugins: Courses, Study, Telegram, Convert](#plugins-courses-study-telegram-convert)
- [Built-in chat, off by default](#built-in-chat-off-by-default)
- [For League of Legends players](#for-league-of-legends-players)
- [Everything else in the box](#everything-else-in-the-box)
- [Privacy and what OmniGet refuses to do](#privacy-and-what-omniget-refuses-to-do)
- [Frequently asked questions](#frequently-asked-questions)
- [Command line](#command-line)
- [Build from source](#build-from-source)
- [Contributing and translations](#contributing-and-translations)

---

## Why OmniGet

You bought a course and want it on your disk before the platform pulls it. You keep a yt-dlp cheat sheet because the flags never stick. You have one site for Instagram stories, another for X videos, a Chrome extension for Pinterest, a Python script for subtitles, and none of them remember your login.

OmniGet puts all of that behind one text box. Paste a link, see a preview with quality options, click download. The same window then plays the course, reads the PDF, transcribes the audio and backs up the Pinterest board. yt-dlp and FFmpeg install themselves and stay updated, so there is nothing to configure and no terminal to open.

<p align="center">
  <img src="assets/readme/workflow.svg" alt="How OmniGet works: paste a link or press the hotkey, OmniGet detects the site and fetches with yt-dlp or a native extractor, the file lands in your folder and opens in the built-in player, reader or tools." width="100%" />
</p>

### How it compares

| | OmniGet | yt-dlp alone | Single-site web downloaders | Paid course downloaders |
|---|---|---|---|---|
| Sites | Courses, Instagram, X, Pinterest, Bilibili, Telegram, torrents natively, plus 1,800+ through yt-dlp | 1,800+ | One | One or two platforms |
| Setup | Download one file, open it | Python, PATH, FFmpeg, flags | None | Installer, license key |
| Logged-in content | Cookies from your browser through the extension | Manual `--cookies` export | Rarely | Sometimes |
| Queue | Resume, retry with backoff, rules, followed channels | One command at a time | No | Varies |
| After the download | Player, reader, flashcards, notes, 108 tools | Files | Files, often re-encoded | Files |
| Price and license | Free, GPL-3.0 | Free, Unlicense | Free with ads | Subscription |

yt-dlp is the engine OmniGet runs on, and OmniGet would not exist without it. If you live in a terminal and only want files, yt-dlp alone is the right tool.

---

## Download and install

Pick your system. Every build is published on the [Releases page](https://github.com/tonhowtf/omniget/releases/latest). Updates arrive inside the app.

<table>
  <tr>
    <th align="left">System</th>
    <th align="left">What to download</th>
    <th align="left">Other ways</th>
  </tr>
  <tr>
    <td><b>Windows 10 / 11</b></td>
    <td><code>omniget_x.y.z_x64-setup.exe</code> (installer)<br/><code>omniget_x.y.z_x64-portable.exe</code> (no install, runs from anywhere)<br/><code>omniget_x.y.z_x64_en-US.msi</code> (for IT deployments)</td>
    <td><code>winget install -e --id tonhowtf.OmniGet</code></td>
  </tr>
  <tr>
    <td><b>macOS 10.15+</b></td>
    <td><code>omniget_x.y.z_aarch64.dmg</code> for Apple Silicon (M1 and later)<br/><code>omniget_x.y.z_x64.dmg</code> for Intel Macs</td>
    <td><code>brew install --cask tonhowtf/tap/omniget</code></td>
  </tr>
  <tr>
    <td><b>Linux</b></td>
    <td><code>.deb</code> for Debian and Ubuntu (amd64 and arm64)<br/><code>.rpm</code> for Fedora, openSUSE and RHEL family (x86_64 and aarch64)<br/><code>.AppImage</code> for everything else (amd64 and aarch64)</td>
    <td>AppImage self-updates through the <code>.zsync</code> files</td>
  </tr>
</table>

### The first launch warning, and how to clear it

OmniGet is not signed with a paid certificate, so each system shows a warning the first time. This is normal for open source desktop apps and you handle it once.

**Windows.** SmartScreen shows a blue box. Click **More info**, then **Run anyway**.

**macOS.** Gatekeeper refuses to open the app and may say it is "damaged". After you drag OmniGet into Applications, open Terminal (Spotlight, type "Terminal") and paste these two lines:

```bash
xattr -cr /Applications/omniget.app
codesign --force --deep --sign - /Applications/omniget.app
```

Then open OmniGet from Launchpad as usual.

**Linux, AppImage on Debian 12+ or Ubuntu 24.04+.** Those releases ship without FUSE 2, which AppImage needs. If the file fails with a libfuse error, run `sudo apt install libfuse2`, or launch it with `./omniget.AppImage --appimage-extract-and-run`. The `.deb` avoids this entirely.

### Portable mode

Create an empty file named `portable.txt` (or `.portable`) next to the Windows `.exe` and relaunch. Settings, the database, cookies, plugins, caches, yt-dlp and FFmpeg all move to a `data` folder next to the executable. Nothing touches `AppData`, so the whole install fits on a USB stick.

---

## Your first download in one minute

1. Open OmniGet. The setup screen asks for your language and theme, then installs yt-dlp and FFmpeg with one click. yt-dlp is checked against its SHA-256 before it runs.
2. Copy any link: a YouTube video, an Instagram reel, an X post, a Pinterest board, a magnet, a direct file URL.
3. Paste it in the box on the home screen. OmniGet detects the site and shows the title, thumbnail and available qualities. Pick one and press Enter.

The Downloads page shows speed, phase and ETA read straight from the downloader, so a stalled download looks stalled instead of frozen at "3 seconds left". Interrupted downloads resume where they stopped. Rate-limited sites get retried with backoff, and connections per site adapt on their own, so YouTube gets up to 16 parallel fragments while a site that answers 429 gets fewer. When a Python 3.10 or newer is present, yt-dlp runs as a zipapp on it and starts in under a second instead of unpacking the bundled binary on every launch.

<p align="center">
  <img src="assets/readme/downloads.png" alt="OmniGet Downloads page with an active 4K YouTube download showing phase, speed, ETA and the exact yt-dlp command, plus queued and finished items" width="900" />
</p>

### Skip the window entirely

Copy a link anywhere on your system and press **Ctrl+Shift+D** (**Cmd+Shift+D** on macOS). OmniGet reads the clipboard and starts the download in the background. A second hotkey, **Ctrl+Shift+M**, grabs audio only, so a YouTube link becomes an MP3 without opening anything. It is off until you enable it, and both shortcuts can be rebound in **Settings → Downloads → Clipboard & hotkeys**.

---

## What OmniGet downloads

OmniGet has native extractors for the platforms people use most, and hands everything else to [yt-dlp](https://github.com/yt-dlp/yt-dlp), which covers roughly 1,800 sites.

| Category | Sites and formats |
|---|---|
| Online courses | Hotmart, Udemy, Kiwify, Rocketseat and Meta-Analysis Academy through the Courses plugin. Every lesson, section selection, attachments, resume where you stopped. |
| Video and audio | YouTube (videos, playlists, channels, live from start, chapters, SponsorBlock), Instagram, TikTok, X/Twitter, Reddit, Twitch (VODs, clips, live), Vimeo, Bluesky, Threads, Pinterest, Douyin |
| Bilibili, signed in | 4K, HDR, Dolby Vision, Hi-Res lossless and Dolby Atmos according to your subscription. Danmaku comments as XML, ASS or JSON, NFO files for Kodi and Jellyfin, custom naming templates, 11 URL types including bangumi, courses, favorites, watch later and history |
| Image galleries | Whole galleries and profiles from 250+ sites through gallery-dl (DeviantArt, Pixiv, ArtStation, Flickr, Tumblr, Imgur, Kemono and more) |
| Bulk | Paste many links or load a `.txt`, download whole subreddits, Reddit and X profiles, Instagram and Pinterest profiles |
| Files and transfer | `.torrent` files and magnet links with a built-in BitTorrent client, direct HTTP files, HLS and DASH manifests, and person-to-person transfer between two OmniGet installs with a short word code |
| Telegram | Photos, videos, files and audio from any channel or group you belong to, through the Telegram plugin |

Options you set once and forget: default quality, audio-only format (MP3, M4A, Opus, FLAC or WAV), subtitle languages and format (SRT, VTT, ASS, embedded or sidecar), thumbnail and metadata embedding, filename template, organize by platform, skip existing files, split by chapters, speed limit, concurrent downloads, proxy. Rules send a given channel or host to a folder and quality of your choice without asking again. Followed channels are checked in the background and can download new uploads automatically with a tray notification.

<p align="center">
  <img src="assets/readme/settings.png" alt="OmniGet settings: grouped sidebar with Appearance, Downloads, Network, Cookies, Channels, AI, Plugins and Advanced, and a download section with Output, Quality, Subtitles, Clipboard and hotkeys, Per-platform options" width="900" />
</p>

---

## The browser extension, step by step

The extension does two jobs. On sites it recognizes (YouTube, Instagram, TikTok, X, Reddit, Twitch, Pinterest, Bluesky, Telegram, Vimeo, Udemy, Hotmart, Rocketseat, Bilibili, SoundCloud) it sends the page to OmniGet with one click or with **Alt+O**. On any other site it watches network traffic for MP4, HLS, DASH, WebM and audio streams and lists them in its popup. In both cases it forwards your cookies and referer, which is what lets OmniGet download private content you are logged into, such as Instagram stories, a paid course, or a members-only video. Cookies are grouped by real site, so a Brazilian `.com.br` domain gets its own entry instead of sharing one with every other `.com.br` site. The popup also has a **Force H.264** switch for YouTube, for computers that stutter on VP9 and AV1.

Pick the level that matches how comfortable you are.

<p align="center">
  <img src="assets/readme/extension.svg" alt="Pairing flow: install the extension in Chrome, click Pair extension in OmniGet Settings, the extension finds the app on localhost and connects. From then on every download carries your cookies." width="100%" />
</p>

### Level 1: from inside the app (no downloads, no zip files)

1. Open OmniGet and install it if you have not. Launch it once.
2. Go to **Settings → Plugins → Browser extension**. Click **Update / Install** next to Chrome. OmniGet copies the extension that ships inside it to a folder and opens that folder for you.
3. Open Chrome (Edge, Brave and other Chromium browsers work the same way) and type `chrome://extensions` in the address bar.
4. Turn on **Developer mode** with the switch in the top right corner.
5. Click **Load unpacked** and pick the folder OmniGet just opened.
6. The OmniGet icon appears in the toolbar. An options page opens on its own and says it is looking for the app.
7. Back in OmniGet, still in **Settings → Plugins → Browser extension**, click **Pair extension**. Within a few seconds the app says "Extension connected" and the options page turns green. Done.

From now on, visit any supported page and click the icon. The page, its cookies and its title go to OmniGet and the download starts. Your cookies also show up in **Settings → Cookies**, where the Courses plugin and the Instagram, X and Pinterest tools reuse them.

### Level 2: from the release zip

Every release ships `omniget-chrome-extension-vX.Y.Z.zip`. Download it from the [latest release](https://github.com/tonhowtf/omniget/releases/latest), unzip it, then follow steps 3 to 7 above pointing **Load unpacked** at the unzipped folder. Use this if you keep the app on one machine and the browser on another, or if you are installing for someone else.

### Level 3: Firefox, other browsers and manual pairing

Firefox: **Settings → Plugins → Browser extension → Update / Install** next to Firefox, then open `about:debugging#/runtime/this-firefox`, click **Load Temporary Add-on** and pick `manifest.json` in the exported folder. Firefox drops temporary add-ons on restart, so repeat this until the extension is published on AMO. Safari is not supported yet because Safari extensions must ship through the App Store.

Manual pairing: if **Pair extension** times out, open the extension's options page (right-click the icon → Options), then in OmniGet reveal and copy the **Pairing token** and paste it into the options page. The endpoint URL is detected automatically. The app listens on `127.0.0.1` ports 47720 to 47729 and the token is generated per install, so nothing leaves your machine.

If the extension is installed but OmniGet is closed, clicks fall back to the `omniget://` link scheme, which still queues the URL but cannot carry cookies. Tick "Always allow" the first time Chrome asks.

---

## The Tools section: 108 tools in 16 categories

Tools is the part of OmniGet that grew beyond downloading. Each tile is one job: an isolated Rust command with JSON in and JSON out, which is also what lets AI agents drive them through the built-in MCP server. The hub has a search box that understands English and Portuguese ("legenda" finds subtitle tools) and a platform filter, and tools that only run on Windows say so on the tile and stay hidden elsewhere.

<p align="center">
  <img src="assets/readme/tools.png" alt="OmniGet Tools hub with 16 categories: YouTube, Speech and subtitles, Video editing, Instagram, X, Pinterest, Spotify, PDF, Documents, Images, System, Files, Downloads, Automation, Phone and AI" width="900" />
</p>

Status legend: no mark means ready, **beta** means it works but has not been tested against every account type, **planned** means the tile exists so you can see where things are going and does nothing yet.

<table>
  <tr>
    <td><img src="assets/readme/tools-instagram.png" alt="Instagram tools in OmniGet: download post, bulk download, reel audio, stories, highlights, story viewers, profile viewer, HD avatar, profile download, unfollowers, fans, mutuals, who unfollowed, ghost followers, whitelist, data export, analytics, compare profiles, hashtag explorer, comments, likers, giveaway picker, publish and schedule" /></td>
    <td><img src="assets/readme/tools-x.png" alt="X / Twitter tools in OmniGet: download post, unroll thread, post to image, profile X-ray, profile media, advanced search, export bookmarks, who doesn't follow back, your X archive and Grok" /></td>
  </tr>
  <tr>
    <td><img src="assets/readme/tools-pinterest.png" alt="Pinterest tools in OmniGet: download pin, board backup, profile backup, search without AI or ads, similar pins, find the source, duplicates, color palette, offline gallery and keyword ideas" /></td>
    <td><img src="assets/readme/tools-speech.png" alt="Speech and subtitles tools in OmniGet: transcribe with whisper.cpp, text to speech, translate subtitles, dub from subtitles, and planned voice cloning, voice design, vocal isolation and dictation" /></td>
  </tr>
</table>

### YouTube (11)

- **Download video.** Paste a link and pick quality, format and subtitles. Same engine as the home screen.
- **Metadata.** Save the info, description and thumbnail without the video.
- **Thumbnails.** Browse every cover image and save it at any resolution.
- **Subtitles.** Download subtitles, or merge two languages into one bilingual file.
- **Comments and chapters.** Fetch comments or chapter markers, filter them, export JSON or CSV.
- **Live chat.** Save the chat replay of a stream as JSON or CSV.
- **Subtitle workshop.** Edit, translate and re-time SRT, VTT and ASS files with a waveform, two-point sync, find and replace, an auto fix, and AI grammar and translation.
- **SponsorBlock.** See sponsor, intro and outro segments and get the yt-dlp flags to skip them.
- **Dislikes.** Likes, dislikes and rating from Return YouTube Dislike.
- **Real thumbnail.** The frames the CDN already has at 25, 50 and 75 percent, instead of the clickbait cover.
- **Force H.264.** A switch in the browser extension that keeps YouTube on H.264 instead of VP9 and AV1, for machines that stutter on newer codecs.

### Speech and subtitles (8)

- **Transcribe.** Audio or video to subtitles with whisper.cpp, offline. Models download on demand, Metal acceleration on macOS.
- **Text to speech.** Natural voices from Microsoft Edge, free, with a synced subtitle file.
- **Translate subtitles.** Translate an SRT with your AI provider or a LibreTranslate server, keeping the timing.
- **Dub from subtitles.** Turn an SRT into a voice track that fits each line and optionally replace the video's audio. *beta*
- **Clone a voice**, **Design a voice** and **Isolate vocals** through a VoiceStudio install running on your machine. *beta*
- **Dictation.** Press a global shortcut, speak, and whisper types the text where your cursor is. *beta*

### Video editing (6)

- **Cut a clip.** Pick a video on disk and cut out a section. The result lands in the downloads queue.
- **Convert.** Change container, codec or resolution, or compress, through the Convert plugin.
- **Auto captions** and **Text to speech** open the speech tools above.
- **Record screen.** Screen and system audio through FFmpeg, with a replay buffer that saves what just happened. *beta*
- **Timeline editor.** *planned*

### Instagram (24)

All of these run on your own Instagram session captured by the browser extension, so stories, close friends and your own lists work. Reads are paced and write actions stop on the first sign of a rate limit.

- **Download post.** Photo, video, reel, IGTV or carousel from a link, best quality.
- **Download many links.** Paste a list or a `.txt` and get everything.
- **Reel audio.** Keep only the sound, as M4A or MP3.
- **Stories.** Download stories, including close friends, without marking them as seen.
- **Highlights.** One highlight or every highlight of a profile.
- **Who viewed my story.** List and export viewers of each active story.
- **Profile viewer.** Bio, counts, HD photo and whether the account follows you.
- **Profile picture in HD.**
- **Download a profile.** All posts, reels, tagged or saved posts, with a limit you choose.
- **Who doesn't follow back.** Compare followers and following, protect accounts with a whitelist, unfollow at a safe pace.
- **Fans.** Accounts that follow you but you don't follow back, with the option to remove them.
- **Mutuals.**
- **Who unfollowed me.** Snapshots of your lists over time show who left and who arrived.
- **Ghost followers.** Followers who never like or comment, and the ones who engage the most.
- **Whitelist.** Accounts never suggested for unfollowing.
- **Data export.** Read Meta's "Download your information" zip offline: pending requests, close friends, blocked and more.
- **Profile analytics.** Engagement rate, cadence, best days and hours, hashtags and top posts of any public profile.
- **Compare profiles.** Up to six profiles side by side.
- **Hashtag explorer.** Post count, recent and top posts, related hashtags, download.
- **Export comments.** All comments of a post as CSV, with filter.
- **Who liked.** List and export the accounts that liked a post.
- **Giveaway picker.** Draw winners among the comments with rules for mentions, keyword and one entry per person.
- **Publish.** Photo, carousel, reel, video or story through your session or the official Graph API. *beta*
- **Schedule posts.** Queue posts for a date and time. OmniGet publishes them while it is open. *beta*

### X / Twitter (10)

Public data comes through the FxTwitter API without login. Anything private (bookmarks, your follows, Grok on X) uses your X session from the cookie manager.

- **Download post.** Videos, images and GIFs from any post.
- **Unroll thread.** The whole thread on one page, exported as Markdown, HTML or text.
- **Post to image.** A clean PNG card of a post for sharing anywhere.
- **Profile X-ray.** Engagement, best time to post, top posts and hashtags of any account.
- **Profile media.** Every photo and video from a profile, original quality, in one go.
- **Advanced search.** Build queries with X operators, see trends, export results.
- **Export bookmarks.** All bookmarks with folders, to JSON, CSV, Markdown or HTML. *beta*
- **Who doesn't follow back.** Audit following vs. followers and unfollow safely with a whitelist. *beta*
- **Your X archive.** Open the data zip offline: stats, top posts, likes and follow lists.
- **Grok.** Ask Grok with live X search or summarize a thread, through the xAI API or your X session. *beta*

### Pinterest (10)

Works without login for anything public. Cookies are only needed for secret boards and for unsaving.

- **Download pin.** Image in original quality, video as MP4, GIF, carousel or story pages.
- **Board backup.** Every pin of a board or section with originals, videos, CSV/JSON and incremental sync.
- **Profile backup.** All public boards of a profile, one folder per board, plus created pins.
- **Search without AI or ads.** Filters that hide AI images, promoted pins and videos, then download.
- **Similar pins.** The "More like this" of any pin, filterable and downloadable.
- **Find the source.** Destination link, creator, dead-link check, Wayback Machine and reverse image search.
- **Duplicates in a board.** Identical and near-identical pins, with optional unsave.
- **Color palette.** Palette of a board or pin as hex, CSS or JSON.
- **Offline gallery, PDF, CSV.** A board as a searchable HTML gallery, a PDF moodboard or a spreadsheet.
- **Keyword ideas.** Search suggestions, refinements and the words top pins use.

### Spotify (2)

- **Themes and colors.** Customize the Spotify client with Spicetify themes. *beta*
- **Extensions.** Install Spicetify extensions and custom apps from its Marketplace. *beta*

### PDF (6)

- **Merge.** Join several PDFs into one, in the order you choose.
- **Split.** Extract pages or break a PDF into parts.
- **Compress.** Shrink a PDF while keeping it readable.
- **Convert.** PDF to images or Word, and back.
- **OCR.** Make scanned PDFs searchable. *beta*
- **Safe PDF.** Rebuild a PDF from pixels to strip scripts and forms.

### Documents (5)

- **SlideShare to PDF.** Every slide at the largest size, assembled into one PDF.
- **Google Docs export.** Public Docs, Slides and Sheets as PDF, DOCX, PPTX or XLSX.
- **Calameo pages.** Save the pages of a Calameo publication as SVG or JPG. *beta*
- **Image galleries.** Whole galleries and profiles from 250+ sites with gallery-dl.
- **Scribd.** Save readable books as PDF using your own session. *planned*

### Images (3)

- **Upscale.** Real-ESRGAN on any Vulkan GPU, 2x, 3x or 4x. *beta*
- **Resize images.** Batch resize by width, height, fit or percent, converting the format if you want.
- **OCR.** Copy the text out of images and slides. *beta*

### Files (4)

- **Duplicates.** Find identical files by hash and free space safely.
- **Bulk rename.** Regex, counters and case changes with a preview before applying.
- **Find files.** Instant search with Everything on Windows, Spotlight on macOS or fd on Linux.
- **Keep awake.** Stop the computer from sleeping during long jobs.

### Downloads (2)

- **Accelerated download.** Big files with 16 connections, resume and checksum via aria2.
- **HLS / DASH manifest.** Paste a `.m3u8` or `.mpd` with Referer and cookie. FFmpeg saves an MP4.

### Phone (1)

- **Send to phone.** Files, links and text to a paired KDE Connect device.

### System (9, Windows-only items marked)

- **Clean caches.** Temp files, logs and app caches with rules per operating system. You review the list before anything is deleted.
- **Disk analyzer.** What takes space, as a treemap plus the largest files, with a send-to-trash button.
- **Startup manager.** See what launches with the system and switch items off. *beta*
- **Uninstaller.** Remove apps and the leftovers they leave behind. *beta*
- **Privacy shield.** Control Windows telemetry, ad ID and tracking settings. Windows. *beta*
- **Harden Windows.** Macros, AutoRun, script host, UAC and Defender settings from hardentools, reversible. Windows. *beta*
- **Debloat Windows.** Remove preinstalled Store apps. Windows. *beta*
- **Registry cleaner.** Orphaned entries, with a `.reg` backup before removal. Windows. *beta*
- **Software updater.** Update programs in bulk through winget, Chocolatey and Scoop. Windows. *beta*

### Automation (1)

- **Auto clicker.** Click at the exact speed you set, with a global hotkey, limits and random ranges. Windows, macOS and Linux. *beta*

### AI (6)

- **Compare prices.** The cost of the same model across providers, with prices from LiteLLM and models.dev.
- **AI spending.** How much OmniGet spent on AI, by day, model and task, from a local ledger.
- **Local models (Ollama).** See, download and remove local models and use them as a free provider.
- **Humanize text.** Rewrite AI-sounding text so it reads like a person wrote it, without changing what it says. Runs on the AI provider you configured. *beta*
- **API keys.** A local vault for keys and accounts, with a connection test, balance for OpenRouter, DeepSeek, SiliconFlow and New API, and export to Claude Code, Codex, Cherry Studio, opencode or a `.env` file.
- **MCP server.** OmniGet's tools exposed over the Model Context Protocol on the local bridge, 31 tools behind the same token the extension uses, with ready-made config snippets for Claude Code, Claude Desktop, Cursor, VS Code, Goose and Codex. *beta*

Every tool that talks to an AI uses the provider you set in **Settings → AI**: OpenAI, Anthropic, or any OpenAI-compatible local endpoint such as Ollama or LM Studio. The key is stored locally and never logged. The auto clicker, dictation and the replay buffer can each get a global shortcut of their own.

---

## Plugins: Courses, Study, Telegram, Convert

Plugins are separate Rust libraries loaded at startup. OmniGet installs its official set on first launch and updates them by itself. The Marketplace page shows what is installed, what each plugin is allowed to do (events, notifications, settings, download folders, proxy, managed tools, download queue), and lets you hide, disable or uninstall any of them.

<p align="center">
  <img src="assets/readme/marketplace.png" alt="OmniGet Marketplace listing the Courses, Study, Telegram and Convert plugins with version, author, permissions and enable switches" width="900" />
</p>

### Courses

Sign in to **Hotmart**, **Udemy**, **Kiwify**, **Rocketseat** or **Meta-Analysis Academy** through a browser window inside the app, with saved cookies from the extension, or with email and password where the platform allows it. OmniGet lists your purchases, opens the course outline so you can tick the sections you want (it tells you how many lectures are DRM-protected and will be skipped), and downloads every lesson and attachment with continuous lecture numbers if you want them. Hotmart uses the current OIDC login flow, so it keeps working after Hotmart's 2026 auth change, and free courses and courses delivered outside Hotmart Club are listed too. Downloaded courses appear in Study automatically.

### Study

Study turns the folder of files you downloaded into something you can actually finish.

- Library and player. Point Study at your course folders (nothing is copied or moved). The player resumes to the second, and pressing **N** captures a note at the current timestamp that jumps back there when clicked.
- Reader. PDF, EPUB, DJVU, MOBI, AZW3, FB2, CBZ, CBR, TXT, RTF and HTML, with highlights, bookmarks, collections, a focus mode and a paper-like theme. Covers, titles and authors are pulled from the files.
- Notes. A Markdown and LaTeX editor with links between pages, a daily journal, templates, tags, a knowledge graph, and export to `.md` or PDF. Any note can become a flashcard.
- Anki. Spaced repetition decks with import from `.apkg`, `.txt` and CSV, filtered decks, presets, note types, tags, media, stats and a review log.
- Focus. Pomodoro and deep-work timers with daily and weekly targets that pause the player when the session ends.
- Progress and achievements. Streaks, daily goals, a year heatmap and local XP with no leaderboard.
- Music. Your local library with covers, artists and albums, synced lyrics, favorites, history, playlists, genres, transcoding, and browsers for Spotify, SoundCloud and YouTube Music so playlists and likes sit next to your files.

### Telegram

Sign in with a QR code or your phone number. Browse every channel and group you belong to, filter by photo, video, document or audio, search files, and download one item or the whole chat with a progress list. Videos from channels can be imported straight into the Study library.

### Convert

FFmpeg conversions with GPU acceleration where the machine has it: container, codec, resolution, bitrate and compression for video and audio, no internet required.

---

## Built-in chat, off by default

OmniGet ships a Discord-style chat called OmniDisc for servers you host yourself with [omnidisc-server](https://github.com/tonhowtf/omnidisc-server). Text channels, direct messages, friends, roles and permissions, pins, search, voice, video and screen sharing. Direct messages and the files sent in them are end-to-end encrypted with MLS, and the key for an encrypted call is derived from the same group, so the server operator cannot listen in. Voice runs in Rust rather than the web view and screen sharing uses the machine's hardware encoder. Files sent through chat are encrypted at rest and deleted from the server after thirty minutes.

It is experimental and does nothing until you turn it on in **Settings → Advanced → Chat (OmniDisc)** and add a server.

---

## For League of Legends players

A League menu sits in the sidebar. It reads your running League client locally, with no account and no third-party build site, and does nothing until the client is open. If you never play, switch it off in **Settings → Advanced → League of Legends** and the menu disappears.

Match scouting for both teams with rank, recent form, KDA and the champions each player actually plays. Win probability that shrinks win rates toward the baseline by sample size and always shows a range. Live gold, CS and level for all ten players. Goals per role you can edit. Runes and summoner spells recommended by the client itself, applied in one click and only ever replacing the page OmniGet created. Champion tiers by role. Player search by Riot ID. Opt-in automation: accept matches, pick and ban from your priority list, grab a champion off the ARAM bench. Every automation has its own switch.

New and marked beta or experimental: a **Profile** tab that edits what other players see (rank shown in chat, challenge medals and title, banner and crest, chat icon, bulk friend management); a **skin, chroma and ward roulette** that rolls an owned skin the moment you lock in, with rerolls; a **champion and lane raffle** for when you want the queue to decide, plus an optional random pick in champion select; full **match history and ranked stats for any player** through the client's own backend gateway, with replay download; and an **AI coach** that reviews a game, spots trends over your last matches or answers a question about the current champ select, using your configured AI provider and OP.GG's public data.

---

## Everything else in the box

- Command palette (**Ctrl+K** or **Cmd+K**) that jumps to any page, setting or tool.
- Clipboard detection that offers to download a copied link with one click on a toast.
- Cookie manager that keeps sessions per site, captured by the extension or imported from a `cookies.txt`, with a test button per domain.
- Video summaries: paste a URL in **Settings → AI**, OmniGet fetches the subtitles and summarizes them in the length and language you choose.
- Send a file to someone: pick a file, share the word code, the other person pastes it in their OmniGet.
- Discord Rich Presence showing what you are listening to, watching or reading. Downloads stay private.
- Tray icon, start with system, start minimized, prevent sleep during downloads.
- Every download keeps the exact yt-dlp command it ran. Open it, edit a flag, retry.
- 14 themes, including Catppuccin (four flavors), Dracula, One Dark Pro, three e-ink variants and three Nyxvamp variants.
- 11 languages: English, Portuguese, Spanish, French, Italian, Greek, Russian, Japanese, Persian, Simplified and Traditional Chinese.
- Runs on Windows, macOS (Apple Silicon and Intel) and Linux (x86_64 and ARM64).

---

## Privacy and what OmniGet refuses to do

Everything runs on your computer. There is no account, no server of ours in the middle, and no telemetry about what you download. Cookies and API keys live in your local profile. The only network calls OmniGet makes on its own are to the sites you asked it to download from, to GitHub for updates and plugins, and to the AI provider you configured, when you use an AI tool.

OmniGet downloads what your own logged-in session can already open. It does not bypass DRM, break paywalls, or share credentials, and DRM-protected lectures are skipped and reported. You are responsible for respecting copyright and each platform's terms of service. The full text is in the app under **About → Terms and ethics**.

---

## Frequently asked questions

**Is OmniGet free?**
Yes. GPL-3.0, no paid tier, no ads, no account.

**Is OmniGet a yt-dlp GUI?**
Partly. yt-dlp handles the long tail of sites and OmniGet bundles it, verifies it and updates it. On top of that sit native extractors for courses, Instagram, X, Pinterest, Bilibili, Telegram and torrents, a queue with resume and retry, the Tools section, and the Study library.

**Can OmniGet download a Udemy or Hotmart course I bought?**
Yes. Install the Courses plugin (it comes preinstalled), sign in through the app, pick the course and sections, and download. Lessons and attachments land in a folder per course and appear in Study. Kiwify, Rocketseat and Meta-Analysis Academy work the same way.

**Can it download Instagram stories, close friends or highlights?**
Yes, using your own session captured by the browser extension. Stories are downloaded without being marked as seen.

**Can it download an X video, a whole thread or all media from a profile?**
Yes. Public posts need no login. Bookmarks and your own follow lists need your X session.

**Can it back up a Pinterest board in original quality?**
Yes, including videos, sections, secret boards with cookies, and incremental sync so you only fetch what is new.

**Does it resume interrupted downloads?**
Yes. Partial files are kept and continued, and rate limits trigger retries with backoff.

**Which formats can it save?**
Video as MP4, MKV or WebM. Audio as MP3, M4A, Opus, FLAC or WAV. Subtitles as SRT, VTT or ASS, embedded or beside the file.

**Does it need Python, Node or a terminal?**
No. Download the app, open it, paste a link. The only terminal step is the one-time macOS Gatekeeper fix above.

**macOS says the app is damaged.**
Run the two commands in [the first launch section](#the-first-launch-warning-and-how-to-clear-it). It happens because the app is not notarized, and it happens once.

**Can I transcribe a video to subtitles offline?**
Yes. Tools → Speech and subtitles → Transcribe uses whisper.cpp locally. Models download on demand.

**Can I run it from a USB stick?**
Yes, on Windows, with a `portable.txt` file next to the executable.

**Which Linux package should I pick?**
Debian and Ubuntu: `.deb`. Fedora, openSUSE, RHEL family: `.rpm`. Anything else: `.AppImage`. Both x86_64 and ARM64 are published. OmniGet is not on Flathub.

---

## Command line

`omniget-cli` ships with every release for Windows, macOS (Intel and Apple Silicon) and Linux. Grab `omniget-cli-<version>-<target>` from the [latest release](https://github.com/tonhowtf/omniget/releases/latest).

```bash
omniget info <url>                     # title, formats and size, downloads nothing
omniget download <url> -q 1080 -o ~/Videos
omniget download <url> --audio-only --subs en,pt
omniget batch links.txt -m 3           # one URL per line, 3 at a time
omniget import-cookies cookies.txt     # Netscape format
```

---

## Build from source

If you only want to use OmniGet, [grab a release](#download-and-install). To build it you need [Rust](https://rustup.rs/) (the exact toolchain is pinned in `rust-toolchain.toml` because the plugin ABI depends on it), [Node.js](https://nodejs.org/) 18+ and [pnpm](https://pnpm.io/).

```bash
git clone https://github.com/tonhowtf/omniget.git
cd omniget
pnpm install
pnpm tauri dev
```

<details>
<summary>Linux build dependencies</summary>

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev \
  libayatana-appindicator3-dev librsvg2-dev patchelf libasound2-dev libpipewire-0.3-dev clang libclang-dev
```

</details>

Production build:

```bash
pnpm tauri build --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

Releases sign their updater artifacts with a private key only the maintainer holds, so a plain `pnpm tauri build` stops with "A public key has been found, but no private key". The flag above turns those artifacts off for a local build and changes nothing else.

The plugins live in their own repositories: [omniget-plugin-courses](https://github.com/tonhowtf/omniget-plugin-courses), [omniget-plugin-telegram](https://github.com/tonhowtf/omniget-plugin-telegram), [omniget-plugin-convert](https://github.com/tonhowtf/omniget-plugin-convert) and [omniget-study-release](https://github.com/tonhowtf/omniget-study-release). The registry is [omniget-plugins](https://github.com/tonhowtf/omniget-plugins). `pnpm plugins:deploy` builds the sibling plugin checkouts and copies them into your local data folder.

Stack: Tauri 2, Rust, SvelteKit with Svelte 5, SQLite, yt-dlp, FFmpeg, librqbit for torrents, whisper.cpp, aria2, gallery-dl.

---

## Contributing and translations

Bug reports and pull requests go to [Issues](https://github.com/tonhowtf/omniget/issues) and [Pull requests](https://github.com/tonhowtf/omniget/pulls). Questions and quick help live on [Discord](https://discord.gg/jgdxyPy7Vn).

Translations are managed on [Weblate](https://hosted.weblate.org/engage/omniget/). Pick your language and translate in the browser. New strings appear there a few hours after they land in `main`.

OmniGet is built on [yt-dlp](https://github.com/yt-dlp/yt-dlp), [FFmpeg](https://ffmpeg.org/), [gallery-dl](https://github.com/mikf/gallery-dl), [whisper.cpp](https://github.com/ggerganov/whisper.cpp), [aria2](https://aria2.github.io/), [SponsorBlock](https://sponsor.ajay.app/), [Return YouTube Dislike](https://returnyoutubedislike.com/), [FxTwitter](https://github.com/FixTweet/FxTwitter), [Spicetify](https://spicetify.app/) and [Tauri](https://tauri.app/). Thank you to everyone who maintains them.

Loop, the creature on the home screen, is OmniGet's mascot. Fan art is welcome. The original artwork may not be used commercially or redistributed modified.

<p align="center">
  <a href="https://star-history.com/#tonhowtf/omniget&Date"><img src="https://api.star-history.com/svg?repos=tonhowtf/omniget&type=Date" alt="Star history of tonhowtf/omniget" width="600" /></a>
</p>

<p align="center">
  <a href="https://github.com/tonhowtf/omniget/releases/latest"><b>Download OmniGet</b></a> · <a href="LICENSE">GPL-3.0</a>
</p>
