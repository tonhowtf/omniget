import { isUrl } from "./omnibox-controller";

// Links pasted into the home box: every link in the text, cleaned of tracking
// parameters, unwrapped from redirect pages and counted once even when the same
// video comes in two shapes (youtu.be/ID and watch?v=ID&si=...). The idea comes
// from Arroxy's paste step (ClearURLs catalog, bulk-URL parser); this is a
// small rule list of our own, no code was copied.

const HTTP_LINK = /https?:\/\/[^\s,;|<>"'`]+/gi;
const TRAILING = /[.,;:!?]+$/;

// Query parameters that only track where a click came from. Any site.
const TRACKING_ANYWHERE = /^(utm_[a-z_]+|fbclid|gclid|dclid|gbraid|wbraid|msclkid|mc_eid|yclid|igsh|igshid|_hsenc|_hsmi|mkt_tok)$/i;
// Parameters that track only on these sites; elsewhere the same name can matter.
const TRACKING_BY_HOST: [RegExp, RegExp][] = [
  [/(^|\.)(youtube\.com|youtu\.be)$/i, /^(si|pp|feature|ab_channel)$/i],
  [/(^|\.)(x\.com|twitter\.com)$/i, /^(s|t|ref_src|ref_url)$/i],
  [/(^|\.)instagram\.com$/i, /^(img_index|utm_source)$/i],
  [/(^|\.)tiktok\.com$/i, /^(is_from_webapp|sender_device|web_id|_r|_t)$/i],
  [/(^|\.)(spotify\.com|soundcloud\.com)$/i, /^(si|in|ref|context)$/i],
  [/(^|\.)reddit\.com$/i, /^(share_id|rdt|ref|ref_source)$/i],
];
// Pages that only forward to another address given in a parameter.
const REDIRECTS: [RegExp, RegExp, string][] = [
  [/(^|\.)youtube\.com$/i, /^\/redirect$/, "q"],
  [/^l\.facebook\.com$|^lm\.facebook\.com$/i, /^\/l\.php$/, "u"],
  [/^l\.instagram\.com$/i, /^\/$/, "u"],
  [/(^|\.)google\.[a-z.]+$/i, /^\/url$/, "q"],
  [/^out\.reddit\.com$/i, /./, "url"],
];

function parse(raw: string): URL | null {
  try {
    const url = new URL(raw);
    return url.protocol === "http:" || url.protocol === "https:" ? url : null;
  } catch {
    return null;
  }
}

function trimToken(raw: string): string {
  let s = raw.replace(TRAILING, "");
  // A closing bracket belongs to the link only when the link opened one.
  while (/[)\]]$/.test(s)) {
    const close = s.at(-1) as ")" | "]";
    const open = close === ")" ? "(" : "[";
    if (s.split(open).length >= s.split(close).length) break;
    s = s.slice(0, -1).replace(TRAILING, "");
  }
  return s;
}

/** The address with tracking parameters removed and redirect pages unwrapped. */
export function cleanLink(raw: string): string {
  const first = parse(raw);
  if (!first) return raw;
  let url: URL = first;
  for (let hops = 0; hops < 3; hops++) {
    const current: URL = url;
    const rule = REDIRECTS.find(([host, path]) => host.test(current.hostname) && path.test(current.pathname));
    const target: URL | null = rule ? parse(current.searchParams.get(rule[2]) ?? "") : null;
    if (!target) break;
    url = target;
  }
  const host = url.hostname;
  const byHost = TRACKING_BY_HOST.filter(([pattern]) => pattern.test(host)).map(([, names]) => names);
  for (const name of [...url.searchParams.keys()]) {
    if (TRACKING_ANYWHERE.test(name) || byHost.some((names) => names.test(name))) {
      url.searchParams.delete(name);
    }
  }
  const out = url.toString();
  // `new URL` leaves a bare "?" once every parameter is gone.
  return out.endsWith("?") ? out.slice(0, -1) : out;
}

/**
 * One key per piece of media, so two shapes of the same link count once:
 * youtu.be/ID, m.youtube.com, music.youtube.com and /shorts/ID all become the
 * watch page of ID (a playlist or a start time stays part of the key).
 */
export function linkKey(raw: string): string {
  const url = parse(raw);
  if (!url) return raw;
  const host = url.hostname.replace(/^(www|m|music)\./i, "").toLowerCase();
  let id: string | null = null;
  if (host === "youtu.be") id = url.pathname.slice(1).split("/")[0] || null;
  else if (host === "youtube.com") {
    id = url.searchParams.get("v") ?? url.pathname.match(/^\/(?:shorts|live|embed)\/([^/]+)/)?.[1] ?? null;
  }
  if (id) {
    const list = url.searchParams.get("list");
    const t = url.searchParams.get("t") ?? url.searchParams.get("start");
    return `youtube:${id}${list ? `&list=${list}` : ""}${t ? `&t=${t}` : ""}`;
  }
  url.hash = "";
  return `${url.protocol}//${host}${url.pathname.replace(/\/+$/, "")}${url.search}`;
}

/**
 * Every link in pasted text, in order, cleaned and without repeats. Magnet
 * links, `.torrent` paths and p2p codes pass through untouched.
 */
export function linksFromPaste(text: string): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  const add = (link: string, key: string) => {
    if (seen.has(key)) return;
    seen.add(key);
    out.push(link);
  };
  for (const token of text.split(/\s+/)) {
    if (!token) continue;
    if (/^https?:\/\//i.test(token) || token.includes("http")) {
      for (const match of token.matchAll(HTTP_LINK)) {
        const link = cleanLink(trimToken(match[0]));
        if (parse(link)) add(link, linkKey(link));
      }
    } else if (isUrl(token)) {
      add(token, token);
    }
  }
  return out;
}
