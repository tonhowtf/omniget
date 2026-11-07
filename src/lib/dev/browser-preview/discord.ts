import { emit } from "@tauri-apps/api/event";

type Args = Record<string, unknown>;
type User = { id: string; username: string; global_name: string | null; avatar: string | null; bot: boolean };
type Msg = Record<string, unknown> & { id: string; channel_id: string; author: User; content: string };

const EPOCH = 1420070400000n;
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const flake = (ms: number, n = 0) => ((BigInt(Math.floor(ms)) - EPOCH) << 22n | BigInt(n & 0x3fffff)).toString();
const DAY = 24 * 3600 * 1000;
const NOW = Date.now();
const BASE = Date.UTC(2024, 0, 1);

const ME: User = { id: flake(BASE - 900 * DAY, 1), username: "preview.me", global_name: "Preview", avatar: null, bot: false };
const PEOPLE: User[] = [
  { id: flake(BASE - 800 * DAY, 2), username: "marina.dev", global_name: "Marina", avatar: null, bot: false },
  { id: flake(BASE - 700 * DAY, 3), username: "kenji", global_name: "Kenji Sato", avatar: null, bot: false },
  { id: flake(BASE - 600 * DAY, 4), username: "lucas_r", global_name: null, avatar: null, bot: false },
  { id: flake(BASE - 500 * DAY, 5), username: "archivebot", global_name: "Archive Bot", avatar: null, bot: true },
];

const ACCOUNT = { id: ME.id, username: ME.username, global_name: ME.global_name, avatar: null };
const SECOND = { id: flake(BASE - 400 * DAY, 9), username: "alt.preview", global_name: "Alt account", avatar: null };

let accounts = [ACCOUNT, SECOND];
let active: string | null = ACCOUNT.id;
let connected: string | null = ACCOUNT.id;

function channel(id: string, guild: string | null, kind: number, name: string, extra: Record<string, unknown> = {}) {
  return { id, guild_id: guild, kind, name, position: 0, parent_id: null, last_message_id: flake(NOW - 60_000, Number(id.slice(-3))), topic: null, can_send: true, icon: null, recipients: [], ...extra };
}

const G1 = flake(BASE - 1000 * DAY, 100);
const G2 = flake(BASE - 990 * DAY, 101);
const G3 = flake(BASE - 980 * DAY, 102);
const id = (n: number) => flake(BASE - 970 * DAY, n);

const GUILDS = [
  {
    id: G1,
    name: "OmniGet Community",
    icon: null,
    channels: [
      channel(id(201), G1, 0, "welcome", { topic: "Read this first", can_send: false }),
      channel(id(202), G1, 5, "announcements", { can_send: false }),
      channel(id(210), G1, 4, "Talk"),
      channel(id(211), G1, 0, "general", { parent_id: id(210), topic: "Anything about OmniGet goes here." }),
      channel(id(212), G1, 0, "help", { parent_id: id(210) }),
      channel(id(213), G1, 11, "Download keeps failing on macOS", { parent_id: id(212) }),
      channel(id(214), G1, 0, "showcase", { parent_id: id(210) }),
      channel(id(220), G1, 4, "Dev"),
      channel(id(221), G1, 0, "contributors", { parent_id: id(220) }),
    ],
  },
  {
    id: G2,
    name: "Photography Club",
    icon: null,
    channels: [channel(id(301), G2, 0, "photos"), channel(id(302), G2, 0, "gear-talk")],
  },
  {
    id: G3,
    name: "Study Group",
    icon: null,
    channels: [channel(id(401), G3, 0, "lectures"), channel(id(402), G3, 0, "random")],
  },
];

const DMS = [
  channel(id(501), null, 1, "Marina", { recipients: [PEOPLE[0]] }),
  channel(id(502), null, 1, "Kenji Sato", { recipients: [PEOPLE[1]] }),
  channel(id(503), null, 3, "Weekend trip", { recipients: [PEOPLE[0], PEOPLE[2]] }),
];

const READ: Record<string, { last_message_id: string | null; mention_count: number }> = {};
for (const c of [...GUILDS.flatMap((g) => g.channels), ...DMS]) READ[c.id] = { last_message_id: c.last_message_id, mention_count: 0 };
READ[id(211)] = { last_message_id: flake(NOW - 3 * 3600_000), mention_count: 2 };
READ[id(212)] = { last_message_id: flake(NOW - 2 * DAY), mention_count: 0 };
READ[id(301)] = { last_message_id: flake(NOW - 2 * DAY), mention_count: 0 };
READ[id(502)] = { last_message_id: flake(NOW - DAY), mention_count: 1 };

function art(hue: number, w: number, h: number, label: string) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue} 70% 62%)"/><stop offset="1" stop-color="hsl(${(hue + 60) % 360} 60% 38%)"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><text x="50%" y="52%" fill="white" font-family="system-ui" font-size="${Math.round(h / 9)}" text-anchor="middle">${label}</text></svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

const LINES = [
  "Has anyone tried the new release yet?",
  "Yes! The playlist import is way faster now.",
  "Here is what I got after the update:",
  "Nice. Does it keep the original file names?",
  "It does, and the dates too 🙂",
  "I'll write it up in **#help** later.",
  "Check https://getomniget.com for the changelog",
  "`yt-dlp` updated itself in the background, no restart needed",
  "Can confirm, works on Windows 11 as well.",
  "Thanks everyone!",
];

const cache = new Map<string, Msg[]>();

function history(channelId: string): Msg[] {
  const hit = cache.get(channelId);
  if (hit) return hit;
  const c = [...GUILDS.flatMap((g) => g.channels), ...DMS].find((x) => x.id === channelId);
  const pool = c && !c.guild_id ? [...(c.recipients as User[]), ME] : [...PEOPLE, ME];
  const total = 180;
  const out: Msg[] = [];
  let t = NOW - 6 * DAY;
  for (let i = 0; i < total; i++) {
    t += (i % 9 === 0 ? 5 * 3600_000 : 40_000 + ((i * 7919) % 900_000)) * (i > total - 20 ? 0.2 : 1);
    if (t > NOW - 120_000) t = NOW - 120_000 + i * 500;
    const author = pool[(i * 5 + 3) % pool.length];
    const mid = flake(t, i + Number(channelId.slice(-3)) * 1000);
    const m: Msg = {
      id: mid,
      channel_id: channelId,
      guild_id: c?.guild_id ?? null,
      author,
      content: LINES[i % LINES.length],
      timestamp: new Date(t).toISOString(),
      edited_timestamp: i % 23 === 0 ? new Date(t + 60_000).toISOString() : null,
      attachments: [],
      embeds: [],
      mentions: [],
      mention_roles: [],
      mention_everyone: false,
      type: 0,
      sticker_items: [],
      referenced_message: null,
    };
    if (i % 10 === 2) {
      m.attachments = [
        { id: `${mid}1`, filename: `photo_${i}.png`, url: art((i * 37) % 360, 1280, 853, `photo ${i}`), proxy_url: null, size: 1_482_331, content_type: "image/png", width: 1280, height: 853 },
      ];
    }
    if (i % 17 === 5) {
      m.attachments = [{ id: `${mid}2`, filename: "notes.pdf", url: "https://example.com/notes.pdf", proxy_url: null, size: 245_120, content_type: "application/pdf", width: null, height: null }];
    }
    if (i % 10 === 6) {
      m.embeds = [{ type: "rich", title: "OmniGet 1.0 is out", description: "Download videos, courses and whole channels. Now with a Discord superpower.", url: "https://getomniget.com", color: 0xff9f0a, author: { name: "getomniget.com" }, image: null, thumbnail: null, video: null }];
    }
    if (i === total - 4 || i === total - 11) {
      m.author = PEOPLE[0];
      m.content = `<@${ME.id}> can you take a look at this when you have a minute?`;
      m.mentions = [ME];
    }
    if (i % 13 === 7 && out.length) {
      const r = out[out.length - 2] ?? out[out.length - 1];
      m.referenced_message = { ...r, referenced_message: null, embeds: [] };
    }
    out.push(m);
  }
  cache.set(channelId, out);
  return out;
}

function status() {
  return {
    accounts,
    active,
    connected_account: connected,
    connection: connected ? { state: "ready" } : { state: "offline" },
  };
}

function snapshot() {
  if (!connected) return null;
  const me = connected === ME.id ? ME : { ...SECOND, bot: false };
  return { me, guilds: connected === ME.id ? GUILDS : GUILDS.slice(1), dms: DMS, read_states: READ };
}

function qrSvg() {
  const cells: string[] = [];
  let seed = 7;
  for (let y = 0; y < 29; y++) {
    for (let x = 0; x < 29; x++) {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      const finder = (x < 7 && y < 7) || (x > 21 && y < 7) || (x < 7 && y > 21);
      const on = finder ? (x % 6 === 0 || y % 6 === 0 || x % 28 === 0 || (x % 22 >= 2 && x % 22 <= 4 && y % 22 >= 2 && y % 22 <= 4)) : seed % 3 === 0;
      if (on) cells.push(`<rect x="${x}" y="${y}" width="1" height="1"/>`);
    }
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 29 29" shape-rendering="crispEdges"><g fill="#000">${cells.join("")}</g></svg>`;
}

let qrRun = 0;

async function qrStart() {
  const run = ++qrRun;
  await sleep(400);
  if (run !== qrRun) return null;
  await emit("discord-qr", { stage: "code", svg: qrSvg(), url: "https://discord.com/ra/preview" });
  await sleep(5000);
  if (run !== qrRun) return null;
  await emit("discord-qr", { stage: "scanned", user: { id: SECOND.id, discriminator: "0", username: SECOND.username, avatar: null } });
  await sleep(4000);
  if (run !== qrRun) return null;
  connected = SECOND.id;
  active = SECOND.id;
  await emit("discord-qr", { stage: "done", account: SECOND });
  return null;
}

let mentionSent = false;

function messages(args: Args) {
  const channelId = String(args.channelId ?? "");
  const before = args.before ? String(args.before) : null;
  const limit = Number(args.limit ?? 50);
  const all = history(channelId);
  const end = before ? all.findIndex((m) => m.id === before) : all.length;
  const page = all.slice(Math.max(0, end - limit), end < 0 ? 0 : end).reverse();
  if (!before && channelId === id(211) && !mentionSent) {
    mentionSent = true;
    setTimeout(() => {
      const m = { ...all[all.length - 1], id: flake(Date.now(), 7), author: PEOPLE[1], content: `Welcome back <@${ME.id}>! New messages show up live here.`, mentions: [ME], timestamp: new Date().toISOString(), attachments: [], embeds: [], referenced_message: null, edited_timestamp: null };
      all.push(m);
      void emit("discord-message-create", { message: m, guild_id: G1 });
    }, 6000);
  }
  return sleep(250).then(() => page);
}

function send(args: Args) {
  const channelId = String(args.channelId ?? "");
  const content = String(args.content ?? "").trimEnd();
  if (!content.trim()) throw "empty";
  if ([...content].length > 2000) throw "too_long";
  const all = history(channelId);
  const replyTo = args.replyTo ? all.find((m) => m.id === args.replyTo) : null;
  const m: Msg = {
    id: flake(Date.now(), 11),
    channel_id: channelId,
    guild_id: null,
    author: ME,
    content,
    timestamp: new Date().toISOString(),
    edited_timestamp: null,
    attachments: [],
    embeds: [],
    mentions: [],
    mention_roles: [],
    mention_everyone: false,
    type: replyTo ? 19 : 0,
    sticker_items: [],
    referenced_message: replyTo ? { ...replyTo, referenced_message: null } : null,
  };
  all.push(m);
  return sleep(200).then(() => m);
}

function edit(args: Args) {
  const all = history(String(args.channelId ?? ""));
  const m = all.find((x) => x.id === args.messageId);
  if (!m) throw "not_found:Unknown Message";
  m.content = String(args.content ?? "");
  m.edited_timestamp = new Date().toISOString();
  return { ...m };
}

function remove(args: Args) {
  const all = history(String(args.channelId ?? ""));
  const i = all.findIndex((x) => x.id === args.messageId);
  if (i >= 0) all.splice(i, 1);
  return null;
}

const cancelled = new Set<string>();

async function downloadChannel(args: Args) {
  const job = String(args.jobId ?? "");
  const folder = `${String(args.dir || "/preview/Downloads")}/Discord/OmniGet Community/general`;
  let scanned = 0;
  let found = 0;
  for (let i = 0; i < 6; i++) {
    if (cancelled.has(job)) break;
    scanned += 100;
    found += 9;
    await emit("discord-download-progress", { job_id: job, phase: "scan", scanned, found, done: 0, skipped: 0, failed: 0, bytes: 0 });
    await sleep(300);
  }
  let done = 0;
  while (!cancelled.has(job) && done < found) {
    done += 3;
    await emit("discord-download-progress", { job_id: job, phase: "download", scanned, found, done, skipped: 0, failed: 0, bytes: done * 1_400_000 });
    await sleep(250);
  }
  const wasCancelled = cancelled.delete(job);
  await emit("discord-download-progress", { job_id: job, phase: wasCancelled ? "cancelled" : "done", scanned, found, done, skipped: 0, failed: 0, bytes: done * 1_400_000 });
  return { folder, paths: Array.from({ length: done }, (_, i) => `${folder}/photo_${i}.png`), failed: 0, skipped: 2, cancelled: wasCancelled };
}

async function exportChannel(args: Args) {
  const job = String(args.jobId ?? "");
  const folder = `${String(args.dir || "/preview/Downloads")}/Discord/OmniGet Community/general`;
  const formats = ((args.options as Args | undefined)?.formats as string[] | undefined) ?? ["html"];
  let scanned = 0;
  for (let i = 0; i < 8 && !cancelled.has(job); i++) {
    scanned += 100;
    await emit("discord-export-progress", { job_id: job, phase: "scan", scanned });
    await sleep(250);
  }
  const wasCancelled = cancelled.delete(job);
  await emit("discord-export-progress", { job_id: job, phase: wasCancelled ? "cancelled" : "done", scanned });
  return { folder, files: wasCancelled ? [] : formats.map((f) => `${folder}/general.${f}`), messages: scanned, cancelled: wasCancelled };
}

export const discordFixtures: Record<string, (args: Args) => unknown> = {
  discord_status: status,
  discord_connect: () => {
    connected = active;
    return status();
  },
  discord_disconnect: () => {
    connected = null;
    return status();
  },
  discord_login_token: ({ token }) => {
    const t = String(token ?? "").trim();
    if (t.length < 50 || t.split(".").length !== 3) throw "token_format";
    connected = SECOND.id;
    active = SECOND.id;
    return SECOND;
  },
  discord_qr_start: () => {
    void qrStart();
    return null;
  },
  discord_qr_cancel: () => {
    qrRun++;
    return null;
  },
  discord_logout: ({ accountId }) => {
    accounts = accounts.filter((a) => a.id !== accountId);
    if (connected === accountId) connected = null;
    if (active === accountId) active = accounts[0]?.id ?? null;
    return status();
  },
  discord_switch_account: ({ accountId }) => {
    if (!accounts.some((a) => a.id === accountId)) throw "unknown_account";
    active = String(accountId);
    connected = active;
    return status();
  },
  discord_snapshot: snapshot,
  discord_messages: messages,
  discord_send: send,
  discord_edit: edit,
  discord_delete: remove,
  discord_ack: ({ channelId, messageId }) => {
    READ[String(channelId)] = { last_message_id: String(messageId), mention_count: 0 };
    return null;
  },
  discord_cancel: ({ jobId }) => {
    cancelled.add(String(jobId));
    return null;
  },
  discord_download_message: ({ dir }) => {
    const folder = `${String(dir || "/preview/Downloads")}/Discord/OmniGet Community/general`;
    return sleep(500).then(() => ({ folder, paths: [`${folder}/photo.png`], failed: 0, skipped: 0, cancelled: false }));
  },
  discord_download_channel: downloadChannel,
  discord_export: exportChannel,
  reveal_file: () => null,
};
