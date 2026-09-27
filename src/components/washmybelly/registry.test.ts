import { describe, expect, it } from "vitest";
import { availability, visibleModules, WMB_MODULES, type Caps } from "./registry";

const ids = (caps: Caps) => visibleModules(caps).map((m) => m.mod.id);

describe("washmybelly registry", () => {
  it("never shows a module of another system", () => {
    expect(ids({ os: "macos" })).not.toContain("tweaks");
    expect(ids({ os: "macos" })).toContain("security");
    expect(ids({ os: "windows" })).toContain("tweaks");
    expect(ids({ os: "windows" })).not.toContain("security");
    expect(ids({ os: "linux" })).not.toContain("tweaks");
    expect(ids({ os: "linux" })).not.toContain("security");
  });

  it("shows a module of this system with the reason when a requirement is missing", () => {
    const uninstall = WMB_MODULES.find((m) => m.id === "uninstall")!;
    const a = availability(uninstall, { os: "linux", linux: { flatpak_sandbox: true, package_managers: ["flatpak"] } });
    expect(a).toEqual({ show: true, available: false, reasonKey: "washmybelly.reason.flatpak_sandbox" });
  });

  it("lets the backend module list have the last word", () => {
    const caps: Caps = { os: "macos", modules: [{ id: "overview", available: true }, { id: "clean", available: false, reason_key: "washmybelly.reason.full_disk_access" }] };
    expect(ids(caps)).toEqual(["overview", "clean"]);
    const clean = visibleModules(caps).find((m) => m.mod.id === "clean")!;
    expect(clean.availability).toEqual({ show: true, available: false, reasonKey: "washmybelly.reason.full_disk_access" });
  });

  it("mirrors the backend module ids", () => {
    const backend = ["overview", "clean", "disk", "uninstall", "startup", "privacy", "tweaks", "packages", "power", "games", "perf", "security", "maintenance", "receipts"];
    expect(new Set(WMB_MODULES.map((m) => m.id))).toEqual(new Set(backend));
  });
});
