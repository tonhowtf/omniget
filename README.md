<p align="center">
  <img src="static/loop.png" alt="OmniGet Loop mascot" width="120" />
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-green?style=for-the-badge" alt="License GPL-3.0" /></a>
  <a href="https://github.com/tonhowtf/omniget/stargazers"><img src="https://img.shields.io/github/stars/tonhowtf/omniget?style=for-the-badge" alt="Stars" /></a>
</p>

<h1 align="center">OmniGet</h1>

<h3 align="center">Paste a link. Get your file.<br>No browser extensions, no web apps, no nonsense.</h3>

OmniGet is a free, open source desktop app for downloading videos and media from the internet. It supports YouTube, Instagram, TikTok, Twitter/X, Reddit, Twitch, Pinterest, Bluesky, and Hotmart courses. Built with Tauri and Rust for speed and a small footprint.

<p align="center">
  <img src="omniget.jpg" alt="OmniGet screenshot" width="800" />
</p>

## Features

- Supports 9+ platforms: YouTube, Instagram, TikTok, Twitter/X, Reddit, Twitch, Pinterest, Bluesky
- Download Hotmart courses with login integration (videos, attachments, descriptions)
- Real-time download progress with ETA and speed
- Loop, the expressive mascot that reacts to your downloads
- Dark and light theme
- Multi-language (English and Portuguese)
- Built with Tauri + Rust. Fast, lightweight, no Electron bloat
- Free and open source under GPL-3.0

## Supported Platforms

| Platform | Content |
|----------|---------|
| YouTube | Videos, Shorts, Playlists |
| Instagram | Posts, Reels, Stories |
| TikTok | Videos, Photos |
| Twitter / X | Videos, GIFs |
| Reddit | Videos, Images |
| Twitch | Clips |
| Pinterest | Images, Videos |
| Bluesky | Images, Videos |
| Hotmart | Full courses (with login) |

Platform availability may change depending on each service.

## Download

Grab the latest release for your platform:

<p align="center">
  <a href="https://github.com/tonhowtf/omniget/releases/latest"><img src="https://img.shields.io/badge/-Windows-blue.svg?style=for-the-badge&logo=windows" alt="Download for Windows" /></a>
</p>

OmniGet currently supports Windows. macOS and Linux support is planned.

## Windows SmartScreen

Since OmniGet is a new, unsigned application, Windows SmartScreen may show a warning when you first run it. This is normal for open source apps without a paid code signing certificate. To proceed, click **More info** and then **Run anyway**. The app is fully open source and you can inspect every line of code in this repository.

## Building from source

Prerequisites: [Rust](https://rustup.rs/), [Node.js](https://nodejs.org/) 18+, [pnpm](https://pnpm.io/)

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

## Legal

OmniGet is a tool that facilitates downloading publicly available content from the internet. You are responsible for how you use it. Personal use is supported by Article 184, §4 of the Brazilian Penal Code, which allows copying intellectual works for private use without profit intent. Please respect copyright and the terms of service of each platform.

## Contributing

Contributions are welcome! If you found a bug or want to suggest a feature, [open an issue](https://github.com/tonhowtf/omniget/issues). Pull requests are appreciated.

## License

OmniGet is licensed under [GPL-3.0](LICENSE). The OmniGet name, logo, and Loop mascot are project trademarks and are not covered by the code license.

<!--
GitHub Settings Reminder:
About: "Free, open source media downloader. YouTube, Instagram, TikTok, Twitter/X and more. Built with Tauri + Rust."
Topics: downloader, media-downloader, youtube-downloader, video-downloader, tiktok-downloader, instagram-downloader, twitter-downloader, reddit-downloader, tauri, rust, svelte, desktop-app, open-source, download-manager
-->
