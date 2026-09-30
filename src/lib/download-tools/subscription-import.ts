/**
 * Reads a subscriptions export from another app and turns it into channel
 * URLs that OmniGet can follow (Settings › Channels).
 *
 * Formats, detected from the content rather than the file name:
 * - FreeTube `.db` (one JSON profile per line, and its older one-channel-per-line form)
 * - NewPipe / FreeTube "NewPipe" `.json` (`{ subscriptions: [{ url, name }] }`)
 * - YouTube `.json` (`[{ snippet: { resourceId: { channelId }, title } }]`)
 * - YouTube Takeout / FreeTube `.csv` (`Channel Id,Channel Url,Channel Title`)
 * - OPML / RSS feed lists (`<outline xmlUrl="…feeds/videos.xml?channel_id=…">`)
 * - Plain text: one channel or playlist URL per line
 *
 * The formats are public; this is a fresh implementation, no code was copied.
 */

export type SubscriptionFormat = "freetube" | "newpipe" | "youtube-json" | "csv" | "opml" | "urls";

export interface ImportedChannel {
  /** The URL OmniGet follows. YouTube channels point at their Videos tab. */
  url: string;
  title: string;
}

export interface SubscriptionImport {
  format: SubscriptionFormat;
  channels: ImportedChannel[];
  /** Entries that were recognised as subscriptions but had no usable channel. */
  skipped: number;
}

const CHANNEL_ID = /^UC[\w-]{22}$/;

/** The Videos tab: a bare channel URL lists its tabs, not its uploads. */
export function youtubeChannelUrl(channelId: string): string {
  return `https://www.youtube.com/channel/${channelId}/videos`;
}

/** Normalises a channel URL from an export; YouTube channel pages get the Videos tab. */
export function channelUrlFrom(raw: string): string | null {
  const value = raw.trim();
  if (CHANNEL_ID.test(value)) return youtubeChannelUrl(value);
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  const host = url.hostname.replace(/^(www|m)\./, "");
  if (host === "youtube.com") {
    const feedId = url.searchParams.get("channel_id");
    if (url.pathname.startsWith("/feeds/") && feedId && CHANNEL_ID.test(feedId)) {
      return youtubeChannelUrl(feedId);
    }
    const parts = url.pathname.split("/").filter(Boolean);
    if (parts[0] === "channel" && parts[1] && CHANNEL_ID.test(parts[1])) return youtubeChannelUrl(parts[1]);
    if (parts[0]?.startsWith("@") && parts.length <= 2) return `https://www.youtube.com/${parts[0]}/videos`;
    if ((parts[0] === "c" || parts[0] === "user") && parts[1]) {
      return `https://www.youtube.com/${parts[0]}/${parts[1]}/videos`;
    }
    if (parts[0] === "playlist" && url.searchParams.get("list")) {
      return `https://www.youtube.com/playlist?list=${url.searchParams.get("list")}`;
    }
  }
  // Invidious feed links (`/feed/channel/UC…`) from FreeTube's OPML variant.
  const invidious = url.pathname.match(/\/feed\/channel\/(UC[\w-]{22})\/?$/);
  if (invidious) return youtubeChannelUrl(invidious[1]);
  return url.toString();
}

function push(out: Map<string, ImportedChannel>, rawUrl: string | null | undefined, title: unknown): boolean {
  const url = rawUrl ? channelUrlFrom(rawUrl) : null;
  if (!url) return false;
  const name = typeof title === "string" && title.trim() ? title.trim() : url;
  if (!out.has(url)) out.set(url, { url, title: name });
  return true;
}

function parseCsvLine(line: string): string[] {
  const cells: string[] = [];
  let cell = "";
  let quoted = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (quoted) {
      if (c === '"' && line[i + 1] === '"') {
        cell += '"';
        i++;
      } else if (c === '"') quoted = false;
      else cell += c;
    } else if (c === '"') quoted = true;
    else if (c === ",") {
      cells.push(cell);
      cell = "";
    } else cell += c;
  }
  cells.push(cell);
  return cells.map((v) => v.trim());
}

function decodeXml(value: string): string {
  return value
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&#(\d+);/g, (_, n) => String.fromCodePoint(Number(n)))
    .replace(/&amp;/g, "&");
}

function attr(tag: string, name: string): string | null {
  const m = tag.match(new RegExp(`\\s${name}\\s*=\\s*("([^"]*)"|'([^']*)')`, "i"));
  return m ? decodeXml(m[2] ?? m[3] ?? "") : null;
}

function fromJson(value: unknown, out: Map<string, ImportedChannel>): { format: SubscriptionFormat; skipped: number } | null {
  let skipped = 0;
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const subs = (value as { subscriptions?: unknown }).subscriptions;
    if (!Array.isArray(subs)) return null;
    // A FreeTube profile has `{ id, name }` entries; NewPipe has `{ url, name, service_id }`.
    const freetube = subs.some((s) => s && typeof s === "object" && "id" in s && !("url" in s));
    for (const s of subs as Record<string, unknown>[]) {
      if (!s || typeof s !== "object") continue;
      // NewPipe also exports SoundCloud, PeerTube and Bandcamp channels; their URLs are kept as they are.
      const ok = push(out, (typeof s.url === "string" ? s.url : typeof s.id === "string" ? s.id : null) as string | null, s.name);
      if (!ok) skipped++;
    }
    return { format: freetube ? "freetube" : "newpipe", skipped };
  }
  if (Array.isArray(value)) {
    const looksYouTube = value.some((v) => v && typeof v === "object" && "snippet" in v);
    if (!looksYouTube) return null;
    for (const v of value as { snippet?: { resourceId?: { channelId?: string }; title?: string } }[]) {
      if (!push(out, v?.snippet?.resourceId?.channelId, v?.snippet?.title)) skipped++;
    }
    return { format: "youtube-json", skipped };
  }
  return null;
}

/** Parses an export. Returns null when nothing in the text looks like a subscription list. */
export function parseSubscriptions(text: string): SubscriptionImport | null {
  const body = text.replace(/^﻿/, "").trim();
  if (!body) return null;
  const out = new Map<string, ImportedChannel>();

  // One JSON document (NewPipe, YouTube JSON, a single FreeTube profile).
  if (body.startsWith("{") || body.startsWith("[")) {
    try {
      const parsed = fromJson(JSON.parse(body), out);
      if (parsed && out.size) return { format: parsed.format, channels: [...out.values()], skipped: parsed.skipped };
    } catch {
      // Not one document: a FreeTube .db has one JSON object per line.
    }
  }

  // FreeTube .db: one profile per line; the older format had one channel per line.
  if (body.startsWith("{")) {
    let skipped = 0;
    let lines = 0;
    for (const line of body.split(/\r?\n/)) {
      if (!line.trim()) continue;
      let row: Record<string, unknown>;
      try {
        row = JSON.parse(line);
      } catch {
        continue;
      }
      lines++;
      if (Array.isArray(row.subscriptions)) {
        for (const s of row.subscriptions as Record<string, unknown>[]) {
          if (!push(out, typeof s?.id === "string" ? s.id : null, s?.name)) skipped++;
        }
      } else if (typeof row.channelId === "string") {
        if (!push(out, row.channelId, row.channelName)) skipped++;
      }
    }
    if (lines && out.size) return { format: "freetube", channels: [...out.values()], skipped };
  }

  // OPML / RSS feed list.
  if (/<outline[\s>]/i.test(body)) {
    let skipped = 0;
    for (const tag of body.match(/<outline\b[^>]*>/gi) ?? []) {
      const feed = attr(tag, "xmlUrl");
      if (!feed) continue; // a folder outline
      if (!push(out, feed, attr(tag, "title") ?? attr(tag, "text"))) skipped++;
    }
    if (out.size) return { format: "opml", channels: [...out.values()], skipped };
  }

  // CSV with a Channel Id / Channel Url column (YouTube Takeout and FreeTube's CSV export).
  const lines = body.split(/\r?\n/).filter((l) => l.trim());
  const header = parseCsvLine(lines[0] ?? "").map((h) => h.toLowerCase());
  const idCol = header.findIndex((h) => h === "channel id");
  const urlCol = header.findIndex((h) => h === "channel url");
  const titleCol = header.findIndex((h) => h === "channel title");
  if (idCol >= 0 || urlCol >= 0) {
    let skipped = 0;
    for (const line of lines.slice(1)) {
      const cells = parseCsvLine(line);
      const raw = (idCol >= 0 && cells[idCol]) || (urlCol >= 0 && cells[urlCol]) || null;
      if (!push(out, raw, titleCol >= 0 ? cells[titleCol] : null)) skipped++;
    }
    if (out.size) return { format: "csv", channels: [...out.values()], skipped };
  }

  // Plain list of URLs.
  let skipped = 0;
  for (const line of lines) {
    const value = line.trim();
    if (value.startsWith("#")) continue;
    if (!/^https?:\/\//i.test(value) && !CHANNEL_ID.test(value)) {
      skipped++;
      continue;
    }
    if (!push(out, value, null)) skipped++;
  }
  return out.size ? { format: "urls", channels: [...out.values()], skipped } : null;
}
