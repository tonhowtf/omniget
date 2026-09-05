/**
 * Manual "Capture cookies from this site" flow.
 *
 * Reads cookies for the active tab's URL and ships them to the desktop app via
 * the localhost HTTP bridge (`POST /v1/cookies`). Includes metadata so the
 * desktop Cookie Manager UI can show the source URL, page title (as alias hint)
 * and label the capture as "manual".
 */

import { sendCookiesViaBridge } from "./bridge-client.js";

const TRACKED_PLATFORMS = {
  "youtube.com": "youtube",
  "youtu.be": "youtube",
  "music.youtube.com": "youtube_music",
  "soundcloud.com": "soundcloud",
  "spotify.com": "spotify",
  "twitch.tv": "twitch",
  "instagram.com": "instagram",
  "x.com": "x_twitter",
  "twitter.com": "x_twitter",
  "vimeo.com": "vimeo",
  "tiktok.com": "tiktok",
  "bilibili.com": "bilibili",
  "reddit.com": "reddit",
  "pinterest.com": "pinterest",
  "bsky.app": "bluesky",
  "bsky.social": "bluesky",
  "hotmart.com": "hotmart",
  "udemy.com": "udemy",
  "rocketseat.com.br": "rocketseat",
};

// Two-label public suffixes: the registrable name is the last three labels
// (`app.rocketseat.com.br` -> `rocketseat.com.br`). Mirrors the desktop side
// (`cookies/platform.rs`), so both file the capture under the same bucket.
const TWO_LABEL_PUBLIC_SUFFIXES = new Set([
  "com.br", "net.br", "org.br", "edu.br", "gov.br", "art.br", "eng.br",
  "co.uk", "org.uk", "ac.uk", "gov.uk", "me.uk",
  "com.au", "net.au", "org.au", "edu.au",
  "co.jp", "ne.jp", "or.jp", "ac.jp",
  "com.mx", "org.mx", "com.ar", "com.co", "com.pe", "com.cl", "com.ve", "com.uy",
  "co.za", "co.nz", "co.in", "co.kr", "co.id", "com.sg", "com.hk", "com.tw", "com.my",
  "com.tr", "com.pt", "com.es", "com.pl", "com.ua", "com.ru", "com.cn", "com.ph",
]);

export function rootDomainOf(host) {
  const h = (host || "").replace(/^\./, "").toLowerCase();
  const parts = h.split(".");
  if (parts.length < 2) return h;
  const lastTwo = parts.slice(-2).join(".");
  if (parts.length >= 3 && TWO_LABEL_PUBLIC_SUFFIXES.has(lastTwo)) {
    return parts.slice(-3).join(".");
  }
  return lastTwo;
}

export function detectPlatformKind(hostname) {
  const h = (hostname || "").toLowerCase().replace(/^\./, "");
  if (h === "music.youtube.com") return "youtube_music";
  const root = rootDomainOf(h);
  return TRACKED_PLATFORMS[root] ?? "generic";
}

function chromeCookieToExtensionCookie(cookie) {
  return {
    domain: cookie.domain,
    name: cookie.name,
    value: cookie.value,
    path: cookie.path || "/",
    secure: !!cookie.secure,
    httpOnly: !!cookie.httpOnly,
    expires: cookie.expirationDate ? Math.floor(cookie.expirationDate) : 0,
    hostOnly: !!cookie.hostOnly,
    sameSite: cookie.sameSite ?? null,
  };
}

export async function captureCookiesForTab(tab, {
  cookiesApi = globalThis.chrome?.cookies,
  send = sendCookiesViaBridge,
} = {}) {
  if (!tab?.url) {
    return { ok: false, reason: "no-url" };
  }
  let parsed;
  try {
    parsed = new URL(tab.url);
  } catch {
    return { ok: false, reason: "invalid-url" };
  }
  const proto = parsed.protocol;
  if (proto !== "http:" && proto !== "https:") {
    return { ok: false, reason: "unsupported-scheme" };
  }
  if (!cookiesApi?.getAll) {
    return { ok: false, reason: "no-cookies-api" };
  }

  const root = rootDomainOf(parsed.hostname);
  let raw;
  try {
    raw = await new Promise((resolve, reject) => {
      cookiesApi.getAll({ domain: root }, (cookies) => {
        const err = globalThis.chrome?.runtime?.lastError;
        if (err) reject(new Error(err.message));
        else resolve(cookies || []);
      });
    });
  } catch (error) {
    return { ok: false, reason: "cookies-fetch-failed", message: error?.message ?? String(error) };
  }

  if (!raw || raw.length === 0) {
    return { ok: false, reason: "no-cookies-for-domain", domain: root };
  }

  const cookies = raw.map(chromeCookieToExtensionCookie);
  const platformKind = detectPlatformKind(parsed.hostname);
  const title = (tab.title || "").trim();
  const alias = title ? `${title} (${root})` : root;

  const result = await send(cookies, {
    sourceUrl: tab.url,
    pageTitle: title || null,
    alias,
  });

  if (!result?.ok) {
    return { ok: false, reason: result?.reason ?? "send-failed", message: result?.message ?? null };
  }

  return {
    ok: true,
    domain: root,
    cookie_count: cookies.length,
    platform_kind: platformKind,
  };
}
