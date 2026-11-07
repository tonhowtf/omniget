import { emit } from "@tauri-apps/api/event";
import { captureCombo } from "$lib/dictation/logic";

type Args = Record<string, unknown>;
type Json = Record<string, unknown>;

const scenario = (): string => new URLSearchParams(location.search).get("scenario") ?? "";
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

const MODELS = [
  { id: "whisper-tiny", label: "Whisper tiny (fast, ~45 MB)", bytes: 41_884_787, downloaded: false, default: false },
  { id: "whisper-base", label: "Whisper base (balanced, ~80 MB)", bytes: 77_934_045, downloaded: true, default: true },
  { id: "whisper-small", label: "Whisper small (accurate, ~250 MB)", bytes: 250_120_000, downloaded: false, default: false },
];

const DEVICES = [
  { id: "Microsoft Teams Audio", name: "Microsoft Teams Audio", index: 0 },
  { id: "MacBook Air Microphone", name: "MacBook Air Microphone", index: 1 },
  { id: "CORSAIR VIRTUOSO XT Wireless Gaming Receiver", name: "CORSAIR VIRTUOSO XT Wireless Gaming Receiver", index: 2 },
];

const SAMPLES = [
  "Remind me to send the invoice to Marta before Friday.",
  "Open the downloads folder and sort everything by date.",
  "Ok, so the plan is: record the intro first, then the demo, then the outro.",
  "Vamos marcar a revisão para quinta às três da tarde.",
];

let settings: Json = {
  enabled: true,
  shortcut: "Alt+Space",
  mode: "hold",
  device: null,
  language: "auto",
  model: "whisper-base",
  output: "paste",
  restore_clipboard: true,
  capitalize: true,
  remove_fillers: true,
  replacements: [{ find: "omniget", replace: "OmniGet" }],
  max_seconds: 120,
  save_history: true,
};

const now = Date.now();
let history: Json[] = [
  { id: "h1", text: SAMPLES[0], raw: SAMPLES[0], created_at: now - 4 * 60_000, duration_ms: 3_900, language: "en", model: "whisper-base", outcome: "pasted" },
  { id: "h2", text: SAMPLES[3], raw: SAMPLES[3], created_at: now - 52 * 60_000, duration_ms: 4_600, language: "pt", model: "whisper-base", outcome: "copied" },
  {
    id: "h3",
    text: "Draft for the release notes: the converter now handles HEIC photos, the queue remembers paused downloads after a restart, and the settings page loads twice as fast on older machines.",
    raw: "",
    created_at: now - 26 * 3_600_000,
    duration_ms: 14_200,
    language: "en",
    model: "whisper-base",
    outcome: "pasted",
  },
];

let session = 0;
let live: { session: number; source: "page" | "shortcut"; started: number; timer: ReturnType<typeof setInterval> } | null = null;
let last: Json = idle(0);
let installing = false;
let modelReady = typeof location === "undefined" || scenario() !== "setup";
let takes = 0;

function idle(s: number, notice: string | null = null): Json {
  return { phase: "idle", session: s, source: "page", started_at: null, max_ms: 0, entry: null, outcome: null, error: null, notice };
}

async function send(ev: Json) {
  last = ev;
  await emit("dictation-state", ev);
}

function status(): Json {
  return {
    settings: structuredClone(settings),
    state: last,
    platform: "macos",
    ffmpeg: scenario() !== "noffmpeg",
    paste_supported: true,
    runtime_installed: modelReady,
    runtime_downloadable: true,
    model_ready: modelReady && MODELS.find((m) => m.id === settings.model)?.downloaded !== false,
    models: MODELS,
    installing,
    shortcut: { enabled: settings.enabled, shortcut: settings.shortcut, registered: !!settings.enabled, paused: false, error: null },
    default_shortcut: "Alt+Space",
  };
}

function normalize(raw: string): { normalized: string | null; error: string | null } {
  const parts = raw.split("+").map((p) => p.trim()).filter(Boolean);
  const mods = parts.filter((p) => ["Ctrl", "Alt", "Shift", "Super"].includes(p));
  const keys = parts.filter((p) => !mods.includes(p));
  if (keys.length === 0) return { normalized: null, error: "NO_KEY" };
  if (keys.length > 1) return { normalized: null, error: "TOO_MANY_KEYS" };
  const key = keys[0];
  if (key === "Escape") return { normalized: null, error: "RESERVED" };
  if (mods.length === 1 && mods[0] === "Super" && ["Space", "Q", "W", "Tab", "C", "V", "X", "Z", "A", "H", "M"].includes(key)) return { normalized: null, error: "RESERVED" };
  if (mods.join("+") === "Super+Shift" || raw === "Shift+Super+O") return { normalized: null, error: "CONFLICT_APP" };
  const fnLone = /^F(1[3-9]|2[0-4])$/.test(key);
  if (!mods.some((m) => m !== "Shift") && !fnLone) return { normalized: null, error: "NEEDS_MODIFIER" };
  if (raw === "Ctrl+Alt+T") return { normalized: null, error: "TAKEN" };
  const order = ["Ctrl", "Alt", "Shift", "Super"];
  return { normalized: [...order.filter((m) => mods.includes(m)), key].join("+"), error: null };
}

function postprocess(text: string, s: Json): string {
  let out = text.trim().replace(/\s+/g, " ");
  if (s.remove_fillers) out = out.replace(/\b(u+m+|u+h+|h+m+|e+r+m+)\b[,.…]*\s*/gi, "").replace(/(^|[,.!?]\s*)(like|tipo|é),\s*/gi, "$1");
  for (const r of (s.replacements as { find: string; replace: string }[]) ?? []) {
    if (!r.find.trim()) continue;
    const esc = r.find.trim().replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    out = out.replace(new RegExp(`(?<![\\p{L}\\p{N}_])${esc}(?![\\p{L}\\p{N}_])`, "giu"), r.replace);
  }
  out = out.replace(/\s+([,.!?])/g, "$1").replace(/^[,\s]+/, "").trim();
  if (s.capitalize) out = out.replace(/(^|[.!?]\s+)(\p{L})/gu, (_, a: string, b: string) => a + b.toLocaleUpperCase());
  return out;
}

async function start(source: "page" | "shortcut"): Promise<Json> {
  if (live) throw "BUSY";
  session += 1;
  const s = session;
  if (!status().model_ready) {
    const ev = { ...idle(s), phase: "error", source, error: { code: "MODEL_MISSING", detail: null } };
    await send(ev);
    throw "MODEL_MISSING";
  }
  const started = Date.now();
  const maxMs = Number(settings.max_seconds) * 1000;
  let t = 0;
  const timer = setInterval(() => {
    t += 1;
    const speaking = Math.sin(t / 3) > -0.3;
    const level = speaking ? 0.35 + Math.random() * 0.55 : Math.random() * 0.12;
    void emit("dictation-level", { session: s, level });
    if (Date.now() - started >= maxMs) void finish(s, "auto");
  }, 50);
  live = { session: s, source, started, timer };
  const ev = { ...idle(s), phase: "listening", source, started_at: started, max_ms: maxMs };
  await send(ev);
  return ev;
}

async function finish(s: number, how: "stop" | "cancel" | "auto") {
  if (!live || live.session !== s) return;
  const { source, started, timer } = live;
  clearInterval(timer);
  live = null;
  if (how === "cancel") {
    await send({ ...idle(s, "CANCELLED"), source });
    return;
  }
  if (source === "shortcut" && settings.mode === "hold" && Date.now() - started < 300) {
    await send({ ...idle(s, "HOLD_HINT"), source });
    return;
  }
  await send({ ...idle(s), phase: "transcribing", source, notice: how === "auto" ? "MAX_REACHED" : null });
  await sleep(900);
  takes += 1;
  const sc = scenario();
  if (sc === "silent" && takes % 2 === 1) {
    await send({ ...idle(s), phase: "error", source, error: { code: "SILENT", detail: null } });
    return;
  }
  if (Date.now() - started < 500) {
    await send({ ...idle(s), phase: "error", source, error: { code: "TOO_SHORT", detail: null } });
    return;
  }
  const raw = `um, ${SAMPLES[takes % SAMPLES.length]}`;
  const text = postprocess(raw, settings);
  const denied = sc === "denied" && source === "shortcut" && settings.output === "paste";
  const outcome = source === "page" ? "inserted" : denied ? "copied" : settings.output === "copy" ? "copied" : "pasted";
  const entry = {
    id: `m${Date.now()}`,
    text,
    raw,
    created_at: Date.now(),
    duration_ms: Date.now() - started,
    language: text.startsWith("Vamos") ? "pt" : "en",
    model: settings.model,
    outcome,
  };
  if (settings.save_history) history = [entry, ...history].slice(0, 200);
  await send({
    ...idle(s),
    phase: "done",
    source,
    entry,
    outcome,
    error: denied ? { code: "PASTE_ACCESSIBILITY", detail: "osascript is not allowed to send keystrokes. (1002)" } : null,
  });
}

async function install(args: Args) {
  if (installing) throw "BUSY";
  installing = true;
  const model = String(args.model ?? settings.model);
  for (const [stage, steps] of [["runtime", 6], ["model", 14]] as const) {
    for (let i = 1; i <= steps; i++) {
      await sleep(140);
      await emit("dictation-model-progress", { model, stage, fraction: i / steps });
    }
  }
  const m = MODELS.find((x) => x.id === model);
  if (m) m.downloaded = true;
  modelReady = true;
  installing = false;
  return status();
}

function onPreviewKey(down: boolean) {
  return (e: KeyboardEvent) => {
    if (!location.pathname.startsWith("/dictation") || !settings.enabled) return;
    const target = e.target as HTMLElement | null;
    if (target?.closest(".dc-rec")) return;
    if (down) {
      const c = captureCombo(e);
      if (c.combo !== settings.shortcut || e.repeat) return;
      e.preventDefault();
      if (settings.mode === "toggle" && live) void finish(live.session, "stop");
      else if (!live) void start("shortcut").catch(() => {});
    } else if (settings.mode === "hold" && live?.source === "shortcut") {
      const key = String(settings.shortcut).split("+").pop();
      const code = e.code.replace(/^Key/, "").replace(/^Digit/, "");
      if (code === key || e.key === "Alt" || e.key === "Control" || e.key === "Meta") void finish(live.session, "stop");
    }
  };
}

if (typeof window !== "undefined") {
  window.addEventListener("keydown", onPreviewKey(true), true);
  window.addEventListener("keyup", onPreviewKey(false), true);
}

export const dictationFixtures: Record<string, (args: Args) => unknown> = {
  dictation_status: () => status(),
  dictation_save_settings: (args) => {
    const next = structuredClone(args.settings as Json);
    if (next.enabled && next.shortcut !== settings.shortcut) {
      const check = normalize(String(next.shortcut));
      if (check.error) throw check.error;
      next.shortcut = check.normalized;
    }
    settings = next;
    return status();
  },
  dictation_check_shortcut: ({ shortcut }) => normalize(String(shortcut ?? "")),
  dictation_pause_shortcut: ({ paused }) => ({ enabled: settings.enabled, shortcut: settings.shortcut, registered: !paused, paused: !!paused, error: null }),
  dictation_devices: async () => {
    await sleep(250);
    return DEVICES;
  },
  dictation_start: () => start("page"),
  dictation_stop: () => (live ? finish(live.session, "stop") : null),
  dictation_cancel: () => (live ? finish(live.session, "cancel") : null),
  dictation_history: () => (scenario() === "first" ? (history = []) : history),
  dictation_history_delete: ({ id }) => (history = history.filter((h) => h.id !== id)),
  dictation_history_clear: () => {
    history = [];
    return null;
  },
  dictation_copy: ({ text }) => navigator.clipboard?.writeText(String(text ?? "")).catch(() => null) ?? null,
  dictation_preview: ({ text, settings: s }) => postprocess(String(text ?? ""), s as Json),
  dictation_open_privacy: () => null,
  dictation_install_model: install,
};

export function dictationDialog(_options: Record<string, unknown> | undefined): unknown {
  return undefined;
}
