#!/usr/bin/env node
/*
 * Screenshot harness for the visual remake.
 * Usage: node scripts/shots.mjs [phase] [--routes=/a,/b] [--themes=light,dark]
 * Captures every main route at 3 viewports x 2 themes into remake/shots/<phase>/.
 * Runs against `vite dev` (port 1420) with the Tauri IPC fully mocked, so no
 * real backend, credentials or user data are involved.
 */
import { chromium } from "playwright";
import { mkdirSync, existsSync } from "node:fs";
import { spawn } from "node:child_process";
import path from "node:path";

const PHASE = process.argv[2] && !process.argv[2].startsWith("--") ? process.argv[2] : "baseline";
const argOf = (name) => {
  const a = process.argv.find((x) => x.startsWith(`--${name}=`));
  return a ? a.slice(a.indexOf("=") + 1) : null;
};

const BASE = "http://localhost:1420";
const OUT = path.resolve("remake/shots", PHASE);

const VIEWPORTS = [
  { name: "390x844", width: 390, height: 844 },
  { name: "834x1194", width: 834, height: 1194 },
  { name: "1440x900", width: 1440, height: 900 },
];

const THEMES = (argOf("themes") || "light,dark").split(",");

const DEFAULT_ROUTES = [
  "/_kitchen-sink",
  "/",
  "/downloads",
  "/marketplace",
  "/settings",
  "/about",
  "/about/changelog",
  "/about/project",
  "/about/terms",
  "/about/privacy",
  "/courses",
  "/convert",
  "/telegram",
  "/misc",
  "/misc/studio",
  "/misc/library",
  "/misc/file-clip",
  "/study",
  "/study/player",
  "/study/read",
  "/study/library",
  "/study/music",
  "/study/watch",
];

const ROUTES = argOf("routes") ? argOf("routes").split(",") : DEFAULT_ROUTES;

const SETTINGS = {
  schema_version: 1,
  appearance: { theme: "dark", language: "en" },
  download: {
    default_output_dir: "/Users/demo/Downloads/OmniGet",
    always_ask_path: false,
    video_quality: "1080p",
    skip_existing: true,
    download_attachments: false,
    download_descriptions: false,
    embed_metadata: true,
    embed_thumbnail: true,
    clipboard_detection: false,
    auto_download_on_paste: false,
    filename_template: "%(title)s.%(ext)s",
    organize_by_platform: true,
    download_subtitles: false,
    include_auto_subtitles: false,
    caption_locale: "en",
    keep_vtt: false,
    subtitle_format: "srt",
    embed_subtitles: false,
    keep_subtitle_files: true,
    skip_archived: false,
    continuous_lecture_numbers: false,
    translate_metadata: false,
    youtube_sponsorblock: false,
    sponsorblock_mode: "mark",
    sponsorblock_categories: [],
    split_by_chapters: false,
    live_from_start: false,
    speed_limit: "",
    hotkey_enabled: true,
    hotkey_binding: "CmdOrCtrl+Shift+D",
    music_hotkey_enabled: false,
    music_hotkey_binding: "CmdOrCtrl+Shift+M",
    music_audio_format: "mp3",
    copy_to_clipboard_on_hotkey: false,
    cookie_file: "",
    always_use_managed_cookies: false,
    bilibili_danmaku_enabled: false,
    bilibili_danmaku_format: "ass",
    bilibili_container: "mp4",
    bilibili_nfo_enabled: false,
    bilibili_cover_sidecar: false,
    bilibili_cover_format: "jpg",
    bilibili_naming_video: "",
    bilibili_naming_multi_part: "",
    bilibili_naming_bangumi: "",
    bilibili_naming_cheese: "",
    bilibili_naming_collection: "",
    bilibili_cdn_hosts: "",
    bilibili_cdn_prefer_alternatives: false,
    bilibili_preferred_qn: 80,
    bilibili_preferred_codec: 7,
    bilibili_preferred_audio_qn: 30280,
  },
  proxy: { enabled: false, proxy_type: "http", host: "", port: 8080, username: "", password: "" },
  advanced: {
    max_concurrent_segments: 4,
    max_retries: 3,
    max_concurrent_downloads: 3,
    concurrent_fragments: 4,
    stagger_delay_ms: 500,
    torrent_listen_port: 6881,
    torrent_auto_trackers: true,
    torrent_upnp: true,
    prevent_sleep: true,
    cookies_from_browser: "",
    twitter_manual_cookie: "",
    user_agent: "",
  },
  telegram: { concurrent_downloads: 2, fix_file_extensions: true },
  rpc: { enabled: false, app_id: "", large_image_key: "" },
  onboarding_completed: true,
  start_with_system: false,
  start_minimized: false,
  legal_acknowledged: true,
  last_download_options: { mode: "auto", quality: "1080p" },
};

const navLabel = (en) => ({ en });
const PLUGINS = [
  {
    id: "courses", name: "Courses", version: "1.4.0", description: "Download courses", author: "tonhowtf",
    enabled: true, loaded: true, icon: null, load_error: null,
    nav: [{ route: "/courses", label: navLabel("Courses"), icon_svg: null, group: "plugins", order: 10 }],
  },
  {
    id: "study", name: "Study", version: "2.1.0", description: "Reader, player, notes", author: "tonhowtf",
    enabled: true, loaded: true, icon: null, load_error: null,
    nav: [{ route: "/study", label: navLabel("Study"), icon_svg: null, group: "plugins", order: 20 }],
  },
  {
    id: "telegram", name: "Telegram", version: "1.2.0", description: "Telegram downloads", author: "tonhowtf",
    enabled: true, loaded: true, icon: null, load_error: null,
    nav: [{ route: "/telegram", label: navLabel("Telegram"), icon_svg: null, group: "plugins", order: 30 }],
  },
  {
    id: "convert", name: "Convert", version: "1.0.3", description: "Media conversion", author: "tonhowtf",
    enabled: true, loaded: true, icon: null, load_error: null,
    nav: [{ route: "/convert", label: navLabel("Convert"), icon_svg: null, group: "plugins", order: 40 }],
  },
  {
    id: "misc", name: "Utilities", version: "1.1.0", description: "Studio, clips, library", author: "tonhowtf",
    enabled: true, loaded: true, icon: null, load_error: null,
    nav: [{ route: "/misc", label: navLabel("Utilities"), icon_svg: null, group: "plugins", order: 50 }],
  },
];

const now = Math.floor(Date.now() / 1000);
const HISTORY = [
  { id: 101, url: "https://www.youtube.com/watch?v=abc123", platform: "youtube", title: "Building a Desktop App with Tauri 2 — Full Walkthrough", file_path: "/Users/demo/Downloads/OmniGet/YouTube/tauri-walkthrough.mp4", file_size_bytes: 734003200, total_bytes: 734003200, success: true, error: null, completed_at: now - 3600, thumbnail_url: null, kind: "video" },
  { id: 102, url: "https://vimeo.com/9911223", platform: "vimeo", title: "Ambient Study Mix Vol. 4", file_path: "/Users/demo/Downloads/OmniGet/Vimeo/ambient-mix-4.m4a", file_size_bytes: 88080384, total_bytes: 88080384, success: true, error: null, completed_at: now - 7200, thumbnail_url: null, kind: "audio" },
  { id: 103, url: "https://www.instagram.com/p/Cxyz/", platform: "instagram", title: "Travel reel — Kyoto in autumn", file_path: null, file_size_bytes: null, total_bytes: null, success: false, error: "Login required: this post is from a private account", completed_at: now - 10800, thumbnail_url: null, kind: "video" },
  { id: 104, url: "https://twitter.com/user/status/1", platform: "twitter", title: "Thread screenshots (4 images)", file_path: "/Users/demo/Downloads/OmniGet/Twitter/thread-imgs", file_size_bytes: 8388608, total_bytes: 8388608, success: true, error: null, completed_at: now - 86400, thumbnail_url: null, kind: "image" },
  { id: 105, url: "https://www.reddit.com/r/rust/comments/xyz", platform: "reddit", title: "Why Rust for desktop apps — discussion", file_path: "/Users/demo/Downloads/OmniGet/Reddit/rust-desktop.mp4", file_size_bytes: 157286400, total_bytes: 157286400, success: true, error: null, completed_at: now - 172800, thumbnail_url: null, kind: "video" },
  { id: 106, url: "https://www.tiktok.com/@user/video/72", platform: "tiktok", title: "60-second pasta recipe", file_path: "/Users/demo/Downloads/OmniGet/TikTok/pasta.mp4", file_size_bytes: 26214400, total_bytes: 26214400, success: true, error: null, completed_at: now - 259200, thumbnail_url: null, kind: "video" },
];

const QUEUE = [
  { id: 1, url: "https://www.youtube.com/watch?v=live1", platform: "youtube", title: "Conference Keynote 2026 — Day 1 (4K)", status: { type: "Downloading" }, percent: 42.5, speed_bytes_per_sec: 11534336, downloaded_bytes: 891289600, total_bytes: 2097152000, file_path: null, file_size_bytes: null, file_count: null, thumbnail_url: null, eta_seconds: 105 },
  { id: 2, url: "https://www.twitch.tv/videos/222", platform: "twitch", title: "Speedrun VOD — GDQ finals", status: { type: "Queued" }, percent: 0, speed_bytes_per_sec: 0, downloaded_bytes: 0, total_bytes: null, file_path: null, file_size_bytes: null, file_count: null, thumbnail_url: null, eta_seconds: null },
  { id: 3, url: "https://www.bilibili.com/video/BV1xx", platform: "bilibili", title: "Lo-fi mix for late nights", status: { type: "Complete" }, percent: 100, speed_bytes_per_sec: 0, downloaded_bytes: 104857600, total_bytes: 104857600, file_path: "/Users/demo/Downloads/OmniGet/Bilibili/lofi.mp4", file_size_bytes: 104857600, file_count: 1, thumbnail_url: null, eta_seconds: null },
  { id: 4, url: "https://www.youtube.com/watch?v=priv1", platform: "youtube", title: "Members-only masterclass", status: { type: "Error", data: "ERROR: [youtube] priv1: Private video. Sign in if you've been granted access" }, percent: 0, speed_bytes_per_sec: 0, downloaded_bytes: 0, total_bytes: null, file_path: null, file_size_bytes: null, file_count: null, thumbnail_url: null, eta_seconds: null },
];

const REGISTRY = [
  { id: "courses", name: "Courses", description: "Download from Hotmart, Udemy, Kiwify and Rocketseat.", author: "tonhowtf", repo: "tonhowtf/omniget-plugin-courses", homepage: null, tags: ["courses", "education"], official: true, capabilities: ["nav"], installed: true, installed_version: "1.4.0" },
  { id: "study", name: "Study", description: "Reader, player, notes, flashcards and focus tools.", author: "tonhowtf", repo: "tonhowtf/omniget-study", homepage: null, tags: ["study", "reader"], official: true, capabilities: ["nav"], installed: true, installed_version: "2.1.0" },
  { id: "telegram", name: "Telegram", description: "Browse and batch-download from Telegram chats.", author: "tonhowtf", repo: "tonhowtf/omniget-plugin-telegram", homepage: null, tags: ["telegram"], official: true, capabilities: ["nav"], installed: true, installed_version: "1.2.0" },
  { id: "convert", name: "Convert", description: "FFmpeg conversions with GPU acceleration.", author: "tonhowtf", repo: "tonhowtf/omniget-plugin-convert", homepage: null, tags: ["ffmpeg", "convert"], official: true, capabilities: ["nav"], installed: true, installed_version: "1.0.3" },
  { id: "misc", name: "Utilities", description: "Screen recording studio, file clips and media library.", author: "tonhowtf", repo: "tonhowtf/omniget-plugin-misc", homepage: null, tags: ["studio", "library"], official: true, capabilities: ["nav"], installed: false, installed_version: null },
];

const DEPS = [
  { name: "yt-dlp", installed: true, version: "2026.07.10" },
  { name: "ffmpeg", installed: true, version: "7.1" },
  { name: "pdfium", installed: false, version: null },
];

const FIXTURES = { SETTINGS, PLUGINS, HISTORY, QUEUE, REGISTRY, DEPS };

function initScript(theme) {
  return `(() => {
    const F = ${JSON.stringify(FIXTURES)};
    F.SETTINGS.appearance.theme = ${JSON.stringify(theme)};
    let cbId = 4000;
    const listeners = {};
    window.__MOCK_EMIT = (name, payload) => {
      for (const cb of (listeners[name] || [])) {
        try { cb({ event: name, id: 1, payload }); } catch (e) {}
      }
    };
    const respond = (cmd, args) => {
      switch (cmd) {
        case "plugin:event|listen": {
          const cb = window["_" + args.handler];
          if (cb) (listeners[args.event] ||= []).push(cb);
          return ++cbId;
        }
        case "plugin:event|unlisten":
        case "plugin:event|emit":
        case "plugin:event|emit_to":
          return null;
        case "plugin:app|version": return "0.7.0";
        case "plugin:app|name": return "OmniGet";
        case "plugin:updater|check": return null;
        case "get_settings": return F.SETTINGS;
        case "save_settings": return null;
        case "list_plugins": return F.PLUGINS;
        case "check_ytdlp_available": return true;
        case "register_external_frontend": return [];
        case "check_cookie_error": return false;
        case "get_download_history": return F.HISTORY;
        case "check_dependencies": return F.DEPS;
        case "fetch_marketplace_registry": return F.REGISTRY;
        case "check_plugin_updates": return [];
        case "rpc_set_idle_stats": return null;
        case "plugin_command": return null;
        case "diagnose_download_error": {
          const t = String(args.stderr || "").toLowerCase();
          if (t.includes("private video") || t.includes("sign in")) {
            return { cause_key: "error.cause.needs_login", remedy: "import_cookies", detail: null };
          }
          return null;
        }
        default: return null;
      }
    };
    window.__TAURI_INTERNALS__ = {
      invoke: (cmd, args = {}) => {
        try { return Promise.resolve(respond(cmd, args)); }
        catch (e) { return Promise.reject(String(e)); }
      },
      transformCallback: (cb, once) => {
        const id = ++cbId;
        window["_" + id] = cb;
        return id;
      },
      unregisterCallback: (id) => { delete window["_" + id]; },
      convertFileSrc: (p) => p,
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
    };
    window.isTauri = true;
  })();`;
}

async function waitForServer(url, ms) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url);
      if (res.ok) return true;
    } catch {}
    await new Promise((r) => setTimeout(r, 500));
  }
  return false;
}

async function main() {
  let devProc = null;
  const up = await waitForServer(BASE, 2000);
  if (!up) {
    console.log("starting vite dev...");
    devProc = spawn("pnpm", ["dev"], {
      cwd: process.cwd(),
      env: { ...process.env, OMNIGET_I18N_STRICT: "0" },
      stdio: "ignore",
      detached: false,
    });
    const ok = await waitForServer(BASE, 90000);
    if (!ok) {
      console.error("vite dev did not come up on " + BASE);
      process.exit(1);
    }
  }

  mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch();
  let count = 0;
  const failures = [];

  for (const theme of THEMES) {
    for (const vp of VIEWPORTS) {
      const ctx = await browser.newContext({
        viewport: { width: vp.width, height: vp.height },
        colorScheme: theme === "light" ? "light" : "dark",
        deviceScaleFactor: 2,
        reducedMotion: "reduce",
      });
      await ctx.addInitScript(initScript(theme));
      const page = await ctx.newPage();
      for (const route of ROUTES) {
        const slug = route === "/" ? "home" : route.replace(/^\//, "").replace(/[\/?=&]/g, "-");
        const file = path.join(OUT, `${slug}__${vp.name}__${theme}.png`);
        try {
          await page.goto(BASE + route, { waitUntil: "domcontentloaded", timeout: 30000 });
          await page.evaluate(() => document.fonts.ready.catch(() => {}));
          await page.waitForTimeout(400);
          await page.evaluate((queue) => {
            if (window.__MOCK_EMIT) window.__MOCK_EMIT("queue-state-update", queue);
          }, QUEUE);
          await page.waitForTimeout(700);
          await page.screenshot({ path: file });
          count++;
          process.stdout.write(`\r${count} shots (${slug} ${vp.name} ${theme})        `);
        } catch (e) {
          failures.push({ route, vp: vp.name, theme, error: String(e).slice(0, 200) });
        }
      }
      await ctx.close();
    }
  }

  await browser.close();
  console.log(`\ndone: ${count} screenshots in ${OUT}`);
  if (failures.length) {
    console.log("failures:");
    for (const f of failures) console.log(` - ${f.route} ${f.vp} ${f.theme}: ${f.error}`);
  }
  if (devProc) devProc.kill();
  process.exit(failures.length ? 2 : 0);
}

main();
