import type { ProcessRow } from "./monitor";

type GameBackend = "feral_gamemode" | "portal" | "native" | "windows" | "macos" | "none";

type GameActive = {
  pid: number;
  name: string;
  method: string;
  since: number;
  applied: string[];
  notes: string[];
  children_lowered: number;
  auto_revert: string;
};

export type GameStatus = {
  os: "macos" | "windows" | "linux";
  backend: GameBackend;
  available: boolean;
  can_force: boolean;
  reason?: string | null;
  system?: Record<string, unknown>;
  install?: { manager: string; argv: string[] } | null;
  offers?: string[];
  fallback?: string[];
  active: GameActive[];
};

export type Candidate = ProcessRow & { score: number };

const NOT_GAMES = [
  "omniget",
  "yt-dlp",
  "ffmpeg",
  "ffprobe",
  "node",
  "python",
  "python3",
  "zsh",
  "bash",
  "fish",
  "sh",
  "pwsh",
  "powershell.exe",
  "cmd.exe",
  "conhost.exe",
  "explorer.exe",
  "dwm.exe",
  "svchost.exe",
  "searchhost.exe",
  "msmpeng.exe",
  "windowserver",
  "kernel_task",
  "launchd",
  "mds",
  "mds_stores",
  "mdworker",
  "spotlight",
  "finder",
  "dock",
  "systemd",
  "xorg",
  "xwayland",
  "gnome-shell",
  "kwin_wayland",
  "kwin_x11",
  "plasmashell",
  "pipewire",
  "pulseaudio",
  "claude",
  "codex",
  "gemini",
  "ollama",
];

const HELPER_HINTS = ["helper", "renderer", "gpu process", "crashpad", "webview", "updater", "daemon", "agent", "service", "worker"];

function isHelper(name: string): boolean {
  const n = name.toLowerCase();
  if (NOT_GAMES.includes(n) || NOT_GAMES.includes(n.replace(/\.exe$/, ""))) return true;
  return HELPER_HINTS.some((h) => n.includes(h));
}

const MIN_CPU = 5;
const MIN_MEMORY = 400 * 1024 * 1024;

export function candidates(list: ProcessRow[], activePids: number[], limit = 12): Candidate[] {
  const active = new Set(activePids);
  return list
    .filter((p) => p.mine && p.pid > 1 && !active.has(p.pid) && !isHelper(p.name))
    .filter((p) => p.cpu_percent >= MIN_CPU || p.memory_bytes >= MIN_MEMORY)
    .map((p) => ({ ...p, score: p.cpu_percent + (p.memory_bytes / (1024 * 1024 * 1024)) * 10 }))
    .sort((a, b) => b.score - a.score)
    .slice(0, limit);
}

export type Tone = "success" | "info" | "warning" | "default";

export function statusPill(s: GameStatus): { key: string; tone: Tone } {
  if (s.active.length) return { key: "washmybelly.games.pill.on", tone: "success" };
  switch (s.backend) {
    case "feral_gamemode":
    case "portal":
      return { key: "washmybelly.games.pill.ready", tone: "info" };
    case "windows":
      return { key: "washmybelly.games.pill.ready", tone: "info" };
    case "macos":
      return systemOn(s) ? { key: "washmybelly.games.pill.system_on", tone: "success" } : { key: "washmybelly.games.pill.automatic", tone: "default" };
    case "native":
      return { key: "washmybelly.games.pill.basic", tone: "warning" };
    default:
      return { key: "washmybelly.games.pill.unavailable", tone: "default" };
  }
}

export function explainKey(s: GameStatus): string {
  if (s.os === "macos") return "washmybelly.games.explain.macos";
  if (s.os === "windows") return "washmybelly.games.explain.windows";
  if (s.backend === "portal") return "washmybelly.games.explain.linux_portal";
  if (s.backend === "feral_gamemode") return "washmybelly.games.explain.linux_gamemode";
  return "washmybelly.games.explain.linux_native";
}

function systemOn(s: GameStatus): boolean {
  return s.system?.["game_mode_on"] === true;
}

export function macGameMode(s: GameStatus): boolean | null {
  const v = s.system?.["game_mode_on"];
  return typeof v === "boolean" ? v : null;
}

export function windowsAutoGameMode(s: GameStatus): boolean | null {
  const v = s.system?.["auto_game_mode"];
  return typeof v === "boolean" ? v : null;
}

export function installLine(install: GameStatus["install"]): string {
  return install?.argv?.join(" ") ?? "";
}

export function methodKey(method: string): string {
  const known = ["feral_gamemode", "portal", "native", "windows", "macos"];
  return `washmybelly.games.method.${known.includes(method) ? method : "other"}`;
}

const APPLIED = [
  "gamemode",
  "power_profile",
  "screensaver_inhibit",
  "children_lowered",
  "children_background",
  "watch_children",
  "game_out_of_background",
  "power_plan",
  "priority",
  "ecoqos_off",
  "display_awake",
];

export function appliedKeys(applied: string[]): string[] {
  return APPLIED.filter((a) => applied.includes(a)).map((a) => `washmybelly.games.applied.${a}`);
}

export function minutesSince(since: number, nowMs = Date.now()): number {
  return Math.max(0, Math.floor((nowMs / 1000 - since) / 60));
}

export function gameErrorKey(e: unknown): string | null {
  const s = String(e);
  for (const c of ["REFUSED", "NOT_FOUND", "NOT_ACTIVE", "NOTHING_APPLIED", "TIMEOUT"]) {
    if (s.includes(`${c}:`)) return `washmybelly.games.error.${c.toLowerCase()}`;
  }
  return null;
}
