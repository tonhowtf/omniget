#!/usr/bin/env node
/*
 * Behavioral a11y/motion audit for the remake (Fase 3 da auditoria).
 * - prefers-reduced-motion: compares running animations with and without the preference
 *   and flags transform/position animations that survive `reduce`.
 * - click targets: measures every interactive element against the macOS-desktop
 *   minimums adopted (>=28px standard controls, >=20px dense in-row targets).
 * - keyboard: tabs through the page and reports focus-visible affordance.
 * Reuses the Tauri IPC mock from shots.mjs via a tiny inline copy.
 */
import { chromium } from "playwright";
import { spawn } from "node:child_process";

const BASE = "http://localhost:1420";
const ROUTES = (process.argv[2] || "/,/downloads,/settings,/_kitchen-sink").split(",");

const SETTINGS = { schema_version: 1, appearance: { theme: "dark", language: "en" }, download: { default_output_dir: "/tmp", always_ask_path: false, video_quality: "1080p", skip_existing: true, download_attachments: false, download_descriptions: false, embed_metadata: true, embed_thumbnail: true, clipboard_detection: false, auto_download_on_paste: false, filename_template: "%(title)s.%(ext)s", organize_by_platform: true, download_subtitles: false, include_auto_subtitles: false, caption_locale: "en", keep_vtt: false, subtitle_format: "srt", embed_subtitles: false, keep_subtitle_files: true, skip_archived: false, continuous_lecture_numbers: false, translate_metadata: false, youtube_sponsorblock: false, sponsorblock_mode: "mark", sponsorblock_categories: [], split_by_chapters: false, live_from_start: false, speed_limit: "", hotkey_enabled: true, hotkey_binding: "CmdOrCtrl+Shift+D", music_hotkey_enabled: false, music_hotkey_binding: "", music_audio_format: "mp3", copy_to_clipboard_on_hotkey: false, cookie_file: "", always_use_managed_cookies: false, bilibili_danmaku_enabled: false, bilibili_danmaku_format: "ass", bilibili_container: "mp4", bilibili_nfo_enabled: false, bilibili_cover_sidecar: false, bilibili_cover_format: "jpg", bilibili_naming_video: "", bilibili_naming_multi_part: "", bilibili_naming_bangumi: "", bilibili_naming_cheese: "", bilibili_naming_collection: "", bilibili_cdn_hosts: "", bilibili_cdn_prefer_alternatives: false, bilibili_preferred_qn: 80, bilibili_preferred_codec: 7, bilibili_preferred_audio_qn: 30280 }, advanced: { max_concurrent_segments: 4, max_retries: 3, max_concurrent_downloads: 3, concurrent_fragments: 4, stagger_delay_ms: 500, torrent_listen_port: 6881, torrent_auto_trackers: true, torrent_upnp: true, prevent_sleep: true, cookies_from_browser: "", twitter_manual_cookie: "", user_agent: "" }, telegram: { concurrent_downloads: 2, fix_file_extensions: true }, rpc: { enabled: false, app_id: "", large_image_key: "" }, onboarding_completed: true, start_with_system: false, start_minimized: false, legal_acknowledged: true };

function initScript() {
  return `(() => {
    const S = ${JSON.stringify(SETTINGS)};
    let id = 4000; const listeners = {};
    window.__TAURI_INTERNALS__ = {
      invoke: (cmd, args = {}) => {
        switch (cmd) {
          case "plugin:event|listen": { const cb = window["_" + args.handler]; if (cb) (listeners[args.event] ||= []).push(cb); return Promise.resolve(++id); }
          case "get_settings": return Promise.resolve(S);
          case "check_ytdlp_available": return Promise.resolve(true);
          case "register_external_frontend": case "get_download_history": case "check_dependencies": return Promise.resolve([]);
          case "check_cookie_error": return Promise.resolve(false);
          case "plugin:app|version": return Promise.resolve("0.7.0");
          default: return Promise.resolve(null);
        }
      },
      transformCallback: (cb) => { const i = ++id; window["_" + i] = cb; return i; },
      unregisterCallback: (i) => { delete window["_" + i]; },
      convertFileSrc: (p) => p,
    };
    window.isTauri = true;
  })();`;
}

async function waitForServer(url, ms) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    try { if ((await fetch(url)).ok) return true; } catch {}
    await new Promise((r) => setTimeout(r, 500));
  }
  return false;
}

async function collect(browser, reduced, route) {
  const ctx = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    colorScheme: "dark",
    reducedMotion: reduced ? "reduce" : "no-preference",
  });
  await ctx.addInitScript(initScript());
  const page = await ctx.newPage();
  await page.goto(BASE + route, { waitUntil: "domcontentloaded", timeout: 30000 });
  await page.waitForTimeout(900);
  const data = await page.evaluate(() => {
    const anims = document.getAnimations().map((a) => {
      const kf = a.effect?.getKeyframes?.() ?? [];
      const props = new Set();
      for (const f of kf) for (const k of Object.keys(f)) if (!["offset", "easing", "composite", "computedOffset"].includes(k)) props.add(k);
      return { props: [...props] };
    });
    const moving = anims.filter((a) => a.props.some((p) => ["transform", "translate", "left", "top", "scale"].includes(p)));
    const sel = "button, a[href], input, select, textarea, [role='button'], [role='switch'], [role='tab']";
    const targets = [];
    for (const el of document.querySelectorAll(sel)) {
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      const style = getComputedStyle(el);
      if (style.visibility === "hidden" || style.display === "none") continue;
      targets.push({ h: Math.round(r.height), w: Math.round(r.width), tag: el.tagName.toLowerCase(), cls: (el.className || "").toString().split(" ")[0], inRow: !!el.closest(".list-row, .download-item, td, .toast, .mac-nav-item") });
    }
    return { totalAnims: anims.length, movingAnims: moving.length, targets };
  });
  await ctx.close();
  return data;
}

async function keyboardPass(browser, route, tabs) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: "dark", reducedMotion: "reduce" });
  await ctx.addInitScript(initScript());
  const page = await ctx.newPage();
  await page.goto(BASE + route, { waitUntil: "domcontentloaded", timeout: 30000 });
  await page.waitForTimeout(700);
  const seq = [];
  for (let i = 0; i < tabs; i++) {
    await page.keyboard.press("Tab");
    const info = await page.evaluate(() => {
      const el = document.activeElement;
      if (!el || el === document.body) return null;
      const s = getComputedStyle(el);
      const visible = (s.outlineStyle !== "none" && parseFloat(s.outlineWidth) > 0) || s.boxShadow !== "none" || el.matches(".menu-item, .mac-command-item");
      return { tag: el.tagName.toLowerCase(), cls: (el.className || "").toString().split(" ")[0], focusAffordance: visible };
    });
    if (info) seq.push(info);
  }
  await ctx.close();
  return seq;
}

async function main() {
  let devProc = null;
  if (!(await waitForServer(BASE, 2000))) {
    devProc = spawn("pnpm", ["dev"], { env: { ...process.env, OMNIGET_I18N_STRICT: "0" }, stdio: "ignore" });
    if (!(await waitForServer(BASE, 90000))) { console.error("dev server down"); process.exit(1); }
  }
  const browser = await chromium.launch();

  console.log("== prefers-reduced-motion ==");
  for (const route of ROUTES) {
    const off = await collect(browser, false, route);
    const on = await collect(browser, true, route);
    console.log(`${route}: sem preferência = ${off.totalAnims} animações (${off.movingAnims} de posição/escala); com reduce = ${on.totalAnims} animações (${on.movingAnims} de posição/escala) -> ${on.movingAnims === 0 ? "OK" : "FALHA"}`);
  }

  console.log("\n== alvos de clique (mínimos macOS adotados: 28px padrão / 20px densos) ==");
  for (const route of ROUTES) {
    const { targets } = await collect(browser, true, route);
    const badDense = targets.filter((t) => t.inRow && t.h < 20);
    const badStd = targets.filter((t) => !t.inRow && t.h < 28 && !(t.tag === "a" && t.h >= 16));
    console.log(`${route}: ${targets.length} alvos; padrão <28px: ${badStd.length}; densos <20px: ${badDense.length}`);
    for (const b of [...badStd, ...badDense].slice(0, 6)) console.log(`   - ${b.tag}.${b.cls} ${b.w}x${b.h}`);
  }

  console.log("\n== teclado (Tab x20, foco visível) ==");
  for (const route of ["/", "/settings", "/_kitchen-sink"]) {
    const seq = await keyboardPass(browser, route, 20);
    const noAffordance = seq.filter((s) => !s.focusAffordance);
    console.log(`${route}: ${seq.length} paradas de foco; sem affordance visível: ${noAffordance.length}`);
    for (const n of noAffordance.slice(0, 5)) console.log(`   - ${n.tag}.${n.cls}`);
  }

  await browser.close();
  if (devProc) devProc.kill();
}

main();
