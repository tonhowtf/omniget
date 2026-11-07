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
};

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
    ...partial,
  };
}

const messages: Record<string, Msg[]> = {
  [FAMILY]: [
    m(FAMILY, 3 * D, { text: "Fotos do almoço de domingo 👇", sender_name: "Tia Lúcia" }),
    m(FAMILY, 3 * D - 60, { kind: "image", mime: "image/jpeg", size: 284_000, thumb: THUMB, sender_name: "Tia Lúcia" }),
    m(FAMILY, 3 * D - 58, { kind: "image", mime: "image/jpeg", size: 312_500, thumb: THUMB, sender_name: "Tia Lúcia", text: "A mesa ficou linda" }),
    m(FAMILY, 2 * D, { kind: "video", mime: "video/mp4", size: 18_400_000, duration: 47, thumb: THUMB, sender_name: "Pedro" }),
    m(FAMILY, 2 * D - 300, { from_me: true, sender_name: null, text: "Que demais! Manda o vídeo do parabéns também" }),
    m(FAMILY, 26 * H, { kind: "voice", mime: "audio/ogg; codecs=opus", size: 41_000, duration: 23, sender_name: "Vó Maria" }),
    m(FAMILY, 25 * H, { kind: "document", mime: "application/pdf", size: 1_240_000, file_name: "Receita do bolo.pdf", sender_name: "Tia Lúcia" }),
    m(FAMILY, 3 * H, { kind: "sticker", mime: "image/webp", size: 18_000, sender_name: "Pedro" }),
    m(FAMILY, 2 * H, { text: "Domingo que vem é aqui em casa!", sender_name: "Vó Maria" }),
  ],
  [ANA]: [
    m(ANA, 5 * D, { text: "Oi! Conseguiu ver o contrato?" }),
    m(ANA, 5 * D - 120, { from_me: true, sender_name: null, text: "Vi sim, já assinei" }),
    m(ANA, 5 * D - 100, { from_me: true, sender_name: null, kind: "document", mime: "application/pdf", size: 820_000, file_name: "Contrato assinado.pdf", saved_path: "/preview/Downloads/WhatsApp/Ana Souza/2025-01-10 Contrato assinado.pdf" }),
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
  await emit("whatsapp-status", status());
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
  return `/preview/Downloads/WhatsApp/${CHAT_NAMES[chat]}/${new Date(x.ts * 1000).toISOString().slice(0, 10)} ${x.file_name ?? `${x.kind} ${x.id.slice(0, 10)}${ext}`}`;
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
  await emit("whatsapp-download-progress", { job_id: null, chat, message_id: id, stage: "start" });
  await sleep(700);
  if (x.ts < NOW - 4 * D) {
    await emit("whatsapp-download-progress", { job_id: null, chat, message_id: id, stage: "retrying" });
    await sleep(3000);
    if (x.ts < NOW - 15 * D || x.kind === "audio") {
      await emit("whatsapp-download-progress", { job_id: null, chat, message_id: id, stage: "error", error: "media_gone" });
      throw "media_gone";
    }
  }
  x.saved_path = savedPath(chat, x);
  await emit("whatsapp-download-progress", { job_id: null, chat, message_id: id, stage: "done", path: x.saved_path });
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
  const dir = `/preview/Downloads/WhatsApp/${CHAT_NAMES[chat]}`;
  void (async () => {
    let done = 0;
    let skipped = 0;
    let failed = 0;
    let bytes = 0;
    const errors: Record<string, number> = {};
    for (const [index, x] of rows.entries()) {
      if (cancelled.has(jobId)) break;
      await emit("whatsapp-download-progress", { job_id: jobId, chat, message_id: x.id, stage: "file", index, total: rows.length, done, skipped, failed, bytes });
      await sleep(450);
      if (!x.saved_path && x.ts < NOW - 2 * D) {
        await emit("whatsapp-download-progress", { job_id: jobId, chat, message_id: x.id, stage: "retrying" });
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
    await emit("whatsapp-download-done", { job_id: jobId, chat, total: rows.length, done, skipped, failed, bytes, dir, cancelled: cancelled.has(jobId), errors });
  })();
  return { job_id: jobId, total: rows.length, dir };
}

export const whatsappFixtures: Record<string, (args: Args) => unknown> = {
  whatsapp_status: () => status(),
  whatsapp_connect: async () => {
    if (stage === "unlinked") {
      pairing = true;
      await pushStatus();
      setTimeout(() => {
        if (!pairing || stage !== "unlinked") return;
        stage = "connected";
        pairing = false;
        void pushStatus();
        void emit("whatsapp-chats-changed", {});
      }, 9000);
    } else if (stage === "offline") {
      stage = "connected";
      await pushStatus();
    }
    return status();
  },
  whatsapp_disconnect: async () => {
    pairing = false;
    pairCode = null;
    await pushStatus();
    return status();
  },
  whatsapp_pair_phone: async (args) => {
    const digits = String(args.phone ?? "").replace(/\D/g, "");
    if (digits.length < 8) throw "invalid_phone";
    await sleep(600);
    pairing = true;
    pairCode = "K7Q2M9XD";
    await pushStatus();
    return pairCode;
  },
  whatsapp_logout: async () => {
    stage = "unlinked";
    pairing = false;
    pairCode = null;
    await pushStatus();
    return status();
  },
  whatsapp_chats: chatRows,
  whatsapp_messages: (args) => {
    const chat = String(args.chat);
    const before = typeof args.before === "number" ? args.before : Infinity;
    const limit = typeof args.limit === "number" ? args.limit : 80;
    const list = (messages[chat] ?? []).filter((x) => x.ts < before);
    return list.slice(Math.max(0, list.length - limit));
  },
  whatsapp_mark_read: ({ chat }) => {
    if (stage !== "connected") throw "offline";
    const n = unread[String(chat)] ?? 0;
    unread[String(chat)] = 0;
    void emit("whatsapp-chats-changed", {});
    return n;
  },
  whatsapp_send_text: async (args) => {
    requireOnline();
    const chat = String(args.chat);
    const row = m(chat, 0, { from_me: true, sender_name: null, text: String(args.text ?? "") });
    (messages[chat] ??= []).push(row);
    await sleep(250);
    void emit("whatsapp-chats-changed", {});
    return row;
  },
  whatsapp_request_older: () => {
    requireOnline();
    return null;
  },
  whatsapp_media_stats: stats,
  whatsapp_default_dir: () => "/preview/Downloads",
  whatsapp_download_media: downloadOne,
  whatsapp_download_chat_media: downloadChat,
  whatsapp_cancel_download: ({ jobId }) => {
    cancelled.add(String(jobId));
    return true;
  },
  whatsapp_export_name: ({ chat, format }) => `WhatsApp - ${CHAT_NAMES[String(chat)] ?? "chat"}.${String(format)}`,
  whatsapp_export: async ({ chat }) => {
    await sleep(400);
    return (messages[String(chat)] ?? []).length;
  },
  reveal_file: () => null,
  open_path_default: () => null,
};

function onWhatsapp(): boolean {
  return location.pathname.startsWith("/whatsapp");
}

export function whatsappDialog(options: Record<string, unknown> | undefined): unknown {
  if (!onWhatsapp()) return undefined;
  if (options?.directory) return "/preview/Downloads";
  return undefined;
}

export function whatsappSaveDialog(options: Record<string, unknown> | undefined): unknown {
  if (!onWhatsapp()) return undefined;
  return typeof options?.defaultPath === "string" ? options.defaultPath : "/preview/Downloads/WhatsApp - chat.txt";
}
