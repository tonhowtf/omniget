import { describe, expect, it } from "vitest";
import { canToggle, disableIds, filterItems, groupItems, groupOf, hueOf, initial, isSoftReason, noteCodes, reasonKey, type StartupItem } from "./startup";

function item(p: Partial<StartupItem>): StartupItem {
  return {
    id: p.id ?? `startup:x:${p.name}`,
    name: p.name ?? "x",
    kind: p.kind ?? "agent",
    scope: "user",
    location: p.location ?? "/L",
    command: p.command ?? "",
    enabled: p.enabled ?? true,
    can_disable: p.can_disable ?? true,
    can_enable: p.can_enable ?? true,
    needs_admin: false,
    third_party: p.third_party ?? true,
    ...p,
  } as StartupItem;
}

describe("startup", () => {
  it("groups kinds the way the screen shows them", () => {
    expect(groupOf("run")).toBe("login");
    expect(groupOf("startup_folder")).toBe("login");
    expect(groupOf("daemon")).toBe("agents");
    expect(groupOf("user_service")).toBe("services");
    expect(groupOf("task")).toBe("tasks");
    expect(groupOf("autostart")).toBe("autostart");
  });

  it("orders groups and puts third parties first", () => {
    const g = groupItems([
      item({ name: "b", kind: "task" }),
      item({ name: "sys", kind: "agent", third_party: false }),
      item({ name: "z", kind: "agent" }),
      item({ name: "a", kind: "login_item", enabled: false }),
    ]);
    expect(g.map((x) => x.group)).toEqual(["login", "agents", "tasks"]);
    expect(g[1].items.map((i) => i.name)).toEqual(["z", "sys"]);
    expect(g[0].on).toBe(0);
  });

  it("filters third parties and text", () => {
    const list = [item({ name: "Steam" }), item({ name: "Finder", third_party: false }), item({ name: "x", signer: "Google LLC" })];
    expect(filterItems(list, { thirdPartyOnly: true }).map((i) => i.name)).toEqual(["Steam", "x"]);
    expect(filterItems(list, { thirdPartyOnly: false, query: "google" }).map((i) => i.name)).toEqual(["x"]);
  });

  it("knows which way a toggle can go", () => {
    expect(canToggle(item({ enabled: true, can_disable: false }))).toBe(false);
    expect(canToggle(item({ enabled: false, can_enable: true }))).toBe(true);
    const list = [item({ id: "a" }), item({ id: "b", enabled: false }), item({ id: "c", can_disable: false })];
    expect(disableIds(list, new Set(["a", "b", "c"]))).toEqual(["a"]);
  });

  it("maps reasons and notes", () => {
    expect(reasonKey("settings_only")).toBe("washmybelly.startup.reason.settings_only");
    expect(reasonKey("whatever")).toBeNull();
    expect(isSoftReason("daemon_until_restart")).toBe(true);
    expect(isSoftReason("system_item")).toBe(false);
    expect(noteCodes(["btm_unavailable: needs admin", "btm_unavailable: x"])).toEqual(["btm_unavailable"]);
  });

  it("draws a letter tile", () => {
    expect(initial("  steam")).toBe("S");
    expect(initial("—")).toBe("?");
    expect(hueOf("Steam")).toBe(hueOf("Steam"));
  });
});
