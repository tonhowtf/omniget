import { loadOpenAppState, isOpenAppEnabled, setOpenAppEnabled } from "../src/open-app-toggle.js";
import { getHlsGroupKey } from "../src/hls-grouping.js";
import { normalizePageKey } from "../src/sniffer-storage.js";
import { formatCookieSummary } from "../src/cookie-summary.js";
import { captureCookiesForTab } from "../src/cookie-capture.js";

const APP_URL = "https://github.com/tonhowtf/omniget/releases/latest";

const tr = (k, ...subs) =>
  (chrome.i18n?.getMessage?.(k, subs.length ? subs.map(String) : undefined) || k);

const SVG = {
  play: `<svg viewBox="0 0 20 20" fill="currentColor"><path d="M6.5 4v12l10-6z"/></svg>`,
  check: `<svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path class="checkmark-path" d="M4 10.5l4 4 8-8"/></svg>`,
  checkStatic: `<svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M4 10.5l4 4 8-8"/></svg>`,
  download: `<svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 3v10M6 9.5L10 13l4-3.5M4 16h12"/></svg>`,
  arrow: `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M8 3v8M5 8l3 3 3-3"/></svg>`,
  warning: `<svg viewBox="0 0 20 20" fill="currentColor"><path d="M10 2L1.5 17h17L10 2zm-.75 5.5h1.5v4h-1.5v-4zm.75 6.25a.75.75 0 100-1.5.75.75 0 000 1.5z"/></svg>`,
};

let currentData = null;
let pageTitle = "";
let pageThumbnail = "";

function localizeStatic() {
  const openApp = tr("popup_open_app_label");
  const sniffer = tr("popup_sniffer_label");
  const h264 = tr("popup_h264_label");
  const h264Wrap = document.getElementById("h264-toggle")?.closest(".toggle-group");
  if (h264Wrap && h264) h264Wrap.title = h264;
  if (h264) document.getElementById("h264-toggle")?.setAttribute("aria-label", h264);
  const openWrap = document.getElementById("open-app-toggle")?.closest(".toggle-group");
  if (openWrap) openWrap.title = openApp;
  document.getElementById("open-app-toggle")?.setAttribute("aria-label", openApp);
  const snifWrap = document.getElementById("sniffer-toggle")?.closest(".toggle-group");
  if (snifWrap) snifWrap.title = sniffer;
  document.getElementById("sniffer-toggle")?.setAttribute("aria-label", sniffer);
  const ckLabel = document.getElementById("cookie-capture-label");
  if (ckLabel) ckLabel.textContent = tr("popup_ck_btn");
  const ckHint = document.getElementById("cookie-capture-hint");
  if (ckHint) ckHint.textContent = tr("popup_ck_hint");
}

async function init() {
  localizeStatic();
  const toggle = document.getElementById("sniffer-toggle");
  const openAppToggle = document.getElementById("open-app-toggle");

  let activeTab = null;
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    activeTab = tab ?? null;
    pageTitle = tab?.title || "";
    pageThumbnail = tab?.favIconUrl || "";
  } catch {}

  initCookieCapture(activeTab);

  const h264Toggle = document.getElementById("h264-toggle");
  chrome.runtime.sendMessage({ type: "getH264" }, (r) => { if (r && h264Toggle) h264Toggle.checked = r.enabled; });
  h264Toggle?.addEventListener("change", () => {
    chrome.runtime.sendMessage({ type: "toggleH264", enabled: h264Toggle.checked }, (r) => { if (r && h264Toggle.checked !== r.enabled) h264Toggle.checked = r.enabled; });
  });

  await loadOpenAppState();
  openAppToggle.checked = isOpenAppEnabled();
  openAppToggle.addEventListener("change", () => {
    setOpenAppEnabled(openAppToggle.checked);
  });

  chrome.runtime.sendMessage({ type: "getDetectedMedia" }, (response) => {
    if (!response) return;
    currentData = response;
    toggle.checked = response.snifferEnabled;
    render();
  });

  toggle.addEventListener("change", () => {
    const requested = toggle.checked;
    chrome.runtime.sendMessage({ type: "toggleSniffer", enabled: requested }, (response) => {
      const effective = response?.enabled ?? requested;
      if (toggle.checked !== effective) toggle.checked = effective;
      if (currentData) {
        currentData.snifferEnabled = effective;
        render();
      }
    });
    if (currentData) {
      currentData.snifferEnabled = requested;
      render();
    }
  });

  chrome.runtime.onMessage.addListener((msg) => {
    if (msg?.type !== "media-detected") return;
    const currentPageKey = normalizePageKey(currentData?.tabUrl);
    if (!currentPageKey || currentPageKey !== msg.pageKey) return;
    chrome.runtime.sendMessage({ type: "getDetectedMedia" }, (response) => {
      if (!response) return;
      currentData = response;
      render();
    });
  });
}

function determineState(data) {
  if (!data) return "loading";
  if (data.pageDetected?.platform) return "known_platform";
  if (data.media?.length > 0) return "media_detected";
  if (!data.snifferEnabled) return "sniffer_off";
  return "listening";
}

function render() {
  const state = determineState(currentData);
  const content = document.getElementById("content");
  document.querySelector(".popup").classList.toggle("dimmed", state === "sniffer_off");
  content.innerHTML = "";

  const renderers = {
    known_platform: renderKnownPlatform,
    media_detected: renderMediaDetected,
    listening: renderListening,
    sniffer_off: renderSnifferOff,
  };

  (renderers[state] || renderListening)(content);
}

function renderKnownPlatform(container) {
  const { pageDetected, tabUrl } = currentData;
  const domain = getDomainFromUrl(tabUrl);
  const label = getDownloadLabel(pageDetected.contentType);
  const meta = domain + (pageTitle ? " \u00b7 " + truncate(pageTitle, 35) : "");

  appendPrimaryButton(container, label, meta, (btn) => {
    handleDownload(btn, tabUrl, pageDetected.platform);
  });
}

function renderMediaDetected(container) {
  const { media } = currentData;
  const best = pickBestMedia(media);
  if (!best) { renderListening(container); return; }

  const hlsGroups = groupHlsManifests(media);
  const groups = [...hlsGroups.values()].filter(g => g.master);
  const nonHls = media.filter(m => m.mediaType !== "hls");
  const directVideo = nonHls.filter(m => m.contentLength > 500 * 1024);
  const totalVideos = groups.length + directVideo.filter(m => m.mediaType === "video").length;

  const domain = getDomainFromUrl(currentData.tabUrl || best.url);
  const title = pageTitle ? truncate(pageTitle, 40) : domain;
  const countText = totalVideos > 1 ? tr("popup_videos_found", totalVideos) : tr("popup_video_found_one");
  const label = best.mediaType === "audio" ? tr("popup_send_audio") : tr("popup_send_default");

  appendPrimaryButton(container, label, title + " \u00b7 " + countText, (btn) => {
    handleDownload(btn, best.url, "generic", best);
  });

  appendMediaControls(container, media);
}

function renderListening(container) {
  container.innerHTML = `
    <div class="state-empty" role="status">
      <div class="listening-dot" aria-hidden="true"></div>
      <span class="state-title">${escapeHtml(tr("popup_listening_title"))}</span>
      <span class="state-hint">${escapeHtml(tr("popup_listening_hint"))}</span>
    </div>
  `;
}

function renderSnifferOff(container) {
  container.innerHTML = `
    <div class="state-empty" role="status">
      <span class="state-title">${escapeHtml(tr("popup_paused_title"))}</span>
      <span class="state-hint">${escapeHtml(tr("popup_paused_hint"))}</span>
    </div>
  `;
}

function appendPrimaryButton(container, label, meta, onClick) {
  const action = document.createElement("div");
  action.className = "primary-action";

  const btn = document.createElement("button");
  btn.className = "primary-btn";
  btn.setAttribute("aria-label", label);
  btn.innerHTML = `
    <span class="btn-icon">${SVG.play}</span>
    <div class="btn-content">
      <span class="btn-label">${escapeHtml(label)}</span>
      <span class="btn-meta">${escapeHtml(meta)}</span>
    </div>
  `;

  btn.addEventListener("click", () => onClick(btn));
  action.appendChild(btn);
  container.appendChild(action);
}

function appendMediaControls(container, media) {
  const hlsGroups = groupHlsManifests(media);
  const groups = [...hlsGroups.values()].filter(g => g.master);
  const nonHls = (media || []).filter(m => m.mediaType !== "hls");
  const directMedia = deduplicateMedia(nonHls.filter(m => m.contentLength > 500 * 1024)).slice(0, 10);
  const totalItems = groups.length + directMedia.length;

  if (groups.length >= 2) {
    const action = document.createElement("div");
    action.className = "secondary-action";

    const btn = document.createElement("button");
    btn.className = "secondary-btn";
    const sendAllLabel = tr("popup_send_all", groups.length);
    btn.setAttribute("aria-label", sendAllLabel);
    btn.innerHTML = `
      <span class="btn-icon">${SVG.download}</span>
      <div class="btn-content">
        <span class="btn-label">${escapeHtml(sendAllLabel)}</span>
      </div>
    `;

    btn.addEventListener("click", () => handleBatchDownload(btn, groups));
    action.appendChild(btn);
    container.appendChild(action);
  }

  if (totalItems > 1) {
    appendMediaList(container, groups, directMedia, totalItems);
  }
}

function appendMediaList(container, hlsGroups, directMedia, totalItems) {
  const section = document.createElement("div");
  section.className = "media-section";

  const toggleBtn = document.createElement("button");
  toggleBtn.className = "media-toggle";
  toggleBtn.setAttribute("aria-expanded", "false");
  toggleBtn.innerHTML = `
    <span>${escapeHtml(totalItems === 1 ? tr("popup_detected_one") : tr("popup_detected_many", totalItems))}</span>
    <span class="media-toggle-arrow">\u25be</span>
  `;

  const list = document.createElement("div");
  list.className = "media-list-collapsible";

  let idx = 1;
  for (const group of hlsGroups) {
    const domain = getDomainFromUrl(group.master.url);
    list.appendChild(createMediaItem(
      tr("popup_video_n", idx), domain,
      () => sendToApp(group.master.url, "generic", group.master)
    ));
    idx++;
  }

  for (const entry of directMedia) {
    const name = getFilenameFromUrl(entry.url);
    const size = entry.sizeText || "";
    const meta = (entry.mediaType === "audio" ? tr("popup_kind_audio") : tr("popup_kind_video")) + (size ? " \u00b7 " + size : "");
    list.appendChild(createMediaItem(name, meta, () => sendToApp(entry.url, "generic", entry)));
  }

  toggleBtn.addEventListener("click", () => {
    const expanded = list.classList.toggle("expanded");
    toggleBtn.classList.toggle("expanded", expanded);
    toggleBtn.setAttribute("aria-expanded", String(expanded));
  });

  section.appendChild(toggleBtn);
  section.appendChild(list);
  container.appendChild(section);
}

function createMediaItem(name, meta, sendFn) {
  const item = document.createElement("div");
  item.className = "media-item";
  item.innerHTML = `
    <div class="media-info">
      <div class="media-details">
        <span class="media-name">${escapeHtml(name)}</span>
        <span class="media-meta">${escapeHtml(meta)}</span>
      </div>
    </div>
    <button class="item-download-btn" aria-label="${escapeHtml(tr("popup_download_aria", name))}">${SVG.arrow}</button>
  `;

  const btn = item.querySelector(".item-download-btn");
  btn.addEventListener("click", async (e) => {
    e.stopPropagation();
    if (btn.dataset.busy) return;
    btn.dataset.busy = "1";
    const result = await sendFn();
    if (result.ok) {
      btn.innerHTML = SVG.checkStatic;
      btn.classList.add("item-success");
      setTimeout(() => window.close(), 800);
    } else {
      btn.classList.add("item-error");
      delete btn.dataset.busy;
    }
  });

  return item;
}

async function handleDownload(btn, url, platform, mediaEntry) {
  if (btn.dataset.busy) return;
  btn.dataset.busy = "1";

  btn.classList.add("sending");
  btn.querySelector(".btn-label").textContent = tr("popup_sending");
  btn.querySelector(".btn-meta").textContent = "";

  const result = await sendToApp(url, platform, mediaEntry);

  if (result.ok) {
    btn.classList.remove("sending");
    btn.classList.add("success");
    btn.querySelector(".btn-icon").innerHTML = SVG.check;
    btn.querySelector(".btn-label").textContent = tr("popup_sent");
    const summaryText = formatCookieSummary(result.cookieSummary);
    const metaEl = btn.querySelector(".btn-meta");
    if (metaEl) metaEl.textContent = summaryText;
    setTimeout(() => window.close(), summaryText ? 1600 : 1000);
  } else {
    showError(btn.closest(".primary-action"));
  }
}

async function handleBatchDownload(btn, groups) {
  if (btn.dataset.busy) return;
  btn.dataset.busy = "1";

  btn.querySelector(".btn-label").textContent = tr("popup_sending_n", groups.length);
  btn.classList.add("sending");

  const result = await sendBatch(groups);

  if (result.ok) {
    btn.classList.remove("sending");
    btn.classList.add("success");
    btn.querySelector(".btn-icon").innerHTML = SVG.check;
    btn.querySelector(".btn-label").textContent = tr("popup_sent_n", result.sent);
    setTimeout(() => window.close(), 1200);
  } else if (result.sent > 0) {
    btn.classList.remove("sending");
    btn.querySelector(".btn-label").textContent = tr("popup_sent_failed", result.sent, result.failed);
    delete btn.dataset.busy;
  } else {
    showError(btn.closest(".secondary-action") || btn.parentElement);
  }
}

function showError(container) {
  container.innerHTML = `
    <div class="error-box">
      <div class="error-header">
        <span class="error-icon">${SVG.warning}</span>
        <span class="error-message">${escapeHtml(tr("popup_not_running"))}</span>
      </div>
      <div class="error-actions">
        <button class="error-btn error-btn-primary" data-action="retry">${escapeHtml(tr("popup_try_again"))}</button>
        <button class="error-btn error-btn-secondary" data-action="open">${escapeHtml(tr("popup_get_app"))}</button>
      </div>
    </div>
  `;

  container.querySelector('[data-action="retry"]').addEventListener("click", render);
  container.querySelector('[data-action="open"]').addEventListener("click", () => {
    chrome.tabs.create({ url: APP_URL });
  });
}

function sendToApp(url, platform, mediaEntry) {
  return new Promise((resolve) => {
    const msg = { type: "sendToOmniGet", url, platform };

    if (pageTitle) msg.title = pageTitle;
    if (pageThumbnail) msg.thumbnail = pageThumbnail;
    if (currentData?.tabUrl) msg.referer = currentData.tabUrl;
    if (mediaEntry?.mediaType) msg.mediaType = mediaEntry.mediaType;
    if (mediaEntry?.contentType) msg.contentType = mediaEntry.contentType;
    msg.openApp = isOpenAppEnabled();

    if (mediaEntry?.requestHeaders) {
      const skip = ["host", "connection", "accept-encoding", "sec-ch-ua", "sec-ch-ua-mobile", "sec-ch-ua-platform", "sec-fetch-dest", "sec-fetch-mode", "sec-fetch-site", "upgrade-insecure-requests"];
      const extracted = {};
      for (const h of mediaEntry.requestHeaders) {
        const name = h.name.toLowerCase();
        if (skip.includes(name)) continue;
        if (name.startsWith("sec-")) continue;
        extracted[h.name] = h.value;
      }
      if (Object.keys(extracted).length > 0) {
        msg.headers = extracted;
      }
    }

    chrome.runtime.sendMessage(msg, (response) => {
      resolve({ ok: response?.ok ?? false, cookieSummary: response?.cookieSummary ?? null });
    });
  });
}

async function sendBatch(groups) {
  let sent = 0;
  let failed = 0;

  for (const group of groups) {
    if (!group.master) continue;

    const msg = { type: "sendToOmniGet", url: group.master.url, platform: "generic" };
    if (currentData?.tabUrl) msg.referer = currentData.tabUrl;
    if (group.master.mediaType) msg.mediaType = group.master.mediaType;
    if (group.master.contentType) msg.contentType = group.master.contentType;
    msg.openApp = isOpenAppEnabled();

    try {
      await new Promise((resolve) => {
        chrome.runtime.sendMessage(msg, (response) => {
          if (response?.ok) sent++; else failed++;
          resolve();
        });
      });
    } catch { failed++; }

    await new Promise(r => setTimeout(r, 200));
  }

  return { sent, failed, ok: failed === 0 };
}

function groupHlsManifests(media) {
  const hlsItems = (media || []).filter(m => m.mediaType === "hls");
  const groups = new Map();

  for (const item of hlsItems) {
    const filename = getFilenameFromUrl(item.url).toLowerCase();
    if (filename.includes("subtitle") || filename.includes("caption")) continue;

    const key = getHlsGroupKey(item.url);
    if (!groups.has(key)) groups.set(key, { master: null, variants: [], all: [] });

    const group = groups.get(key);
    group.all.push(item);

    if (filename.includes("master") || filename.includes("playlist") || filename === "index.m3u8") {
      group.master = item;
    } else if (!group.master) {
      group.master = item;
    }
  }

  return groups;
}

function pickBestMedia(media) {
  if (!media || media.length === 0) return null;

  const hlsGroups = groupHlsManifests(media);
  if (hlsGroups.size > 0) {
    let bestGroup = null;
    let latestTime = 0;

    for (const [, group] of hlsGroups) {
      if (!group.master) continue;
      const maxTime = Math.max(...group.all.map(m => m.detectedAt));
      if (maxTime > latestTime) {
        latestTime = maxTime;
        bestGroup = group;
      }
    }

    if (bestGroup?.master) return bestGroup.master;
  }

  const videoItems = media.filter(m => m.mediaType === "video" && m.contentLength > 500 * 1024);
  if (videoItems.length > 0) {
    return videoItems.reduce((best, m) => m.contentLength > best.contentLength ? m : best, videoItems[0]);
  }

  return null;
}

function getDownloadLabel(contentType) {
  switch (contentType) {
    case "course": return tr("popup_lbl_course");
    case "playlist": return tr("popup_lbl_playlist");
    case "video": return tr("popup_lbl_video");
    case "reel": return tr("popup_lbl_reel");
    case "post": return tr("popup_lbl_post");
    case "short": return tr("popup_lbl_short");
    case "clip": return tr("popup_lbl_clip");
    case "image": return tr("popup_lbl_image");
    case "audio": return tr("popup_lbl_audio");
    case "profile": return tr("popup_lbl_profile");
    default: return tr("popup_lbl_page");
  }
}

function deduplicateMedia(media) {
  const seen = new Map();
  for (const m of media) {
    const key = getDomainFromUrl(m.url) + "_" + m.mediaType;
    const existing = seen.get(key);
    if (!existing || m.contentLength > existing.contentLength) {
      seen.set(key, m);
    }
  }
  return [...seen.values()].sort((a, b) => b.contentLength - a.contentLength);
}

function getDomainFromUrl(url) {
  try { return new URL(url).hostname; } catch { return ""; }
}

function getFilenameFromUrl(url) {
  try {
    const path = new URL(url).pathname;
    const parts = path.split("/");
    const last = parts[parts.length - 1];
    if (last && last.includes(".")) return decodeURIComponent(last).substring(0, 50);
    return url.substring(0, 60) + "\u2026";
  } catch { return url.substring(0, 60) + "\u2026"; }
}

function truncate(str, max) {
  return str.length > max ? str.substring(0, max) + "\u2026" : str;
}

function escapeHtml(str) {
  const d = document.createElement("div");
  d.textContent = str;
  return d.innerHTML;
}

function initCookieCapture(activeTab) {
  const wrap = document.getElementById("cookie-capture");
  const btn = document.getElementById("cookie-capture-btn");
  const label = document.getElementById("cookie-capture-label");
  const hint = document.getElementById("cookie-capture-hint");
  if (!wrap || !btn || !label) return;

  const url = activeTab?.url ?? "";
  let isHttpish = false;
  try {
    const p = new URL(url);
    isHttpish = p.protocol === "http:" || p.protocol === "https:";
  } catch {}

  if (!isHttpish) {
    wrap.dataset.state = "hidden";
    return;
  }

  let busy = false;
  btn.addEventListener("click", async () => {
    if (busy) return;
    busy = true;
    btn.disabled = true;
    btn.removeAttribute("data-state");
    label.textContent = tr("popup_ck_saving");

    const result = await captureCookiesForTab(activeTab);
    busy = false;
    btn.disabled = false;

    if (result.ok) {
      btn.dataset.state = "success";
      label.textContent = tr("popup_ck_saved", result.cookie_count);
      if (hint) hint.textContent = tr("popup_ck_meta", result.domain, result.platform_kind.replace("_", " "));
      setTimeout(() => {
        btn.removeAttribute("data-state");
        label.textContent = tr("popup_ck_btn");
        if (hint) hint.textContent = tr("popup_ck_hint");
      }, 3500);
    } else {
      btn.dataset.state = "error";
      label.textContent = explainFailure(result);
      setTimeout(() => {
        btn.removeAttribute("data-state");
        label.textContent = tr("popup_ck_btn");
      }, 4000);
    }
  });
}

function explainFailure(result) {
  switch (result?.reason) {
    case "no-url": return tr("popup_ckerr_no_url");
    case "invalid-url": return tr("popup_ckerr_invalid_url");
    case "unsupported-scheme": return tr("popup_ckerr_unsupported");
    case "no-cookies-api": return tr("popup_ckerr_no_api");
    case "no-cookies-for-domain": return tr("popup_ckerr_no_cookies");
    case "missing-token": return tr("popup_ckerr_missing_token");
    case "missing-endpoint": return tr("popup_ckerr_missing_endpoint");
    case "fetch-failed": return tr("popup_ckerr_fetch_failed");
    case "unauthorized": return tr("popup_ckerr_unauthorized");
    default: return tr("popup_ckerr_default");
  }
}

init();
