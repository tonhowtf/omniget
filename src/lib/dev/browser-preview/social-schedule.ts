import { emit } from "@tauri-apps/api/event";

type Args = Record<string, unknown>;

const HOUR = 3_600_000;
const iso = (ms: number) => new Date(ms).toISOString();
const base = Date.now();

type Item = Record<string, unknown> & { id: number; status: string; at: string | null; sent_at: string | null };

function item(id: number, status: string, text: string, offsetHours: number | null, extra: Args = {}): Item {
  return {
    id,
    platform: "x",
    kind: "post",
    status,
    text,
    thread: [],
    reply_to: null,
    quote: null,
    repost: null,
    media: [],
    at: offsetHours === null ? null : iso(base + offsetHours * HOUR),
    sent_at: status === "sent" && offsetHours !== null ? iso(base + offsetHours * HOUR) : null,
    post_id: null,
    post_url: null,
    parts_total: 1,
    parts_sent: 0,
    error: null,
    created_by: "screen",
    updated_at: iso(base),
    ...extra,
  };
}

let items: Item[] = [
  item(1, "scheduled", "Shipped the new export flow today. Smaller files, same quality.", 3),
  item(2, "scheduled", "Thread: what I learned moving our docs to plain markdown.", 5, { kind: "thread", thread: ["One file per subject.", "Links over folders."], parts_total: 3, created_by: "agent:claude" }),
  item(3, "scheduled", "New on the blog: Notes on small tools https://example.com/notes", 27, { created_by: "feed:1" }),
  item(4, "draft", "Idea: a weekly post with the three best reference videos.", null),
  item(5, "sent", "Morning light at the studio", -20, { post_id: "1800000001", post_url: "https://x.com/preview.me/status/1800000001" }),
  item(6, "failed", "Launch day checklist, part one.", -6, { kind: "thread", thread: ["Part two."], parts_total: 2, parts_sent: 1, error: 'REJECTED: Short Posts refused the request: duplicate (code 187)' }),
  item(7, "missed", "Good morning!", -30, { error: 'MISSED: {"message":"it came due while OmniGet was closed","retryable":false,"next_step":"use send_now"}' }),
];
let slots = [
  { weekday: "mon", time: "09:00" },
  { weekday: "wed", time: "18:00" },
  { weekday: "fri", time: "12:30" },
];
let feeds: Args[] = [
  { id: 1, platform: "x", url: "https://example.com/feed.xml", template: "New on the blog: {title} {link}", interval_minutes: 60, monthly_cap: 30, posted_this_month: 4, skipped_over_cap: 0, primed: true, last_checked_at: iso(base - HOUR / 2), last_error: null },
];
const settings = { grace_minutes: 10, long_posts: "auto" };
let nextId = 8;

function nextSlot(): string | null {
  if (!slots.length) return null;
  const taken = new Set(items.filter((i) => i.status === "scheduled").map((i) => i.at));
  for (let d = 0; d < 60; d++) {
    for (const s of slots) {
      const day = new Date(base + d * 24 * HOUR);
      const weekday = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"][day.getDay()];
      if (weekday !== s.weekday) continue;
      day.setHours(Number(s.time.slice(0, 2)), Number(s.time.slice(3, 5)), 0, 0);
      if (day.getTime() > Date.now() && !taken.has(day.toISOString())) return day.toISOString();
    }
  }
  return null;
}

function reject(code: string, message: string): never {
  throw `${code}: ${JSON.stringify({ message, retryable: false, next_step: "" })}`;
}

function content(a: Args): Args {
  if (a.repost) return { kind: "repost", repost: String(a.repost), text: null, thread: [], parts_total: 1 };
  const thread = Array.isArray(a.thread) ? (a.thread as string[]) : [];
  if (!String(a.text ?? "").trim()) reject("EMPTY_TEXT", "text is required");
  return {
    kind: thread.length ? "thread" : a.reply_to ? "reply" : a.quote ? "quote" : "post",
    text: String(a.text),
    thread,
    reply_to: a.reply_to ?? null,
    quote: a.quote ?? null,
    repost: null,
    media: Array.isArray(a.media) ? a.media : [],
    parts_total: thread.length + 1,
  };
}

function due(a: Args): string | null {
  if (a.at) {
    if (new Date(String(a.at)).getTime() <= Date.now()) reject("PAST_TIME", "the time is in the past");
    return new Date(String(a.at)).toISOString();
  }
  if (a.slot === "next") return nextSlot() ?? reject("NO_SLOTS", "there is no free posting slot");
  return null;
}

function list(a: Args) {
  const statuses = Array.isArray(a.status) && a.status.length ? (a.status as string[]) : ["draft", "scheduled", "sending", "sent", "failed", "missed"];
  const when = (i: Item) => new Date(i.sent_at ?? i.at ?? String(i.updated_at)).getTime();
  const all = items.filter((i) => statuses.includes(i.status)).sort((x, y) => (a.order === "desc" ? when(y) - when(x) : when(x) - when(y)));
  const shown = all.slice(0, Number(a.limit ?? 50));
  const counts: Record<string, number> = {};
  for (const i of items) counts[i.status] = (counts[i.status] ?? 0) + 1;
  return {
    platform: a.platform,
    items: a.platform === "x" ? shown : [],
    total: all.length,
    returned: shown.length,
    truncated: all.length > shown.length,
    counts,
    slots,
    next_slot: nextSlot(),
    feeds,
    settings,
    limits: { text: 280, thread_parts: 25, images: 4 },
    supported: a.platform === "x",
  };
}

function find(id: unknown): Item {
  return items.find((i) => i.id === Number(id)) ?? reject("NOT_FOUND", "there is no such item");
}

function write(a: Args) {
  switch (a.op) {
    case "add":
    case "draft": {
      const at = a.op === "draft" ? null : (due(a) ?? reject("TIME_REQUIRED", "no time was given"));
      const it = { ...item(nextId++, a.op === "draft" ? "draft" : "scheduled", "", null), ...content(a), at } as Item;
      items = [...items, it];
      return { item: it };
    }
    case "update": {
      const it = find(a.id);
      const at = due(a);
      Object.assign(it, a.text !== undefined || a.repost !== undefined ? content(a) : {}, at ? { at, status: "scheduled", error: null } : {});
      return { item: it };
    }
    case "cancel": {
      const it = find(a.id);
      it.status = "cancelled";
      return { item: it };
    }
    case "send_now": {
      const it = find(a.id);
      it.status = "sending";
      setTimeout(() => {
        it.status = "sent";
        it.sent_at = iso(Date.now());
        it.error = null;
        it.parts_sent = Number(it.parts_total);
        it.post_url = `https://x.com/preview.me/status/${1800000100 + it.id}`;
        void emit("social-schedule-changed", { platform: "x", id: it.id, status: "sent" });
      }, 1500);
      return { item: it, queued: true };
    }
    case "set_slots":
      if (Array.isArray(a.slots)) slots = a.slots as typeof slots;
      if (typeof a.grace_minutes === "number") settings.grace_minutes = a.grace_minutes;
      if (typeof a.long_posts === "string") settings.long_posts = a.long_posts;
      return { slots, next_slot: nextSlot(), settings };
    case "import_csv":
      return {
        dry_run: Boolean(a.dry_run),
        rows: 4,
        valid: 2,
        scheduled: a.dry_run ? 0 : 2,
        rejected_count: 2,
        rejected: [
          { row: 3, code: "PAST_TIME", reason: "the time is in the past" },
          { row: 5, code: "TOO_LONG", reason: "part 1 counts 301 of 280 on Short Posts" },
        ],
        preview: [
          { row: 2, at: iso(base + 30 * HOUR), kind: "post", text: "First imported post", repost: null },
          { row: 4, at: iso(base + 54 * HOUR), kind: "thread", text: "Imported thread", repost: null },
        ],
        truncated: false,
      };
    case "export_ics":
      return { path: "/preview/social/exports/schedule-x-preview.ics", events: 4, sent: 1, scheduled: 3 };
    default:
      return reject("INVALID_ARGUMENTS", "unknown op");
  }
}

function feedsTool(a: Args) {
  if (a.op === "add") {
    const feed = { id: nextId++, platform: "x", url: String(a.url), template: String(a.template ?? "{title} {link}"), interval_minutes: Number(a.interval_minutes ?? 60), monthly_cap: Number(a.monthly_cap ?? 30), posted_this_month: 0, skipped_over_cap: 0, primed: false, last_checked_at: null, last_error: null };
    feeds = [...feeds, feed];
    return { feed };
  }
  if (a.op === "remove") feeds = feeds.filter((f) => f.id !== Number(a.id));
  return { total: feeds.length, returned: feeds.length, truncated: false, feeds };
}

export function scheduleFixture(name: string, a: Args): unknown {
  if (name === "social_schedule_list") return list(a);
  if (name === "social_schedule") return write(a);
  return feedsTool(a);
}
