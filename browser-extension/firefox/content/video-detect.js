// OmniGet in-page download button (IDM-style).
//
// Shows a floating "Download" button only when this tab actually has a
// downloadable media object (a real <video>/<audio> player, or a known embed)
// and parks the button directly under that object. Tabs with no media — search
// pages, homepages, settings, articles without a player — stay clean.
// asks the desktop app (through the background service worker → local bridge)
// for the list of available resolutions and renders an in-page picker; choosing
// a resolution starts the download immediately at that quality.
//
// Everything lives inside a Shadow DOM so the host page's CSS can't touch it
// and vice versa. The script runs in the top frame only.
//
// IMPORTANT: the whole UI is built with DOM APIs (createElement/textContent) and
// never assigns innerHTML. Sites like YouTube ship a Trusted Types CSP
// (`require-trusted-types-for 'script'`) that throws on raw innerHTML strings,
// which would silently break the button on exactly the pages we care about.

(() => {
  "use strict";

  if (window.top !== window) return; // top frame only
  if (window.__omnigetIdmLoaded) return;
  window.__omnigetIdmLoaded = true;

  const NS_SVG = "http://www.w3.org/2000/svg";

  let host, shadow, fab, panel;
  let panelOpen = false;
  let currentUrl = location.href;
  let posPending = false;

  function isPreviewChrome(el) {
    return Boolean(
      el.closest(
        [
          "ytd-video-preview",
          "ytd-moving-thumbnail-renderer",
          "ytd-thumbnail",
          ".ytp-inline-preview-ui",
          ".html5-video-player.unstarted-mode",
          "[class*='inline-preview']",
          "[class*='hover-preview']",
        ].join(",")
      )
    );
  }

  function hasMediaSource(el) {
    if (el.currentSrc || el.src) return true;
    if (el.querySelector && el.querySelector("source[src]")) return true;
    return el.readyState > 0;
  }

  function playerBox(el) {
    const wrap =
      el.closest("#movie_player") ||
      el.closest(".html5-video-player") ||
      el.closest("[data-testid='videoPlayer']") ||
      el.closest("figure") ||
      el.parentElement;
    const box = wrap && wrap.getBoundingClientRect().height >= 40 ? wrap : el;
    return box.getBoundingClientRect();
  }

  // The actual thing the user would download: the playing player if there is
  // one, otherwise the largest real <video>/<audio>, otherwise a known embed.
  // Homepage shells, hover previews and ads do not count.
  function findDownloadTarget() {
    const media = document.querySelectorAll("video, audio");
    const scored = [];
    for (const el of media) {
      if (isPreviewChrome(el)) continue;
      if (!hasMediaSource(el)) continue;
      const r = playerBox(el);
      const isAudio = el.tagName === "AUDIO";
      const minW = isAudio ? 80 : 280;
      const minH = isAudio ? 12 : 160;
      if (r.width < minW || r.height < minH) continue;
      const playing = !el.paused && !el.ended && el.readyState > 1;
      scored.push({
        el,
        rect: r,
        playing,
        area: r.width * r.height,
      });
    }
    if (scored.length) {
      scored.sort((a, b) => {
        if (a.playing !== b.playing) return a.playing ? -1 : 1;
        return b.area - a.area;
      });
      return scored[0];
    }

    const embeds = document.querySelectorAll("iframe[src]");
    for (const frame of embeds) {
      let src = "";
      try {
        src = frame.src || "";
      } catch {
        continue;
      }
      if (
        !/(youtube\.com\/embed|youtube-nocookie\.com\/embed|player\.vimeo\.com|player\.twitch\.tv|tiktok\.com\/embed|dailymotion\.com\/embed|facebook\.com\/plugins\/video)/i.test(
          src
        )
      ) {
        continue;
      }
      const r = frame.getBoundingClientRect();
      if (r.width < 280 || r.height < 160) continue;
      return { el: frame, rect: r, playing: true, area: r.width * r.height };
    }
    return null;
  }

  // ── DOM builders (no innerHTML) ───────────────────────────────────────────

  function el(tag, props, children) {
    const node = document.createElement(tag);
    if (props) {
      for (const key in props) {
        const value = props[key];
        if (value == null) continue;
        if (key === "class") node.className = value;
        else if (key === "text") node.textContent = value;
        else if (key.startsWith("on") && typeof value === "function")
          node.addEventListener(key.slice(2), value);
        else node.setAttribute(key, value);
      }
    }
    if (children) {
      for (const child of children) {
        if (child == null) continue;
        node.appendChild(
          typeof child === "string" ? document.createTextNode(child) : child
        );
      }
    }
    return node;
  }

  function svgIcon(paths, { size = 20, strokeWidth = 2, spin = false } = {}) {
    const svg = document.createElementNS(NS_SVG, "svg");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("width", String(size));
    svg.setAttribute("height", String(size));
    svg.setAttribute("fill", "none");
    svg.setAttribute("stroke", "currentColor");
    svg.setAttribute("stroke-width", String(strokeWidth));
    svg.setAttribute("stroke-linecap", "round");
    svg.setAttribute("stroke-linejoin", "round");
    if (spin) svg.setAttribute("class", "og-spin");
    for (const d of paths) {
      const p = document.createElementNS(NS_SVG, "path");
      p.setAttribute("d", d);
      svg.appendChild(p);
    }
    return svg;
  }

  // Fresh nodes each call — appending an SVG moves it, so it can't be reused.
  function iconDownload(size = 20) {
    return svgIcon(["M12 3v12", "m7 11 5 5 5-5", "M5 21h14"], { size });
  }
  function iconSpinner(size = 18) {
    return svgIcon(["M12 3a9 9 0 1 0 9 9"], {
      size,
      strokeWidth: 2.5,
      spin: true,
    });
  }

  function replaceChildren(node, ...kids) {
    while (node.firstChild) node.removeChild(node.firstChild);
    for (const k of kids) if (k != null) node.appendChild(k);
  }

  // ── UI ────────────────────────────────────────────────────────────────

  function ensureRoot() {
    if (host) {
      if (!host.isConnected) {
        const parent = document.documentElement || document.body;
        if (parent) parent.appendChild(host);
      }
      return;
    }
    try {
      host = document.createElement("div");
      host.id = "omniget-idm-root";
      host.style.cssText =
        "position:fixed;z-index:2147483647;top:0;left:0;display:none;pointer-events:none;";
      shadow = host.attachShadow({ mode: "open" });

    const style = document.createElement("style");
    style.textContent = `
      :host { all: initial; }
      * { box-sizing: border-box; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; }
      .fab {
        display: flex; align-items: center; gap: 8px;
        background: #F04E23; color: #fff; border: none;
        padding: 10px 14px; border-radius: 999px; cursor: pointer;
        box-shadow: 0 6px 20px rgba(0,0,0,.28);
        font-size: 14px; font-weight: 600; line-height: 1;
        transition: transform .15s ease, box-shadow .15s ease, opacity .2s ease;
        user-select: none; pointer-events: auto;
      }
      .fab:hover { transform: translateY(-1px); box-shadow: 0 8px 26px rgba(0,0,0,.34); }
      .fab:active { transform: translateY(0); }
      .fab .label { white-space: nowrap; }
      .panel {
        position: absolute; top: 48px; right: auto; left: 0; bottom: auto;
        min-width: 240px; max-width: 320px;
        background: #1c1c1e; color: #fff; border-radius: 14px;
        box-shadow: 0 12px 40px rgba(0,0,0,.45);
        overflow: hidden; opacity: 0; transform: translateY(-8px) scale(.98);
        transition: opacity .16s ease, transform .16s ease;
        pointer-events: none;
      }
      .panel.open { opacity: 1; transform: translateY(0) scale(1); pointer-events: auto; }
      .panel.drop-up { top: auto; bottom: 48px; transform: translateY(8px) scale(.98); }
      .panel.drop-up.open { transform: translateY(0) scale(1); }
      .panel-header {
        padding: 12px 14px; font-size: 13px; font-weight: 600;
        border-bottom: 1px solid rgba(255,255,255,.08);
        display: flex; align-items: center; gap: 8px;
        color: #fff;
      }
      .panel-title {
        overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
        opacity: .95;
      }
      .panel-body { max-height: 320px; overflow-y: auto; padding: 6px; }
      .row {
        display: flex; align-items: center; justify-content: space-between;
        gap: 10px; width: 100%; text-align: left;
        background: transparent; color: #fff; border: none;
        padding: 10px 12px; border-radius: 10px; cursor: pointer;
        font-size: 14px;
      }
      .row:hover { background: rgba(255,255,255,.10); }
      .row .q { font-weight: 600; }
      .row .meta { font-size: 12px; opacity: .6; }
      .state { padding: 16px 14px; font-size: 13px; opacity: .85; display:flex; align-items:center; gap:10px; }
      .state.err { color: #ff9a8b; }
      .toast {
        position: absolute; top: 48px; left: 0; right: auto; bottom: auto;
        background: #1c1c1e; color: #fff; border-radius: 12px;
        padding: 10px 14px; font-size: 13px; font-weight: 500;
        box-shadow: 0 12px 40px rgba(0,0,0,.45); white-space: nowrap;
        opacity: 0; transform: translateY(8px); transition: opacity .18s ease, transform .18s ease;
        pointer-events: none;
      }
      .toast.show { opacity: 1; transform: translateY(0); }
      .toast.ok::before { content: "✓ "; color: #4ade80; }
      .toast.bad::before { content: "⚠ "; color: #fbbf24; }
      .og-spin { animation: ogspin 0.8s linear infinite; transform-origin: 12px 12px; }
      @keyframes ogspin { to { transform: rotate(360deg); } }
    `;
    shadow.appendChild(style);

    fab = el("button", { class: "fab", type: "button", onclick: onFabClick });
    setFabLoading(false);
    shadow.appendChild(fab);

    panel = el("div", { class: "panel" });
    shadow.appendChild(panel);

    document.documentElement.appendChild(host);
    } catch (err) {
      host = null;
      shadow = null;
      fab = null;
      panel = null;
      console.warn("[OmniGet] in-page button failed to mount", err);
    }
  }

  function positionHost() {
    if (!host) return;
    const target = findDownloadTarget();
    if (!target) {
      host.style.display = "none";
      return;
    }
    const r = target.rect;
    const gap = 8;
    const btnH = 44;
    let top = r.bottom + gap;
    let dropUp = false;
    if (top + btnH > window.innerHeight - 8) {
      top = Math.max(8, r.bottom - btnH - gap);
      dropUp = true;
    }
    if (top < 8) top = 8;
    const left = Math.max(8, Math.min(r.left, window.innerWidth - 160));
    host.style.display = "block";
    host.style.top = `${Math.round(top)}px`;
    host.style.left = `${Math.round(left)}px`;
    host.style.right = "auto";
    host.style.bottom = "auto";
    if (panel) panel.classList.toggle("drop-up", dropUp);
  }

  function schedulePosition() {
    if (posPending) return;
    posPending = true;
    requestAnimationFrame(() => {
      posPending = false;
      update();
    });
  }

  function showToast(text, kind) {
    const t = el("div", { class: `toast ${kind || ""}`, text });
    shadow.appendChild(t);
    requestAnimationFrame(() => t.classList.add("show"));
    setTimeout(() => {
      t.classList.remove("show");
      setTimeout(() => t.remove(), 300);
    }, 3200);
  }

  function closePanel() {
    panelOpen = false;
    if (panel) panel.classList.remove("open");
  }

  function openPanel() {
    panelOpen = true;
    panel.classList.add("open");
  }

  function renderState(text, isError) {
    const state = el("div", { class: `state ${isError ? "err" : ""}` }, [
      iconSpinner(),
      el("span", { text }),
    ]);
    replaceChildren(panel, state);
    openPanel();
  }

  function setFabLoading(loading) {
    if (!fab) return;
    replaceChildren(
      fab,
      loading ? iconSpinner() : iconDownload(),
      el("span", { class: "label", text: loading ? "Loading…" : "Download" })
    );
  }

  function renderQualities(data) {
    const title = data.title || "Available formats";
    const qualities = Array.isArray(data.qualities) ? data.qualities : [];

    const body = el("div", { class: "panel-body" });
    if (qualities.length === 0) {
      body.appendChild(rowEl({ value: "best", label: "Best available", meta: "" }));
    } else {
      for (const q of qualities) {
        const meta = q.width && q.height ? `${q.width}×${q.height}` : q.format || "";
        body.appendChild(rowEl({ value: q.value, label: q.label, meta }));
      }
      body.appendChild(
        rowEl({ value: "best", label: "Best available", meta: "auto" })
      );
    }

    const header = el("div", { class: "panel-header" }, [
      iconDownload(18),
      el("span", { class: "panel-title", text: title }),
    ]);

    replaceChildren(panel, header, body);
    openPanel();
  }

  function rowEl({ value, label, meta }) {
    return el(
      "button",
      { class: "row", type: "button", onclick: () => startDownload(value) },
      [
        el("span", { class: "q", text: label }),
        el("span", { class: "meta", text: meta || "" }),
      ]
    );
  }

  // ── Actions ─────────────────────────────────────────────────────────────

  async function onFabClick() {
    if (panelOpen) {
      closePanel();
      return;
    }
    setFabLoading(true);
    renderState("Fetching available resolutions…");
    try {
      const res = await sendMessage({
        type: "getFormats",
        url: location.href,
        openApp: false,
      });
      if (res && res.ok) {
        renderQualities(res);
      } else {
        renderState(errorText(res), true);
      }
    } catch (e) {
      renderState("Couldn't reach the OmniGet app. Is it running?", true);
    } finally {
      setFabLoading(false);
    }
  }

  async function startDownload(quality) {
    renderState("Starting download…");
    try {
      const res = await sendMessage({
        type: "downloadWithQuality",
        url: location.href,
        quality,
        openApp: false,
      });
      closePanel();
      if (res && res.ok) {
        showToast(`Download started (${quality})`, "ok");
      } else {
        showToast(shortError(res), "bad");
      }
    } catch {
      closePanel();
      showToast("OmniGet app not reachable", "bad");
    }
  }

  function errorText(res) {
    if (!res) return "No response from the OmniGet app.";
    if (
      res.reason === "missing-token" ||
      res.reason === "unauthorized" ||
      res.reason === "missing-endpoint" ||
      res.reason === "fetch-failed" ||
      res.reason === "app-not-running" ||
      res.reason === "window-closed"
    ) {
      return "OmniGet isn't running. Start the app once — it can stay in the tray.";
    }
    return res.message || "Couldn't read the available formats.";
  }

  function shortError(res) {
    if (!res) return "Download failed";
    if (
      res.reason === "missing-token" ||
      res.reason === "unauthorized" ||
      res.reason === "app-not-running" ||
      res.reason === "fetch-failed"
    ) {
      return "Start OmniGet and try again";
    }
    return res.error || res.message || "Download failed";
  }

  // ── Helpers ─────────────────────────────────────────────────────────────

  function sendMessage(msg) {
    return new Promise((resolve, reject) => {
      try {
        chrome.runtime.sendMessage(msg, (response) => {
          const err = chrome.runtime.lastError;
          if (err) return reject(new Error(err.message));
          resolve(response);
        });
      } catch (e) {
        reject(e);
      }
    });
  }

  // ── Visibility management ────────────────────────────────────────────────

  function update() {
    if (findDownloadTarget()) {
      ensureRoot();
      positionHost();
    } else if (host) {
      host.style.display = "none";
      closePanel();
    }
  }

  // React to DOM changes (videos loading in), throttled.
  let pending = false;
  const observer = new MutationObserver(() => {
    if (pending) return;
    pending = true;
    setTimeout(() => {
      pending = false;
      update();
    }, 500);
  });

  // SPA route changes (YouTube etc.) don't reload the page — watch the URL.
  setInterval(() => {
    if (location.href !== currentUrl) {
      currentUrl = location.href;
      closePanel();
    }
    update();
  }, 1000);

  // Close the panel when clicking elsewhere on the page.
  document.addEventListener(
    "click",
    (e) => {
      if (!panelOpen) return;
      if (host && e.composedPath && e.composedPath().includes(host)) return;
      closePanel();
    },
    true
  );

  window.addEventListener("scroll", schedulePosition, true);
  window.addEventListener("resize", schedulePosition);
  document.addEventListener("play", schedulePosition, true);
  document.addEventListener("pause", schedulePosition, true);
  document.addEventListener("loadedmetadata", schedulePosition, true);

  function start() {
    update();
    observer.observe(document.documentElement, {
      childList: true,
      subtree: true,
    });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start, { once: true });
  } else {
    start();
  }
})();
