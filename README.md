<p align="center">
  <img src="static/loop.png" alt="Loop, the OmniGet mascot" width="120" />
</p>

<p align="center">
  <a href="https://github.com/tonhowtf/omniget/releases/latest"><img src="https://img.shields.io/github/v/release/tonhowtf/omniget?style=for-the-badge&label=release" alt="Latest Release" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-green?style=for-the-badge" alt="License GPL-3.0" /></a>
  <a href="https://github.com/tonhowtf/omniget/stargazers"><img src="https://img.shields.io/github/stars/tonhowtf/omniget?style=for-the-badge" alt="Stars" /></a>
</p>

<h1 align="center">OmniGet</h1>

<h3 align="center">Paste a link. Get your file.<br>No browser extensions, no web apps</h3>

OmniGet is a free, open source desktop app for downloading videos, media, and full courses from the internet. It natively supports 50+ platforms including YouTube, Instagram, TikTok, Twitter/X, Reddit, Twitch, Pinterest, Vimeo, Bluesky, and Chinese platforms like Bilibili (哔哩哔哩), Douyin (抖音), Xiaohongshu (小红书), Kuaishou (快手), Youku (优酷视频), Tencent Video (腾讯视频), iQiyi (爱奇艺), and Mango TV (芒果TV). It downloads full courses from 35+ education platforms like Hotmart, Udemy, Kiwify, Teachable, Kajabi, Skool, Pluralsight, MasterClass, Rocketseat, Estratégia Concursos, and more. It also downloads torrents and magnet links natively, and lets you send files directly between computers via P2P transfer over local network or the internet. Any other URL falls back to yt-dlp, covering [1000+ additional sites](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md).

Built with Tauri and Rust for speed and a small footprint.

<p align="center">
  <img src="assets/screenshot.png" alt="OmniGet screenshot" width="800" />
</p>

## Features

- Download from 50+ platforms natively, plus [1000+ more via yt-dlp](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md)
- Download torrents and magnet links natively with seeding, pause/resume
- Open .torrent files directly with file picker or drag-and-drop
- P2P file transfer between devices — works across different networks (powered by [iroh](https://github.com/n0-computer/iroh) with automatic hole punching and relay fallback)
- Download full courses from 35+ platforms with login (videos, attachments, descriptions)
- Download Telegram media with QR code or phone number login
- Convert media files between formats with FFmpeg and GPU acceleration
- Search YouTube directly from the omnibox
- Choose quality, format, and download mode (video, audio only, mute)
- Smart format selection with H.264+AAC codec preference for maximum compatibility
- Real-time progress with speed display
- Global hotkey to download from clipboard URL
- Clipboard URL detection and batch downloads
- System tray with download count badge
- Built-in auto-updater (AppImage with embedded update information and zsync delta updates on Linux)
- Windows portable binary available (no installation required)
- Debug diagnostics export for troubleshooting
- Proxy support (HTTP/SOCKS5)
- Firefox-first cookie detection for authenticated downloads
- Dark and light theme
- Available in English, Portuguese, Chinese, Japanese, Italian, French, and Greek
- Loop, the mascot that reacts to your downloads

## Supported Platforms

### Media Platforms

| Platform | Content |
|----------|---------|
| YouTube | Videos, Shorts, Playlists, Search |
| Instagram | Posts, Reels, Stories |
| TikTok | Videos, Photos |
| Twitter / X | Videos, GIFs |
| Reddit | Videos, Images |
| Twitch | Clips |
| Pinterest | Images, Videos |
| Vimeo | Videos (with referer support) |
| Bluesky | Images, Videos |
| Bilibili (哔哩哔哩) | Videos, Series, Playlists |
| Douyin (抖音) | Videos |
| Xiaohongshu (小红书) | Videos, Images |
| Kuaishou (快手) | Videos |
| Youku (优酷) | Videos |
| Tencent Video (腾讯视频) | Videos |
| iQiyi (爱奇艺) | Videos |
| Mango TV (芒果TV) | Videos |
| Telegram | Photos, Videos, Files, Audio |
| Torrent / Magnet | Any .torrent file or magnet link |
| [1000+ more](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md) | Anything yt-dlp supports |

### Course Platforms

| Platform | Auth | Region |
|----------|------|--------|
| Hotmart | Email + Password | BR / Global |
| Udemy | Email + Cookies | Global |
| Kiwify | Email + Password / Token | BR |
| Gumroad | Email + Password / Token | Global |
| Teachable | OTP (Email) | Global |
| Kajabi | OTP (Email) | Global |
| Skool | Email + Password | Global |
| Pluralsight | Browser Cookies | Global |
| MasterClass | Browser Cookies | Global |
| Wondrium / Great Courses | Email + Password / Token | US |
| Thinkific | Browser Cookies | Global |
| Rocketseat | Browser Cookies | BR |
| Estratégia Concursos | Token / Cookies | BR |
| Estratégia LDI | Token / Cookies | BR |
| Estratégia Militares | Token / Cookies | BR |
| Gran Cursos Online | Session Cookies | BR |
| Fluency Academy | Email + Password / Token | BR |
| Data Science Academy | Token | BR |
| Eduzz / Nutror | Token | BR |
| Kirvano | Token | BR |
| MemberKit | Email + Password / Cookies | BR |
| Cademi | Email + Password / Cookies | BR |
| Curseduca | Email + Password / Token | BR |
| Medcel | Token + API Key | BR |
| Medcof | Token | BR |
| Medway | Token | BR |
| Afya Internato | Token + API Key | BR |
| AlpaClass | Token | BR |
| Área de Membros | Cookies | BR |
| Astron Members | Email + Password / Cookies | BR |
| Cakto | Email + Password / Cookies | BR |
| Cakto Members | Cookies | BR |
| Greenn Club | Token | BR |
| TheMembers | Email + Password / Token | BR |
| Voomp Play | Token | BR |
| Entrega Digital | Token + Metadata | BR |

Platform availability may vary depending on each service. Chinese platforms may require a Chinese IP address (VPN/proxy). Some streaming platforms use DRM on premium content.

## Download

Grab the latest release for your platform:

<p align="center">
  <a href="https://github.com/tonhowtf/omniget/releases/latest"><img src="https://img.shields.io/badge/-Windows-blue.svg?style=for-the-badge&logo=windows" alt="Download for Windows" /></a>
  <a href="https://github.com/tonhowtf/omniget/releases/latest"><img src="https://img.shields.io/badge/-macOS-black.svg?style=for-the-badge&logo=apple" alt="Download for macOS" /></a>
  <a href="https://github.com/tonhowtf/omniget/releases/latest"><img src="https://img.shields.io/badge/-Linux-orange.svg?style=for-the-badge&logo=linux&logoColor=white" alt="Download for Linux" /></a>
</p>

Linux is also available as a Flatpak. If you run into issues on any platform, please [open an issue](https://github.com/tonhowtf/omniget/issues).

## Windows SmartScreen

Windows SmartScreen may warn you the first time you run OmniGet. This is normal for open source apps without a paid code signing certificate. Click **More info**, then **Run anyway**.

The app is fully open source and every line of code is right here in this repository.

## macOS Gatekeeper

macOS may block OmniGet because the app is not yet signed with an Apple Developer certificate. If you see "omniget.app is damaged" or "can't be opened", run this in Terminal:

```bash
xattr -cr /Applications/omniget.app
codesign --force --deep --sign - /Applications/omniget.app
```

## Building From Source

Prerequisites: [Rust](https://rustup.rs/), [Node.js](https://nodejs.org/) 18+, [pnpm](https://pnpm.io/)

On Linux, install additional dependencies:

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

Then build and run:

```bash
git clone https://github.com/tonhowtf/omniget.git
cd omniget
pnpm install
pnpm tauri dev
```

For a production build:

```bash
pnpm tauri build
```

## Notice to Platform Owners

OmniGet is a personal-use tool built in good faith. If you are an authorized representative of a platform listed in this project and have concerns about its inclusion, please send an email to **tonhowtf@gmail.com** from an official company domain address (or include verifiable proof of your role) and the platform will be removed promptly.

## Legal

OmniGet facilitates downloading publicly available content from the internet. You are responsible for how you use it. Personal use is supported by Article 184, §4 of the Brazilian Penal Code, which allows copying works for private use without profit intent.

Respect copyright and each platform's terms of service.

## Contributing

Found a bug or want to suggest a feature? [Open an issue](https://github.com/tonhowtf/omniget/issues). Pull requests are welcome.

## License

OmniGet is licensed under [GPL-3.0](LICENSE). The OmniGet name, logo, and Loop mascot are project trademarks not covered by the code license.

<!-- omniget, video downloader, media downloader, course downloader, youtube downloader, instagram downloader, tiktok downloader, twitter downloader, reddit downloader, twitch downloader, pinterest downloader, vimeo downloader, bluesky downloader, bilibili downloader, douyin downloader, xiaohongshu downloader, hotmart downloader, udemy downloader, kiwify downloader, teachable downloader, kajabi downloader, skool downloader, pluralsight downloader, masterclass downloader, rocketseat downloader, estrategia concursos downloader, telegram downloader, torrent downloader, magnet downloader, desktop app, open source, rust, tauri -->
