import { describe, expect, it } from "vitest";
import { factTags, filterItems, groupItems, needsLook, noteCodes, revealPath, stateKey, toneOf, type PersistenceItem } from "./security";

function item(p: Partial<PersistenceItem>): PersistenceItem {
  return {
    id: `security:${p.kind ?? "launch_agent"}:${p.name}`,
    kind: "launch_agent",
    scope: "user",
    name: "x",
    exists: true,
    summary: "",
    third_party: true,
    notes: [],
    xprotect_matches: [],
    ...p,
  } as PersistenceItem;
}

describe("security", () => {
  it("gives neutral tones, never a danger one", () => {
    expect(toneOf("notarized")).toBe("good");
    expect(toneOf("apple")).toBe("good");
    expect(toneOf("signed")).toBe("neutral");
    expect(toneOf("unsigned")).toBe("warn");
    expect(toneOf("missing")).toBe("warn");
    expect(toneOf(null)).toBe("muted");
  });

  it("filters third-party, look-only and by text", () => {
    const items = [
      item({ name: "Zoom", class: "notarized", signer: "Zoom Video" }),
      item({ name: "com.apple.x", third_party: false, class: "apple" }),
      item({ name: "gone", class: "missing" }),
      item({ name: "odd", class: "signed", xprotect_matches: ["RULE"] }),
    ];
    expect(filterItems(items, { thirdPartyOnly: true, query: "" })).toHaveLength(3);
    expect(filterItems(items, { thirdPartyOnly: false, query: "" })).toHaveLength(4);
    expect(filterItems(items, { thirdPartyOnly: true, query: "zoom video" }).map((i) => i.name)).toEqual(["Zoom"]);
    expect(filterItems(items, { thirdPartyOnly: true, query: "", lookOnly: true }).map((i) => i.name)).toEqual(["gone", "odd"]);
    expect(needsLook(items[0])).toBe(false);
  });

  it("groups in a fixed order with items worth a look first", () => {
    const g = groupItems([
      item({ name: "b", kind: "cron" }),
      item({ name: "a", kind: "launch_daemon" }),
      item({ name: "z", kind: "launch_agent", class: "notarized" }),
      item({ name: "y", kind: "launch_agent", class: "unsigned" }),
      item({ name: "q", kind: "weird" }),
    ]);
    expect(g.map((x) => x.kind)).toEqual(["launch_agent", "launch_daemon", "cron", "other"]);
    expect(g[0].items.map((i) => i.name)).toEqual(["y", "z"]);
    expect(g[0].look).toBe(1);
  });

  it("reads note codes, reveal paths, tags and states", () => {
    expect(noteCodes(["btm_unavailable: sfltool: x", "yara_unavailable: install", "other: y", "btm_unavailable: z"])).toEqual([
      "btm_unavailable",
      "yara_unavailable",
    ]);
    expect(revealPath(item({ checked: "/A/b", path: "/L/p.plist" }))).toBe("/A/b");
    expect(revealPath(item({ exists: false, checked: "/gone", path: "/L/p.plist" }))).toBe("/L/p.plist");
    expect(revealPath(item({ path: "com.acme.profile" }))).toBeNull();
    expect(factTags(item({ notes: ["hidden_folder", "quarantined", "x"] }))).toEqual(["quarantined", "hidden_folder"]);
    expect(stateKey("loaded")).toBe("washmybelly.security.state.loaded");
    expect(stateKey("activated enabled")).toBeNull();
  });
});
