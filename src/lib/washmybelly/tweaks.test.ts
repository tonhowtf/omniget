import { describe, expect, it } from "vitest";
import {
  appliedCount,
  canPick,
  canRevert,
  clearGroup,
  extremePicks,
  orderedCategories,
  pickedItemIds,
  pickedNeedRestart,
  prunePicks,
  reasonKey,
  rowsFor,
  sourceName,
  stateKey,
  togglePick,
  type TweakItem,
  type TweakList,
} from "./tweaks";

function tw(id: string, extra: Partial<TweakItem> = {}): TweakItem {
  return {
    id,
    item_id: `tweaks:${id}`,
    default_item_id: `tweaks:${id}:default`,
    label: id,
    description: "",
    category: "gaming",
    risk: "reversible",
    admin: false,
    needs_restart: false,
    source: "win11debloat",
    license: "MIT",
    state: "not_applied",
    available: true,
    has_default: true,
    ...extra,
  };
}

/// What `wmb_tweaks_list` returns on a gaming desktop (trimmed).
const LIST: TweakList = {
  build: 26100,
  edition: "Professional",
  categories: ["hardening", "gaming", "appearance"],
  plan_id: null,
  restore_point: false,
  groups: [
    {
      id: "ours.power_plan",
      label: "Power plan",
      description: "Undo always goes back to the plan you had before.",
      members: ["ours.power_balanced", "ours.power_high", "ours.power_ultimate"],
      default: "ours.power_balanced",
      active: "ours.power_balanced",
    },
  ],
  items: [
    tw("w11d.DisableDVR", { label: "Disable Xbox game/screen recording", admin: true }),
    tw("ours.power_balanced", { group: "ours.power_plan", state: "applied", has_default: false, default_item_id: null }),
    tw("ours.power_high", { group: "ours.power_plan", has_default: false, default_item_id: null }),
    tw("ours.power_ultimate", { group: "ours.power_plan", has_default: false, default_item_id: null }),
    tw("sophia.hags", { available: false, reason: "no_dedicated_gpu", needs_restart: true, admin: true }),
    tw("ours.hvci_off", { risk: "extreme", needs_restart: true, admin: true, warning: "This trades security for FPS." }),
    tw("winutil.game_mode", { state: "applied" }),
    tw("sophia.aero_shake_off", { category: "appearance", state: "partial" }),
    tw("ours.uac_max", { category: "hardening", state: "unknown", available: false, reason: "unreadable" }),
  ],
};

describe("tweaks", () => {
  it("orders the categories the list sends", () => {
    expect(orderedCategories(LIST)).toEqual(["gaming", "appearance", "hardening"]);
    expect(orderedCategories(null)).toEqual([]);
  });

  it("shows each exclusive group once, as one row", () => {
    const rows = rowsFor(LIST, "gaming");
    expect(rows.map((r) => (r.kind === "group" ? r.group.id : r.item.id))).toEqual([
      "w11d.DisableDVR",
      "ours.power_plan",
      "sophia.hags",
      "ours.hvci_off",
      "winutil.game_mode",
    ]);
    const g = rows[1];
    expect(g.kind === "group" && g.items.length).toBe(3);
    expect(rowsFor(LIST, null).length).toBe(7);
    expect(rowsFor(LIST, "gaming", "recording").length).toBe(1);
    expect(rowsFor(LIST, "gaming", "power").length).toBe(1);
  });

  it("picks only what can change, one per group", () => {
    const [hags, high, ultimate, mode] = ["sophia.hags", "ours.power_high", "ours.power_ultimate", "winutil.game_mode"].map(
      (id) => LIST.items.find((i) => i.id === id)!,
    );
    expect(canPick(hags)).toBe(false);
    expect(canPick(mode)).toBe(false);
    let sel = togglePick(LIST, new Set(), hags);
    expect(sel.size).toBe(0);
    sel = togglePick(LIST, sel, high);
    sel = togglePick(LIST, sel, ultimate);
    expect([...sel]).toEqual(["ours.power_ultimate"]);
    sel = togglePick(LIST, sel, ultimate);
    expect(sel.size).toBe(0);
    sel = togglePick(LIST, new Set(["w11d.DisableDVR"]), high);
    expect([...clearGroup(LIST, sel, "ours.power_plan")]).toEqual(["w11d.DisableDVR"]);
  });

  it("turns picks into plan items and flags the extreme one", () => {
    const sel = new Set(["w11d.DisableDVR", "ours.hvci_off", "sophia.hags"]);
    expect(pickedItemIds(LIST, sel)).toEqual(["tweaks:w11d.DisableDVR", "tweaks:ours.hvci_off"]);
    expect(extremePicks(LIST, sel).map((i) => i.id)).toEqual(["ours.hvci_off"]);
    expect(pickedNeedRestart(LIST, sel)).toBe(true);
    expect(pickedNeedRestart(LIST, new Set(["w11d.DisableDVR"]))).toBe(false);
    expect([...prunePicks(LIST, sel)]).toEqual(["w11d.DisableDVR", "ours.hvci_off"]);
  });

  it("counts, reasons, states and sources", () => {
    expect(appliedCount(LIST)).toBe(2);
    expect(appliedCount(LIST, "appearance")).toBe(0);
    expect(canRevert(LIST.items.find((i) => i.id === "sophia.aero_shake_off")!)).toBe(true);
    expect(canRevert(LIST.items.find((i) => i.id === "w11d.DisableDVR")!)).toBe(false);
    expect(reasonKey("no_dedicated_gpu")).toBe("washmybelly.tweaks.reason.no_dedicated_gpu");
    expect(reasonKey("something new")).toBe("washmybelly.tweaks.reason.unknown");
    expect(reasonKey(null)).toBeNull();
    expect(stateKey("partial")).toBe("washmybelly.tweaks.state.partial");
    expect(stateKey("??")).toBe("washmybelly.tweaks.state.unknown");
    expect(sourceName("win11debloat")).toBe("Win11Debloat");
  });
});
