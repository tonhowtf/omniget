import defaults from "./settings-defaults.json";
import { converterDialog, converterFixtures } from "./converter";
import { paletteDialog, paletteFixtures } from "./palette";
import { fontsDialog, fontsFixtures } from "./fonts";
import { iconsDialog, iconsFixtures } from "./icons";
import { motionDialog, motionFixtures, motionSaveDialog } from "./motion";
import { boardDialog, boardFixtures } from "./board";
import { captureDialog, captureFixtures } from "./capture";
import { studioDialog, studioFixtures, studioSaveDialog } from "./studio";
import { dictationDialog, dictationFixtures } from "./dictation";
import { componentsDialog, componentsFixtures } from "./components";
import { phonechatDialog, phonechatFixtures, phonechatSaveDialog } from "./phonechat";
import { marketFixtures } from "./connector-market";
import { guildchatFixtures } from "./guildchat";
import { socialFixtures } from "./social";
import { pdfDialog, pdfFixtures, pdfSaveDialog } from "./pdf";
import { photosDialog, photosFixtures } from "./photos";
import { cutoutDialog, cutoutFixtures } from "./cutout";
import { stemsFixtures } from "./stems";
import { devtoolsFixtures } from "./devtools";
import { proxyPoolFixtures } from "./proxy-pool";
import { transferFixtures } from "./transfer";
import { researchFixtures } from "./research";
import { quickPanelFixtures } from "./quick-panel";

type Args = Record<string, unknown>;
type Fixture = (args: Args) => unknown;
type Json = Record<string, unknown>;

const STORAGE_KEY = "omniget.preview.settings";

function initialSettings(): Json {
  const seed = structuredClone(defaults) as unknown as Json;
  const appearance = seed.appearance as Json;
  appearance.language = navigator.language.toLowerCase().startsWith("pt") ? "pt" : "en";
  (seed.download as Json).default_output_dir = "/preview/Downloads";
  return { ...seed, schema_version: 3, onboarding_completed: true, legal_acknowledged: true };
}

function loadSettings(): Json {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) return JSON.parse(stored) as Json;
  } catch {
    localStorage.removeItem(STORAGE_KEY);
  }
  return initialSettings();
}

let settings = loadSettings();
let revision = 1;

function merge(target: Json, patch: Json) {
  for (const [key, value] of Object.entries(patch)) {
    const current = target[key];
    if (value && typeof value === "object" && !Array.isArray(value) && current && typeof current === "object") {
      merge(current as Json, value as Json);
    } else {
      target[key] = value;
    }
  }
}

function snapshot() {
  return {
    settings: structuredClone(settings),
    revision: String(revision),
    loadStatus: { kind: "loaded" },
    applied: true,
    pendingEffects: [],
    fieldErrors: [],
  };
}

function commit() {
  revision += 1;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
  return snapshot();
}

const empty = () => null;

const PREVIEW_WORKSPACE = {
  id: "preview",
  name: "Home",
  activeFloorId: "main",
  floors: [{ id: "main", name: "Main" }],
};

const CANVAS_DEMO_KEY = "omniget.preview.canvasDemo";

function canvasDemo(): string | null {
  return localStorage.getItem(CANVAS_DEMO_KEY);
}

function approvalsDemo(): string | null {
  return localStorage.getItem("omniget.preview.approvals");
}

const DEMO_NODES = [
  { id: "demo-a", kind: "terminal", x: 80, y: 80, w: 460, h: 300, z: 1, title: "Reviewer", data: { agent: "claude" } },
  { id: "demo-b", kind: "terminal", x: 580, y: 80, w: 640, h: 300, z: 2, title: "Builder", data: { agent: "claude", command: "claude --dangerously-skip-permissions" } },
  { id: "demo-c", kind: "terminal", x: 80, y: 420, w: 460, h: 300, z: 3, title: "Docs", data: { agent: "codex", command: "codex --full-auto" } },
];

function demoInbox() {
  const now = Date.now();
  const row = (nodeId: string, name: string, reason: string, minutes: number) => ({
    nodeId,
    name,
    workspaceId: "preview",
    floorId: "main",
    reason,
    sinceMs: now - minutes * 60000,
  });
  return [row("demo-c", "Docs", "review", 14), row("demo-a", "Reviewer", "permission", 2), row("demo-b", "Builder", "stuck", 9)];
}

function demoBudget() {
  const paused = canvasDemo() === "paused";
  return {
    policies: [
      {
        id: "bp_demo",
        scope: "global",
        metric: "usd",
        window: "day",
        amount: 5,
        warn_pct: 80,
        hard_stop: true,
        enabled: true,
        observed: paused ? 5.02 : 4.3,
        source: "ledger",
        incident: paused ? { id: "bi_demo", state: "paused", observed: 5.02, limit: 5 } : null,
      },
    ],
    incidents: [],
  };
}

export const fixtures: Record<string, Fixture> = {
  settings_get_v3: snapshot,
  settings_patch_v3: ({ request }) => {
    merge(settings, ((request as Json | undefined)?.patch ?? {}) as Json);
    return commit();
  },
  settings_reset_v3: () => {
    settings = initialSettings();
    return commit();
  },

  settings_import_appearance_v3: () => ({ outcome: "already_imported", snapshot: snapshot() }),

  sync_tray_strings: empty,
  usage_tray_sync_strings: empty,
  sync_llm_prompts: empty,
  sync_llm_notify_strings: empty,
  sync_channels_tray: empty,
  native_agent_register: empty,
  llm_ui_focus: empty,
  usage_tray_resize: empty,

  check_ytdlp_available: () => true,
  validate_output_path: () => ({ limit: 260, current: 40, reserve: 40, ok: true }),
  cookies_list: () => ({ registry: { buckets: {} }, cookies_dir: "/preview/cookies" }),
  video_status: () => ({
    server: { port: 0, token: "" },
    roots: [],
    counts: { movies: 0, shows: 0, courses: 0, videos: 0 },
    scanning: false,
    ffmpeg: true,
  }),
  music_status: () => ({
    port: 0,
    token: "",
    roots: [],
    track_count: 0,
    scanning: false,
    ffmpeg: true,
    download_dir: "/preview/Music",
    analysis: [0, 0, false],
  }),
  read_status: () => ({
    port: 0,
    token: "",
    roots: [],
    book_count: 0,
    series_count: 0,
    scanning: false,
    download_dir: "/preview/Books",
    share: { enabled: false },
    share_running: false,
    share_port: null,
  }),
  tube_status: () => ({ account: { logged_in: false }, server: "", ytdlp: true, ffmpeg: true }),
  tube_home: () => ({ chips: [], shelves: [] }),
  washmybelly_capabilities: () => ({ os: "macos", modules: [] }),
  gd_status: () => ({
    browser: { open: false, mode: "embedded", installed: [], tabs: [] },
    connectors: [],
    invalid_connectors: [],
    jobs: [],
    out_dir: "/preview/Downloads",
  }),
  ...marketFixtures,
  failed_download_leftovers: () => [
    { path: "/preview/Downloads/Clip [abc].f137.mp4.part", name: "Clip [abc].f137.mp4.part", size_bytes: 52_428_800 },
    { path: "/preview/Downloads/Clip [abc].f137.mp4.ytdl", name: "Clip [abc].f137.mp4.ytdl", size_bytes: 512 },
  ],
  imported_archive_count: () => 2,
  import_ytdlp_archive: () => ({ read: 2, added: 2 }),
  export_ytdlp_archive: () => ({ written: 2, skipped: 0 }),
  detect_platform: ({ url }) => {
    const value = String(url ?? "");
    const youtube = /youtube\.com|youtu\.be/.test(value);
    const isList = youtube && /[?&]list=/.test(value);
    return {
      platform: youtube ? "youtube" : "generic",
      supported: true,
      content_id: null,
      content_type: isList ? "playlist" : youtube ? "video" : null,
    };
  },
  playlist_entries: () =>
    Array.from({ length: 60 }, (_, i) => ({
      index: i + 1,
      title: `Preview item ${i + 1}`,
      url: `https://www.youtube.com/watch?v=preview${i + 1}`,
    })),
  league_status: () => ({ connected: false, port: null, region: null }),
  canvas_companion_settings_get: () => ({
    enabled: false,
    minWorkSecs: 60,
    modelMode: "off",
    ollamaUrl: "",
    petBalloon: false,
    onDone: false,
    onBlocked: false,
    onFailed: false,
  }),
  canvas_companion_history: () => [],
  canvas_workspaces: () => [PREVIEW_WORKSPACE],
  canvas_load: () => ({ seq: 0, nodes: canvasDemo() ? DEMO_NODES : [], connections: [], groups: [], drawings: [] }),
  canvas_inbox: () => (canvasDemo() ? demoInbox() : []),
  canvas_checkpoints: () =>
    canvasDemo()
      ? { repository: true, items: [{ turn: 7, atMs: Date.now() - 180000 }, { turn: 6, atMs: Date.now() - 1080000 }] }
      : { repository: false, items: [] },
  canvas_checkpoint_changes: ({ turn }) => ({ turn, total: 9, files: ["src/lib/a.ts", "src/lib/b.ts", "src/routes/+page.svelte", "README.md", "package.json", "src/app.css", "src/x.ts"], truncated: false }),
  canvas_checkpoint_restore: () => ({ turn: 7, files: ["a", "b", "c", "d"], dropped_later: 0 }),
  canvas_checkpoint_undo: () => ({ turn: 1, files: ["a", "b", "c", "d"], dropped_later: 0 }),
  tool_mcp_auth_requests: () =>
    approvalsDemo() === "corner"
      ? [{ id: "auth-demo", client: "Claude Code", platform: "youtube", host: null, reason: "The video is age-restricted", created_at: Date.now() / 1000, expires_at: Date.now() / 1000 + 600 }]
      : [],
  llm_tool_asks_pending: () =>
    approvalsDemo() === "corner"
      ? [{ request_id: "req-demo", tool_call_id: "call-demo", tool: "fs_apply_patch", agent: "Reviewer", kind: "file_change", asked_ms: Date.now() - 20000, deadline_ms: null, can_always: true, preview: "src/lib/a.ts", impact: { files: 3, added: 42, removed: 7, outside: true } }]
      : [],
  connect_pending: () =>
    approvalsDemo() === "dialog"
      ? [{ id: "connect-demo", client: "claude-code", client_name: "Claude Code", workspace: "/preview/project", browser: false, code: "4821", created_ms: Date.now() }]
      : [],
  llm_budget_state: () => (canvasDemo() ? demoBudget() : { policies: [], incidents: [] }),
  llm_budget_resolve: () => {
    localStorage.setItem(CANVAS_DEMO_KEY, "1");
    return { incident: { state: "resolved" }, resumed_loops: [] };
  },
  canvas_apply: ({ baseRev }) => ({ rev: Number(baseRev ?? 0) + 1 }),
  canvas_session_mark: empty,
  "plugin:path|resolve_directory": () => "/preview",
  profile_get: () => ({
    public_key_b64: "",
    fingerprint: "",
    nickname: "Preview",
    skin: { id: "default", tint: [255, 159, 10] },
  }),
  telegram_call: () => {
    throw new Error("unknown telegram command");
  },

  "plugin:app|version": () => "0.0.0-preview",
  "plugin:app|name": () => "OmniGet",
  "plugin:app|tauri_version": () => "2.0.0",
  "plugin:updater|check": empty,
  "plugin:window|is_maximized": () => false,
  "plugin:window|is_fullscreen": () => false,
  "plugin:window|is_focused": () => true,
  "plugin:window|is_visible": () => true,
  "plugin:window|scale_factor": () => devicePixelRatio,
  "plugin:window|inner_size": () => ({ width: innerWidth, height: innerHeight }),
  "plugin:window|available_monitors": () => [],
  "plugin:dialog|open": ({ options }) => {
    const o = options as Record<string, unknown> | undefined;
    for (const dialog of [converterDialog, paletteDialog, fontsDialog, iconsDialog, motionDialog, boardDialog, captureDialog, studioDialog, dictationDialog, componentsDialog, phonechatDialog, pdfDialog, photosDialog, cutoutDialog]) {
      const picked = dialog(o);
      if (picked !== undefined) return picked;
    }
    return null;
  },
  "plugin:dialog|save": ({ options }) => {
    const o = options as Record<string, unknown> | undefined;
    for (const dialog of [motionSaveDialog, studioSaveDialog, phonechatSaveDialog, pdfSaveDialog]) {
      const picked = dialog(o);
      if (picked !== undefined) return picked;
    }
    return null;
  },
  "plugin:dialog|ask": () => confirm("Preview: confirm?"),
  "plugin:dialog|confirm": () => confirm("Preview: confirm?"),
  "plugin:dialog|message": empty,
  "plugin:clipboard-manager|read_text": () => "",
  "plugin:clipboard-manager|write_text": ({ text }) => navigator.clipboard?.writeText(String(text ?? "")).catch(() => null) ?? null,
  "plugin:shell|open": ({ path }) => {
    window.open(String(path), "_blank", "noopener");
    return null;
  },
  ...converterFixtures,
  ...paletteFixtures,
  ...fontsFixtures,
  ...iconsFixtures,
  ...motionFixtures,
  ...boardFixtures,
  ...captureFixtures,
  ...studioFixtures,
  ...dictationFixtures,
  ...componentsFixtures,
  ...phonechatFixtures,
  ...guildchatFixtures,
  ...socialFixtures,
  ...pdfFixtures,
  ...photosFixtures,
  ...cutoutFixtures,
  ...stemsFixtures,
  ...devtoolsFixtures,
  ...proxyPoolFixtures,
  ...transferFixtures,
  ...researchFixtures,
  ...quickPanelFixtures,
};
