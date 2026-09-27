import { describe, expect, it } from "vitest";
import type { ProcessRow } from "./monitor";
import {
  appliedKeys,
  candidates,
  explainKey,
  gameErrorKey,
  installLine,
  isHelper,
  macGameMode,
  methodKey,
  minutesSince,
  statusPill,
  windowsAutoGameMode,
  type GameStatus,
} from "./game";

function proc(p: Partial<ProcessRow>): ProcessRow {
  return {
    pid: 100,
    name: "Game",
    cpu_percent: 0,
    memory_bytes: 0,
    disk_read_per_s: 0,
    disk_write_per_s: 0,
    command: "",
    started: 1,
    status: "Run",
    mine: true,
    can_end: true,
    ...p,
  };
}

function status(p: Partial<GameStatus>): GameStatus {
  return { os: "linux", backend: "native", available: true, can_force: false, active: [], ...p };
}

describe("candidates", () => {
  const list = [
    proc({ pid: 10, name: "Cyberpunk2077.exe", cpu_percent: 60, memory_bytes: 6e9 }),
    proc({ pid: 11, name: "Google Chrome Helper (Renderer)", cpu_percent: 40 }),
    proc({ pid: 12, name: "yt-dlp", cpu_percent: 30 }),
    proc({ pid: 13, name: "hl2_linux", cpu_percent: 25, memory_bytes: 1e9 }),
    proc({ pid: 14, name: "idle app", cpu_percent: 0.1, memory_bytes: 50e6 }),
    proc({ pid: 15, name: "SomeonesGame", cpu_percent: 50, mine: false }),
    proc({ pid: 16, name: "OmniGet", cpu_percent: 20 }),
    proc({ pid: 17, name: "Factorio", cpu_percent: 1, memory_bytes: 2e9 }),
  ];

  it("keeps busy processes of this user and drops helpers, tools and OmniGet", () => {
    const c = candidates(list, []);
    expect(c.map((x) => x.pid)).toEqual([10, 13, 17]);
  });

  it("hides what is already in game mode", () => {
    expect(candidates(list, [10]).map((x) => x.pid)).toEqual([13, 17]);
  });

  it("respects the limit", () => {
    expect(candidates(list, [], 1)).toHaveLength(1);
  });

  it("helper detection", () => {
    expect(isHelper("Discord Helper (GPU)")).toBe(true);
    expect(isHelper("explorer.exe")).toBe(true);
    expect(isHelper("witcher3.exe")).toBe(false);
  });
});

describe("status pill and explanation", () => {
  it("on when something is active", () => {
    const s = status({ active: [{ pid: 1, name: "x", method: "native", since: 0, applied: [], notes: [], children_lowered: 0, auto_revert: "on_process_exit" }] });
    expect(statusPill(s)).toEqual({ key: "washmybelly.games.pill.on", tone: "success" });
  });

  it("linux without gamemode is basic and explains the fallback", () => {
    const s = status({ backend: "native", reason: "gamemode_missing" });
    expect(statusPill(s).tone).toBe("warning");
    expect(explainKey(s)).toBe("washmybelly.games.explain.linux_native");
  });

  it("linux with gamemode and portal", () => {
    expect(explainKey(status({ backend: "feral_gamemode" }))).toBe("washmybelly.games.explain.linux_gamemode");
    expect(explainKey(status({ backend: "portal" }))).toBe("washmybelly.games.explain.linux_portal");
    expect(statusPill(status({ backend: "feral_gamemode" })).key).toBe("washmybelly.games.pill.ready");
  });

  it("mac is automatic, and says on only when the system says so", () => {
    const off = status({ os: "macos", backend: "macos", system: { game_mode_on: false } });
    const on = status({ os: "macos", backend: "macos", system: { game_mode_on: true } });
    const unknown = status({ os: "macos", backend: "macos", system: { game_mode_on: null } });
    expect(statusPill(off).key).toBe("washmybelly.games.pill.automatic");
    expect(statusPill(on).key).toBe("washmybelly.games.pill.system_on");
    expect(macGameMode(unknown)).toBeNull();
    expect(explainKey(off)).toBe("washmybelly.games.explain.macos");
  });

  it("windows game mode setting", () => {
    expect(windowsAutoGameMode(status({ os: "windows", backend: "windows", system: { auto_game_mode: false } }))).toBe(false);
    expect(windowsAutoGameMode(status({ os: "windows", backend: "windows", system: {} }))).toBeNull();
  });
});

describe("small helpers", () => {
  it("install line", () => {
    expect(installLine({ manager: "apt", argv: ["sudo", "apt", "install", "gamemode"] })).toBe("sudo apt install gamemode");
    expect(installLine(null)).toBe("");
  });

  it("applied keys keep a stable order and drop unknown codes", () => {
    expect(appliedKeys(["display_awake", "power_plan", "mystery"])).toEqual(["washmybelly.games.applied.power_plan", "washmybelly.games.applied.display_awake"]);
  });

  it("method keys", () => {
    expect(methodKey("feral_gamemode")).toBe("washmybelly.games.method.feral_gamemode");
    expect(methodKey("zzz")).toBe("washmybelly.games.method.other");
  });

  it("minutes since", () => {
    expect(minutesSince(1000, 1000 * 1000 + 125_000)).toBe(2);
    expect(minutesSince(2000, 1000 * 1000)).toBe(0);
  });

  it("error keys", () => {
    expect(gameErrorKey("REFUSED: the process belongs to another user")).toBe("washmybelly.games.error.refused");
    expect(gameErrorKey("NOT_ACTIVE: x")).toBe("washmybelly.games.error.not_active");
    expect(gameErrorKey("boom")).toBeNull();
  });
});
