import { emit } from "@tauri-apps/api/event";

type Args = Record<string, unknown>;

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const NOW = Math.floor(Date.now() / 1000);
const H = 3600;
const D = 86_400;

const THUMB =
  "data:image/svg+xml;utf8," +
  encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 56 56"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#f6b26b"/><stop offset="1" stop-color="#6fa8dc"/></linearGradient></defs><rect width="56" height="56" fill="url(#g)"/><circle cx="40" cy="16" r="6" fill="#fff8"/><path d="M0 44l16-14 12 10 10-8 18 16v8H0z" fill="#0004"/></svg>',
  );

function fakeQr(): string {
  let seed = 7;
  const rand = () => ((seed = (seed * 9301 + 49297) % 233280) / 233280);
  const n = 29;
  let cells = "";
  for (let y = 0; y < n; y++) {
    for (let x = 0; x < n; x++) {
      const finder = (x < 7 && y < 7) || (x >= n - 7 && y < 7) || (x < 7 && y >= n - 7);
      if (finder) {
        const fx = x >= n - 7 ? x - (n - 7) : x;
        const fy = y >= n - 7 ? y - (n - 7) : y;
        const ring = fx === 0 || fy === 0 || fx === 6 || fy === 6 || (fx >= 2 && fx <= 4 && fy >= 2 && fy <= 4);
        if (ring) cells += `<rect x="${x + 2}" y="${y + 2}" width="1" height="1"/>`;
      } else if (rand() > 0.52) {
        cells += `<rect x="${x + 2}" y="${y + 2}" width="1" height="1"/>`;
      }
    }
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${n + 4} ${n + 4}" shape-rendering="crispEdges"><rect width="100%" height="100%" fill="#fff"/><g fill="#111">${cells}</g></svg>`;
}

type Msg = {
  id: string;
  chat: string;
  from_me: boolean;
  sender: string | null;
  sender_name: string | null;
  ts: number;
  kind: string;
  text: string | null;
  mime: string | null;
  size: number | null;
  file_name: string | null;
  duration: number | null;
  thumb: string | null;
  saved_path: string | null;
  edited: boolean;
  deleted: boolean;
  seq: number;
  quote_id: string | null;
  reactions: { emoji: string; count: number; mine: boolean }[];
};

type Sched = { id: number; chat: string; text: string; reply_to: string | null; at: number; created: number; status: string; error: string | null; sent_id: string | null; sent_at: number | null };
const scheduledRows: Sched[] = [];

const FAMILY = "120363025551234567@g.us";
const ANA = "5511987654321@s.whatsapp.net";
const WORK = "120363099887766554@g.us";
const BRUNO = "5521912345678@s.whatsapp.net";
const LOJA = "5531955554444@s.whatsapp.net";

const CHAT_NAMES: Record<string, string> = {
  [FAMILY]: "Família 🏡",
  [ANA]: "Ana Souza",
  [WORK]: "Projeto Landing",
  [BRUNO]: "Bruno",
  [LOJA]: "+55 31 95555-4444",
};

const chatTags: Record<string, string[]> = { [ANA]: ["cliente"], [LOJA]: ["fornecedor"] };
const chatFields: Record<string, Record<string, string>> = { [ANA]: { cidade: "Belo Horizonte" } };
const chatDrafts: Record<string, string> = { [ANA]: "Posso sim! Te espero \u00e0s 15h no Caf\u00e9 Girondino." };
type PreviewRule = { id: number; name: string; enabled: boolean; trigger: { kind: string; mode?: string; words?: string[] }; chats: string[]; action: { kind: string; text?: string; instructions?: string; fallback?: string | null }; cooldown_minutes: number; tag?: string | null; follow_ups?: { after_minutes: number; text: string }[]; needs_tag?: string | null; ask?: { question: string; field: string } | null };
let ruleSeq = 2;
const ruleLimits = { enabled: true, per_hour: 30, per_day: 200, delay_min_ms: 1500, delay_max_ms: 4000, manual_pause_minutes: 30 };
const ruleRows: PreviewRule[] = [
  { id: 1, name: "Pre\u00e7o", enabled: true, trigger: { kind: "keyword", mode: "contains", words: ["pre\u00e7o", "quanto custa"] }, chats: [], action: { kind: "reply", text: "Oi {{first_name}}! O bolo sai por R$ 60." }, cooldown_minutes: 60, tag: "cliente", follow_ups: [{ after_minutes: 30, text: "Ficou alguma d\u00favida, {{first_name}}?" }], needs_tag: null, ask: { question: "Qual a sua cidade?", field: "cidade" } },
  { id: 2, name: "D\u00favidas", enabled: false, trigger: { kind: "any" }, chats: [], action: { kind: "ai", instructions: "Loja de bolos. Entrega em BH.", fallback: null }, cooldown_minutes: 30, tag: null, follow_ups: [], needs_tag: null, ask: null },
];
const ruleOptOuts = [{ chat: LOJA, name: "+55 31 95555-4444", ts: 0 }];

let seq = 0;
function m(chat: string, ago: number, partial: Partial<Msg>): Msg {
  seq += 1;
  return {
    id: `3EB0${String(seq).padStart(16, "0")}`,
    chat,
    from_me: false,
    sender: null,
    sender_name: chat.endsWith("@g.us") ? "Ana Souza" : CHAT_NAMES[chat],
    ts: NOW - ago,
    kind: "text",
    text: null,
    mime: null,
    size: null,
    file_name: null,
    duration: null,
    thumb: null,
    saved_path: null,
    edited: false,
    deleted: false,
    seq,
    quote_id: null,
    reactions: [],
    ...partial,
  };
}

const messages: Record<string, Msg[]> = {
  [FAMILY]: [
    m(FAMILY, 3 * D, { text: "Fotos do almoço de domingo 👇", sender_name: "Tia Lúcia" }),
    m(FAMILY, 3 * D - 60, { kind: "image", mime: "image/jpeg", size: 284_000, thumb: THUMB, sender_name: "Tia Lúcia" }),
    m(FAMILY, 3 * D - 58, { kind: "image", mime: "image/jpeg", size: 312_500, thumb: THUMB, sender_name: "Tia Lúcia", text: "A mesa ficou linda" }),
    m(FAMILY, 2 * D, { kind: "video", mime: "video/mp4", size: 18_400_000, duration: 47, thumb: THUMB, sender_name: "Pedro" }),
    m(FAMILY, 2 * D - 300, { from_me: true, sender_name: null, text: "Que demais! Manda o vídeo do parabéns também", quote_id: "3EB00000000000000004" }),
    m(FAMILY, 26 * H, { kind: "voice", mime: "audio/ogg; codecs=opus", size: 41_000, duration: 23, sender_name: "Vó Maria" }),
    m(FAMILY, 25 * H, { kind: "document", mime: "application/pdf", size: 1_240_000, file_name: "Receita do bolo.pdf", sender_name: "Tia Lúcia" }),
    m(FAMILY, 3 * H, { kind: "sticker", mime: "image/webp", size: 18_000, sender_name: "Pedro" }),
    m(FAMILY, 2 * H, { text: "Domingo que vem é aqui em casa!", sender_name: "Vó Maria", reactions: [{ emoji: "❤️", count: 3, mine: true }, { emoji: "👍", count: 1, mine: false }] }),
  ],
  [ANA]: [
    m(ANA, 5 * D, { text: "Oi! Conseguiu ver o contrato?" }),
    m(ANA, 5 * D - 120, { from_me: true, sender_name: null, text: "Vi sim, já assinei" }),
    m(ANA, 5 * D - 100, { from_me: true, sender_name: null, kind: "document", mime: "application/pdf", size: 820_000, file_name: "Contrato assinado.pdf", saved_path: "/preview/Downloads/Phone Chats/Ana Souza/2025-01-10 Contrato assinado.pdf" }),
    m(ANA, 40 * 60, { kind: "location", text: "Café Girondino, Centro" }),
    m(ANA, 30 * 60, { text: "Te espero lá às 15h, pode ser?", edited: true }),
    m(ANA, 25 * 60, { deleted: true }),
  ],
  [WORK]: [
    m(WORK, 4 * H, { text: "Subi a nova versão da landing", sender_name: "Bruno" }),
    m(WORK, 4 * H - 30, { kind: "image", mime: "image/png", size: 1_820_000, thumb: THUMB, sender_name: "Bruno", text: "Hero novo" }),
    m(WORK, 3 * H, { kind: "poll", text: "Qual CTA fica?", sender_name: "Carla" }),
    m(WORK, 2 * H, { from_me: true, sender_name: null, deleted: true }),
    m(WORK, 70 * 60, { kind: "video", mime: "video/mp4", size: 64_000_000, duration: 312, thumb: THUMB, sender_name: "Carla", text: "Gravação da call" }),
  ],
  [BRUNO]: [m(BRUNO, 8 * D, { kind: "audio", mime: "audio/mpeg", size: 5_400_000, duration: 214, file_name: "demo-v2.mp3" }), m(BRUNO, 8 * D - 60, { text: "Escuta essa versão" })],
  [LOJA]: [m(LOJA, 20 * D, { text: "Seu pedido #4821 saiu para entrega 🚚" })],
};

const unread: Record<string, number> = { [FAMILY]: 3, [ANA]: 2, [WORK]: 0, [BRUNO]: 0, [LOJA]: 0 };

type Stage = "connected" | "unlinked" | "offline";

function initialStage(): Stage {
  const q = new URLSearchParams(location.search).get("wa");
  if (q === "unlinked" || q === "offline") return q;
  return "connected";
}

let stage: Stage = initialStage();
let pairing = false;
let pairCode: string | null = null;
const cancelled = new Set<string>();

function status() {
  const linked = stage !== "unlinked";
  return {
    state: stage === "connected" ? "connected" : stage === "offline" ? "offline" : pairing ? "pairing" : "idle",
    linked,
    account: linked ? { jid: "5511999990000@s.whatsapp.net", push_name: "Tonho", platform: "android" } : null,
    qr_svg: !linked && pairing ? fakeQr() : null,
    qr_timeout_secs: !linked && pairing ? 60 : null,
    pair_code: !linked ? pairCode : null,
    error: null,
    sync_progress: null,
  };
}

async function pushStatus() {
  await emit("phonechat-status", status());
}

function chatRows(args: Args) {
  if (stage === "unlinked") return [];
  const q = String(args.query ?? "").trim().toLowerCase();
  return Object.keys(CHAT_NAMES)
    .map((jid) => {
      const list = messages[jid] ?? [];
      const last = list[list.length - 1];
      return {
        jid,
        name: CHAT_NAMES[jid],
        is_group: jid.endsWith("@g.us"),
        last_ts: last?.ts ?? null,
        unread: unread[jid] ?? 0,
        archived: false,
        last_kind: last?.kind ?? null,
        last_text: last?.text ?? null,
        last_from_me: last?.from_me ?? false,
        media_count: list.filter((x) => !x.deleted && ["image", "video", "audio", "voice", "document", "sticker"].includes(x.kind)).length,
        message_count: list.length,
        tags: chatTags[jid] ?? [],
        fields: Object.entries(chatFields[jid] ?? {}),
      };
    })
    .filter((c) => !q || c.name.toLowerCase().includes(q) || c.jid.includes(q))
    .sort((a, b) => (b.last_ts ?? 0) - (a.last_ts ?? 0));
}

function stats(args: Args) {
  const list = messages[String(args.chat)] ?? [];
  const s = { image: 0, video: 0, audio: 0, document: 0, sticker: 0, image_bytes: 0, video_bytes: 0, audio_bytes: 0, document_bytes: 0, sticker_bytes: 0, saved: 0 };
  for (const x of list) {
    if (x.deleted) continue;
    const k = x.kind === "voice" ? "audio" : x.kind;
    if (!(k in s) || k === "saved") continue;
    (s as Record<string, number>)[k] += 1;
    (s as Record<string, number>)[`${k}_bytes`] += x.size ?? 0;
    if (x.saved_path) s.saved += 1;
  }
  return s;
}

function savedPath(chat: string, x: Msg): string {
  const ext = x.file_name ? "" : `.${(x.mime ?? "bin").split("/")[1]?.split(";")[0] ?? "bin"}`;
  return `/preview/Downloads/Phone Chats/${CHAT_NAMES[chat]}/${new Date(x.ts * 1000).toISOString().slice(0, 10)} ${x.file_name ?? `${x.kind} ${x.id.slice(0, 10)}${ext}`}`;
}

function requireOnline() {
  if (stage !== "connected") throw "offline";
}

async function downloadOne(args: Args) {
  requireOnline();
  const chat = String(args.chat);
  const id = String(args.messageId);
  const x = (messages[chat] ?? []).find((y) => y.id === id);
  if (!x) throw "no_media";
  await emit("phonechat-download-progress", { job_id: null, chat, message_id: id, stage: "start" });
  await sleep(700);
  if (x.ts < NOW - 4 * D) {
    await emit("phonechat-download-progress", { job_id: null, chat, message_id: id, stage: "retrying" });
    await sleep(3000);
    if (x.ts < NOW - 15 * D || x.kind === "audio") {
      await emit("phonechat-download-progress", { job_id: null, chat, message_id: id, stage: "error", error: "media_gone" });
      throw "media_gone";
    }
  }
  x.saved_path = savedPath(chat, x);
  await emit("phonechat-download-progress", { job_id: null, chat, message_id: id, stage: "done", path: x.saved_path });
  return { message_id: id, path: x.saved_path, skipped: false };
}

let jobs = 0;

async function downloadChat(args: Args) {
  requireOnline();
  const chat = String(args.chat);
  const kinds = (args.kinds as string[]) ?? [];
  if (kinds.length === 0) throw "empty_filter";
  const want = new Set(kinds.flatMap((k) => (k === "audio" ? ["audio", "voice"] : [k])));
  const rows = (messages[chat] ?? []).filter((x) => !x.deleted && want.has(x.kind));
  const jobId = `wa-${++jobs}`;
  const dir = `/preview/Downloads/Phone Chats/${CHAT_NAMES[chat]}`;
  void (async () => {
    let done = 0;
    let skipped = 0;
    let failed = 0;
    let bytes = 0;
    const errors: Record<string, number> = {};
    for (const [index, x] of rows.entries()) {
      if (cancelled.has(jobId)) break;
      await emit("phonechat-download-progress", { job_id: jobId, chat, message_id: x.id, stage: "file", index, total: rows.length, done, skipped, failed, bytes });
      await sleep(450);
      if (!x.saved_path && x.ts < NOW - 2 * D) {
        await emit("phonechat-download-progress", { job_id: jobId, chat, message_id: x.id, stage: "retrying" });
        await sleep(1800);
      }
      if (x.saved_path) skipped += 1;
      else if (x.kind === "sticker") {
        failed += 1;
        errors.retry_timeout = (errors.retry_timeout ?? 0) + 1;
      } else {
        x.saved_path = savedPath(chat, x);
        done += 1;
        bytes += x.size ?? 0;
      }
    }
    await emit("phonechat-download-done", { job_id: jobId, chat, total: rows.length, done, skipped, failed, bytes, dir, cancelled: cancelled.has(jobId), errors });
  })();
  return { job_id: jobId, total: rows.length, dir };
}

export const phonechatFixtures: Record<string, (args: Args) => unknown> = {
  phonechat_status: () => status(),
  phonechat_connect: async () => {
    if (stage === "unlinked") {
      pairing = true;
      await pushStatus();
      setTimeout(() => {
        if (!pairing || stage !== "unlinked") return;
        stage = "connected";
        pairing = false;
        void pushStatus();
        void emit("phonechat-chats-changed", {});
      }, 9000);
    } else if (stage === "offline") {
      stage = "connected";
      await pushStatus();
    }
    return status();
  },
  phonechat_disconnect: async () => {
    pairing = false;
    pairCode = null;
    await pushStatus();
    return status();
  },
  phonechat_pair_phone: async (args) => {
    const digits = String(args.phone ?? "").replace(/\D/g, "");
    if (digits.length < 8) throw "invalid_phone";
    await sleep(600);
    pairing = true;
    pairCode = "K7Q2M9XD";
    await pushStatus();
    return pairCode;
  },
  phonechat_logout: async () => {
    stage = "unlinked";
    pairing = false;
    pairCode = null;
    await pushStatus();
    return status();
  },
  phonechat_chats: chatRows,
  phonechat_messages: (args) => {
    const chat = String(args.chat);
    const before = typeof args.before === "number" ? args.before : Infinity;
    const limit = typeof args.limit === "number" ? args.limit : 80;
    const list = (messages[chat] ?? []).filter((x) => x.ts < before);
    return list.slice(Math.max(0, list.length - limit));
  },
  phonechat_mark_read: ({ chat }) => {
    if (stage !== "connected") throw "offline";
    const n = unread[String(chat)] ?? 0;
    unread[String(chat)] = 0;
    void emit("phonechat-chats-changed", {});
    return n;
  },
  phonechat_send_text: async (args) => {
    requireOnline();
    const chat = String(args.chat);
    const row = m(chat, 0, { from_me: true, sender_name: null, text: String(args.text ?? ""), quote_id: typeof args.replyTo === "string" ? args.replyTo : null });
    (messages[chat] ??= []).push(row);
    await sleep(250);
    void emit("phonechat-chats-changed", {});
    return row;
  },
  phonechat_react: async (args) => {
    requireOnline();
    const row = (messages[String(args.chat)] ?? []).find((x) => x.id === args.id);
    if (!row) throw "message_not_found";
    const emoji = String(args.emoji ?? "");
    const rest = row.reactions
      .map((r) => (r.mine ? { ...r, count: r.count - 1, mine: false } : r))
      .filter((r) => r.count > 0);
    if (emoji) {
      const same = rest.find((r) => r.emoji === emoji);
      if (same) Object.assign(same, { count: same.count + 1, mine: true });
      else rest.push({ emoji, count: 1, mine: true });
    }
    row.reactions = rest;
    void emit("phonechat-message", { chat: args.chat });
  },
  phonechat_field: ({ chat, key, value }) => {
    const name = String(key).trim().toLowerCase();
    if (!/^[\p{L}\p{N}_]{1,30}$/u.test(name)) throw "invalid_field";
    const row = (chatFields[String(chat)] ??= {});
    if (String(value).trim()) row[name] = String(value).trim();
    else delete row[name];
    void emit("phonechat-chats-changed", {});
  },
  phonechat_contacts_export: () => Object.keys(CHAT_NAMES).length,
  phonechat_draft: ({ chat }) => chatDrafts[String(chat)] ?? null,
  phonechat_draft_clear: ({ chat }) => delete chatDrafts[String(chat)],
  phonechat_tag: ({ chat, tag, on }) => {
    const key = String(chat);
    const name = String(tag).trim();
    if (!name || name.length > 30) throw "invalid_tag";
    const rest = (chatTags[key] ?? []).filter((x) => x !== name);
    chatTags[key] = on ? [...rest, name].sort() : rest;
    void emit("phonechat-chats-changed", {});
    return true;
  },
  phonechat_send_file: async (args) => {
    requireOnline();
    const chat = String(args.chat);
    const name = String(args.path).split(/[\\/]/).pop() ?? "file";
    const row = m(chat, 0, { from_me: true, sender_name: null, kind: "document", mime: "application/pdf", size: 420_000, file_name: name, text: (args.caption as string | null) ?? null, saved_path: String(args.path) });
    (messages[chat] ??= []).push(row);
    await sleep(500);
    void emit("phonechat-chats-changed", {});
    return row;
  },
  phonechat_rules: () => ({ limits: ruleLimits, rules: ruleRows, sent_hour: 2, sent_day: 9, opt_outs: ruleOptOuts, stats: [{ rule_id: 1, sent: 14, skipped: 3, failed: 0 }, { rule_id: 2, sent: 0, skipped: 0, failed: 1 }] }),
  phonechat_rule_save: (args) => {
    const rule = args.rule as PreviewRule;
    if (!rule.name.trim()) throw "invalid_rule_name";
    if (rule.id === 0) ruleRows.push({ ...rule, id: ++ruleSeq });
    else ruleRows.splice(ruleRows.findIndex((r) => r.id === rule.id), 1, rule);
    void emit("phonechat-rules-changed", {});
    return rule;
  },
  phonechat_rule_enable: ({ id, enabled }) => {
    const rule = ruleRows.find((r) => r.id === id);
    if (!rule) throw "rule_not_found";
    rule.enabled = Boolean(enabled);
    return rule;
  },
  phonechat_rule_delete: ({ id }) => {
    ruleRows.splice(ruleRows.findIndex((r) => r.id === id), 1);
    return true;
  },
  phonechat_rules_limits: ({ limits }) => {
    Object.assign(ruleLimits, limits);
    return ruleLimits;
  },
  phonechat_rule_test: ({ text }) => {
    const said = String(text).toLowerCase();
    const rule = ruleRows.find((r) => r.trigger.words?.some((w) => said.includes(w))) ?? null;
    return { rule, reply: rule?.action.text?.replace("{{first_name}}", "Ana") ?? null, opt_out: said.trim() === "sair" };
  },
  phonechat_rule_runs: () => [
    { id: 3, rule_id: 1, chat: ANA, chat_name: "Ana Souza", ts: NOW - 300, outcome: "sent", detail: null },
    { id: 2, rule_id: 1, chat: BRUNO, chat_name: "Bruno", ts: NOW - 900, outcome: "skipped", detail: "human_active" },
    { id: 1, rule_id: 2, chat: LOJA, chat_name: CHAT_NAMES[LOJA], ts: NOW - 4000, outcome: "failed", detail: "ai_silent" },
  ],
  phonechat_opt_in: ({ chat }) => {
    ruleOptOuts.splice(ruleOptOuts.findIndex((o) => o.chat === chat), 1);
    return true;
  },
  phonechat_find: ({ query }) => {
    const q = String(query ?? "").toLowerCase();
    return Object.entries(CHAT_NAMES)
      .filter(([, name]) => name.toLowerCase().includes(q))
      .map(([jid, name]) => ({ jid, name, is_group: jid.endsWith("@g.us"), has_chat: true, last_ts: NOW, unread: 0 }));
  },
  phonechat_wait: async ({ timeoutSecs }) => {
    await sleep(Math.min(Number(timeoutSecs ?? 1), 2) * 1000);
    return { messages: [], cursor: seq, timed_out: true };
  },
  phonechat_schedule: ({ chat, text, at }) => {
    const row: Sched = { id: scheduledRows.length + 1, chat: String(chat), text: String(text), reply_to: null, at: Number(at), created: Math.floor(Date.now() / 1000), status: "pending", error: null, sent_id: null, sent_at: null };
    scheduledRows.push(row);
    void emit("phonechat-scheduled-changed", {});
    return row;
  },
  phonechat_scheduled: ({ includeDone }) => scheduledRows.filter((r) => includeDone || r.status === "pending"),
  phonechat_cancel_scheduled: ({ id }) => {
    const row = scheduledRows.find((r) => r.id === Number(id) && r.status === "pending");
    if (row) row.status = "cancelled";
    void emit("phonechat-scheduled-changed", {});
    return !!row;
  },
  phonechat_request_older: () => {
    requireOnline();
    return null;
  },
  phonechat_media_stats: stats,
  phonechat_default_dir: () => "/preview/Downloads",
  phonechat_download_media: downloadOne,
  phonechat_download_chat_media: downloadChat,
  phonechat_cancel_download: ({ jobId }) => {
    cancelled.add(String(jobId));
    return true;
  },
  phonechat_export_name: ({ chat, format }) => `Phone Chats - ${CHAT_NAMES[String(chat)] ?? "chat"}.${String(format)}`,
  phonechat_export: async ({ chat }) => {
    await sleep(400);
    return (messages[String(chat)] ?? []).length;
  },
  reveal_file: () => null,
  open_path_default: () => null,
};

function onPhoneChat(): boolean {
  return location.pathname.startsWith("/phonechat");
}

export function phonechatDialog(options: Record<string, unknown> | undefined): unknown {
  if (!onPhoneChat()) return undefined;
  if (options?.directory) return "/preview/Downloads";
  return undefined;
}

export function phonechatSaveDialog(options: Record<string, unknown> | undefined): unknown {
  if (!onPhoneChat()) return undefined;
  return typeof options?.defaultPath === "string" ? options.defaultPath : "/preview/Downloads/Phone Chats - chat.txt";
}
