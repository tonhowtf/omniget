import type { DcAttachment, DcChannel, DcGuild, DcMessage, DcReadState, DcUser } from "./api";

export const EPOCH = 1420070400000n;
const CDN = "https://cdn.discordapp.com";

export function snowflakeMs(id: string | null | undefined): number {
  if (!id || !/^\d+$/.test(id)) return 0;
  return Number((BigInt(id) >> 22n) + EPOCH);
}

export function snowflakeCmp(a: string | null | undefined, b: string | null | undefined): number {
  const x = a ?? "";
  const y = b ?? "";
  if (x.length !== y.length) return x.length - y.length;
  return x < y ? -1 : x > y ? 1 : 0;
}

export function displayName(u: DcUser | null | undefined): string {
  if (!u) return "";
  return (u.global_name && u.global_name.trim()) || u.username || u.id;
}

export function avatarUrl(u: DcUser | null | undefined, size = 64): string | null {
  if (!u?.id || !u.avatar) return null;
  const ext = u.avatar.startsWith("a_") ? "gif" : "webp";
  return `${CDN}/avatars/${u.id}/${u.avatar}.${ext}?size=${size}`;
}

export function guildIconUrl(g: DcGuild, size = 64): string | null {
  if (!g.icon) return null;
  return `${CDN}/icons/${g.id}/${g.icon}.webp?size=${size}`;
}

export function dmIconUrl(c: DcChannel, size = 64): string | null {
  if (c.kind === 3) return c.icon ? `${CDN}/channel-icons/${c.id}/${c.icon}.webp?size=${size}` : null;
  return avatarUrl(c.recipients[0], size);
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (!parts.length) return "?";
  return parts
    .slice(0, 2)
    .map((p) => Array.from(p)[0] ?? "")
    .join("")
    .toUpperCase();
}

export function isUnread(c: DcChannel, rs: DcReadState | undefined): boolean {
  if (!c.last_message_id || c.kind === 4) return false;
  if (!rs || !rs.last_message_id) return false;
  return snowflakeCmp(c.last_message_id, rs.last_message_id) > 0;
}

export function mentionsMe(m: DcMessage, meId: string): boolean {
  if (m.author.id === meId) return false;
  return m.mention_everyone || m.mentions.some((u) => u.id === meId) || m.referenced_message?.author.id === meId;
}

export function isImage(a: DcAttachment): boolean {
  if (a.content_type) return a.content_type.startsWith("image/");
  return /\.(png|jpe?g|gif|webp|avif)$/i.test(a.filename);
}

export function isVideo(a: DcAttachment): boolean {
  if (a.content_type) return a.content_type.startsWith("video/");
  return /\.(mp4|webm|mov)$/i.test(a.filename);
}

export function fitBox(w: number | null | undefined, h: number | null | undefined, maxW = 400, maxH = 300): { width: number; height: number } {
  if (!w || !h) return { width: maxW, height: Math.round(maxW * 0.5625) };
  const scale = Math.min(1, maxW / w, maxH / h);
  return { width: Math.max(1, Math.round(w * scale)), height: Math.max(1, Math.round(h * scale)) };
}

export function thumbUrl(a: { url: string; proxy_url?: string | null; width?: number | null; height?: number | null }, box: { width: number; height: number }): string {
  const base = a.proxy_url || a.url;
  if (!base.startsWith("https://media.discordapp.net/")) return base;
  const dpr = typeof window !== "undefined" ? Math.min(2, window.devicePixelRatio || 1) : 1;
  const sep = base.includes("?") ? "&" : "?";
  return `${base}${sep}width=${Math.round(box.width * dpr)}&height=${Math.round(box.height * dpr)}`;
}

export function formatBytes(n: number): string {
  if (!n) return "0 B";
  const u = ["B", "KB", "MB", "GB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 10 || i === 0 ? Math.round(v) : v.toFixed(1)} ${u[i]}`;
}

export function sameDay(a: string, b: string): boolean {
  return a.slice(0, 10) === b.slice(0, 10) && new Date(a).toDateString() === new Date(b).toDateString();
}

export function groupedWith(prev: DcMessage | undefined, m: DcMessage): boolean {
  if (!prev || prev.author.id !== m.author.id || m.referenced_message) return false;
  if (m.type !== 0 && m.type !== 19) return false;
  return snowflakeMs(m.id) - snowflakeMs(prev.id) < 7 * 60 * 1000;
}

export type Token =
  | { t: "text"; v: string }
  | { t: "bold"; v: string }
  | { t: "code"; v: string }
  | { t: "block"; v: string }
  | { t: "link"; v: string }
  | { t: "mention"; v: string; me: boolean }
  | { t: "channel"; v: string }
  | { t: "role"; v: string }
  | { t: "emoji"; v: string; src: string };

export type Lookup = { meId: string; users: Map<string, DcUser>; channelName: (id: string) => string | null; everyone: string; role: string };

const INLINE = /(<@!?(\d+)>|<#(\d+)>|<@&(\d+)>|<a?:(\w{1,32}):(\d+)>|@everyone|@here|https?:\/\/[^\s<>()]+[^\s<>().,;:!?'"]|\*\*([^*\n]+)\*\*)/g;

function inline(text: string, look: Lookup, out: Token[]) {
  let last = 0;
  for (const m of text.matchAll(INLINE)) {
    const i = m.index ?? 0;
    if (i > last) out.push({ t: "text", v: text.slice(last, i) });
    last = i + m[0].length;
    if (m[2]) {
      const u = look.users.get(m[2]);
      out.push({ t: "mention", v: `@${u ? displayName(u) : m[2]}`, me: m[2] === look.meId });
    } else if (m[3]) {
      out.push({ t: "channel", v: `#${look.channelName(m[3]) ?? m[3]}` });
    } else if (m[4]) {
      out.push({ t: "role", v: `@${look.role}` });
    } else if (m[5] && m[6]) {
      const animated = m[0].startsWith("<a:");
      out.push({ t: "emoji", v: `:${m[5]}:`, src: `${CDN}/emojis/${m[6]}.${animated ? "gif" : "webp"}?size=48` });
    } else if (m[0] === "@everyone" || m[0] === "@here") {
      out.push({ t: "mention", v: m[0], me: true });
    } else if (m[7]) {
      out.push({ t: "bold", v: m[7] });
    } else {
      out.push({ t: "link", v: m[0] });
    }
  }
  if (last < text.length) out.push({ t: "text", v: text.slice(last) });
}

export function tokenize(content: string, look: Lookup): Token[] {
  const out: Token[] = [];
  const parts = content.split(/(```[\s\S]*?```|`[^`\n]+`)/g);
  for (const p of parts) {
    if (!p) continue;
    if (p.startsWith("```") && p.endsWith("```") && p.length >= 6) {
      out.push({ t: "block", v: p.slice(3, -3).replace(/^[a-z0-9+-]*\n/i, "") });
    } else if (p.startsWith("`") && p.endsWith("`") && p.length >= 2) {
      out.push({ t: "code", v: p.slice(1, -1) });
    } else {
      inline(p, look, out);
    }
  }
  return out;
}

export function dayKey(ts: string): string {
  const d = new Date(ts);
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
}

export function dateToMs(value: string, endOfDay = false): number | null {
  if (!value) return null;
  const d = new Date(`${value}T00:00:00`);
  if (Number.isNaN(d.getTime())) return null;
  return endOfDay ? d.getTime() + 24 * 3600 * 1000 : d.getTime();
}

export function upsertMessage(list: DcMessage[], m: DcMessage): DcMessage[] {
  const i = list.findIndex((x) => x.id === m.id);
  if (i >= 0) {
    const next = list.slice();
    next[i] = m;
    return next;
  }
  if (!list.length || snowflakeCmp(m.id, list[list.length - 1].id) > 0) return [...list, m];
  const next = [...list, m];
  next.sort((a, b) => snowflakeCmp(a.id, b.id));
  return next;
}

export function mergeOlder(list: DcMessage[], older: DcMessage[]): DcMessage[] {
  const have = new Set(list.map((m) => m.id));
  const add = older.filter((m) => !have.has(m.id)).sort((a, b) => snowflakeCmp(a.id, b.id));
  return [...add, ...list];
}

export function trimWindow<T>(list: T[], max: number, keepEnd: boolean): T[] {
  if (list.length <= max) return list;
  return keepEnd ? list.slice(list.length - max) : list.slice(0, max);
}
