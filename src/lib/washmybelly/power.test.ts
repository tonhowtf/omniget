import { describe, expect, it } from "vitest";
import {
  actionsOf,
  capacityBars,
  chargeChoices,
  formatCountdown,
  groupBlockers,
  healthTone,
  holdProgress,
  managerKey,
  profileOptions,
  remainingSecs,
  type PowerStatus,
} from "./power";

const linux: PowerStatus = {
  os: "linux",
  profile: { current: "balanced", available: ["performance", "power-saver", "balanced"], can_switch: true },
  actions: [
    { id: "power:charge_limit:BAT0:80", label: "", admin: true, risk: "reversible", action: "charge_limit", args: { battery: "BAT0", percent: 80 } },
    { id: "power:charge_limit:BAT0:60", label: "", admin: true, risk: "reversible", action: "charge_limit", args: { battery: "BAT0", percent: 60 } },
    { id: "power:charge_limit:BAT1:60", label: "", admin: true, risk: "reversible", action: "charge_limit", args: { battery: "BAT1", percent: 60 } },
    { id: "power:resolve_conflict:tlp", label: "", admin: true, risk: "reversible", action: "resolve_conflict", args: { keep: "tlp" } },
  ],
};

describe("power helpers", () => {
  it("counts down from the deadline", () => {
    expect(remainingSecs(1000, 900_000)).toBe(100);
    expect(remainingSecs(1000, 2_000_000)).toBe(0);
    expect(remainingSecs(null, 0)).toBe(0);
    expect(formatCountdown(245)).toBe("4:05");
    expect(formatCountdown(3729)).toBe("1:02:09");
    expect(formatCountdown(-3)).toBe("0:00");
  });

  it("measures the hold progress", () => {
    const h = { active: true, started: 0, until: 100, auto_holders: [], auto_enabled: false, backend: "x" };
    expect(holdProgress(h, 50_000)).toBe(0.5);
    expect(holdProgress(h, 500_000)).toBe(1);
    expect(holdProgress({ ...h, until: null }, 1)).toBe(0);
  });

  it("grades battery health", () => {
    expect(healthTone(92)).toBe("good");
    expect(healthTone(70)).toBe("warn");
    expect(healthTone(40)).toBe("bad");
    expect(healthTone(undefined)).toBe("default");
  });

  it("builds capacity bars and caps a new battery at design", () => {
    expect(capacityBars({ present: true, state: "full", source: "x", design_capacity: 5000, full_charge_capacity: 4000, capacity_unit: "mAh" })).toEqual({ design: 5000, full: 4000, unit: "mAh", lost: 20 });
    expect(capacityBars({ present: true, state: "full", source: "x", design_capacity: 5000, full_charge_capacity: 5100 })?.lost).toBe(0);
    expect(capacityBars({ present: true, state: "full", source: "x" })).toBeNull();
  });

  it("orders profiles like the desktop and hides them when it cannot switch", () => {
    expect(profileOptions(linux)).toEqual(["power-saver", "balanced", "performance"]);
    expect(profileOptions({ ...linux, profile: { ...linux.profile, can_switch: false } })).toEqual([]);
    expect(profileOptions(null)).toEqual([]);
  });

  it("picks actions and charge choices per battery", () => {
    expect(actionsOf(linux, "resolve_conflict").map((a) => a.id)).toEqual(["power:resolve_conflict:tlp"]);
    expect(chargeChoices(linux, "BAT0")).toEqual([
      { percent: 60, id: "power:charge_limit:BAT0:60" },
      { percent: 80, id: "power:charge_limit:BAT0:80" },
    ]);
  });

  it("collapses blockers per process", () => {
    const g = groupBlockers([
      { process: "caffeinate", kind: "k", reason: "caffeinate command-line tool", display: false },
      { process: "caffeinate", kind: "k", reason: "caffeinate command-line tool", display: false },
      { process: "firefox.exe", kind: "DISPLAY", reason: "Playing video", display: true },
    ]);
    expect(g).toEqual([
      { process: "caffeinate", count: 2, display: false, reasons: ["caffeinate command-line tool"] },
      { process: "firefox.exe", count: 1, display: true, reasons: ["Playing video"] },
    ]);
  });

  it("maps managers to i18n keys", () => {
    expect(managerKey("power-profiles-daemon")).toBe("ppd");
    expect(managerKey("tlp-pd")).toBe("tlp");
    expect(managerKey("auto-cpufreq")).toBe("auto_cpufreq");
    expect(managerKey(null)).toBe("none");
  });
});
