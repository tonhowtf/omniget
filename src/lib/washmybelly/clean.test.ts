import { describe, expect, it } from "vitest";
import {
  addRoots,
  artifactPath,
  artifactRows,
  categorySlices,
  cloneIdsFor,
  defaultDupSelection,
  defaultJunkSelection,
  dupPlanIds,
  dupSelectedBytes,
  dupSelectedCount,
  emptyRows,
  initialKeeps,
  junkRows,
  projectOf,
  pruneSelection,
  ruleLabelKey,
  runningReason,
  sameSet,
  selectedByCategory,
  selectedBytes,
  setKeep,
  toggleCopy,
  trashId,
  cloneId,
  unclonableCount,
  type DupGroup,
  type JunkItem,
} from "./clean";

function junk(p: Partial<JunkItem> & { id: string }): JunkItem {
  return {
    rule_id: "macos.app_caches",
    label: "App caches",
    category: "apps",
    path: `/Users/me/Library/Caches/${p.id}`,
    bytes: 100,
    files: 1,
    risk: "destructive",
    admin: false,
    destination: "unlink",
    running: null,
    selected_by_default: true,
    ...p,
  };
}

describe("junk", () => {
  const items = [
    junk({ id: "a", bytes: 500 }),
    junk({ id: "b", category: "browser", bytes: 900, running: "firefox", selected_by_default: false }),
    junk({ id: "c", category: "privacy", bytes: 10, selected_by_default: false }),
    junk({ id: "d", category: "logs", bytes: 50 }),
    junk({ id: "e", category: "browser", bytes: 100, selected_by_default: true, running: "in use by Explorer" }),
  ];

  it("ticks the backend's default and never an open app", () => {
    expect([...defaultJunkSelection(items)].sort()).toEqual(["a", "d"]);
  });

  it("explains why an item is unticked", () => {
    expect(runningReason(null)).toBeNull();
    expect(runningReason("firefox")).toEqual({ kind: "close", app: "Firefox" });
    expect(runningReason("chrome.exe")).toEqual({ kind: "close", app: "Google Chrome" });
    expect(runningReason("slack")).toEqual({ kind: "close", app: "Slack" });
    expect(runningReason("in use by Windows Explorer, Code")).toEqual({ kind: "in_use", who: "Windows Explorer, Code" });
    expect(runningReason("~/.mozilla/firefox/*/lock")).toEqual({ kind: "lock" });
    expect(runningReason("%APPDATA%\\x\\lock")).toEqual({ kind: "lock" });
  });

  it("slices categories heaviest first with shares", () => {
    const s = categorySlices({ categories: [], items });
    expect(s.map((c) => c.id)).toEqual(["browser", "apps", "logs", "privacy"]);
    expect(s[0].bytes).toBe(1000);
    expect(s.reduce((n, c) => n + c.fraction, 0)).toBeCloseTo(1);
    const fromBackend = categorySlices({ categories: [{ id: "dev", bytes: 0, count: 2 }, { id: "logs", bytes: 5, count: 1 }], items: [] });
    expect(fromBackend.map((c) => c.id)).toEqual(["logs", "dev"]);
    expect(fromBackend[1].fraction).toBe(0);
  });

  it("sums the selection overall and per category", () => {
    const sel = new Set(["a", "d", "zzz"]);
    expect(selectedBytes(items, sel)).toBe(550);
    const m = selectedByCategory(items, sel);
    expect(m.get("apps")).toBe(500);
    expect(m.get("logs")).toBe(50);
    expect(m.has("browser")).toBe(false);
  });

  it("builds rows grouped by category, open apps disabled", () => {
    const rows = junkRows(items, (i) => i.label, (i) => (i.running ? "busy" : null));
    expect(rows[0].group).toBe("browser");
    expect(rows.find((r) => r.id === "b")?.disabled).toBe(true);
    expect(rows.find((r) => r.id === "a")?.disabled).toBe(false);
    expect(rows.find((r) => r.id === "b")?.hint).toBe("busy");
    // Items of one category stay together.
    const groups = rows.map((r) => r.group);
    expect(groups.filter((g, i) => i > 0 && g !== groups[i - 1]).length).toBe(new Set(groups).size - 1);
    // Non-path targets (docker:..., snap:...) have no path detail.
    const [np] = junkRows([junk({ id: "x", path: "docker:prune" })], (i) => i.label, () => null);
    expect(np.detail).toBeNull();
  });

  it("rule keys, set equality, pruning", () => {
    expect(ruleLabelKey("macos.xcode_derived_data")).toBe("washmybelly.clean.rule.macos_xcode_derived_data");
    expect(sameSet(new Set(["a", "b"]), new Set(["b", "a"]))).toBe(true);
    expect(sameSet(new Set(["a"]), new Set(["a", "b"]))).toBe(false);
    expect([...pruneSelection(new Set(["a", "b", "gone"]), [{ id: "a" }, { id: "b", disabled: true }])]).toEqual(["a"]);
  });
});

describe("artifacts", () => {
  const projects = [
    { path: "/code/app", kind: "Cargo", kinds: ["Cargo", "Node"], bytes: 300, age_days: 40 },
    { path: "/code/app/web", kind: "Node", kinds: ["Node"], bytes: 50, age_days: 40 },
    { path: "/code/py", kind: "Python", kinds: ["Python"], bytes: 900, age_days: 3 },
  ];
  const items = [
    { id: "dev_artifacts:cargo:/code/app/target", label: "app/target", group: "Cargo", bytes: 200, risk: "destructive" as const },
    { id: "dev_artifacts:node:/code/app/node_modules", label: "app/node_modules", group: "Node", bytes: 100, risk: "destructive" as const },
    { id: "dev_artifacts:node:/code/app/web/node_modules", label: "web/node_modules", group: "Node", bytes: 50, risk: "destructive" as const },
    { id: "dev_artifacts:python:/code/py/.venv", label: "py/.venv", group: "Python", bytes: 900, risk: "destructive" as const },
  ];

  it("reads the folder out of the id, Windows drive letters included", () => {
    expect(artifactPath("dev_artifacts:cargo:/a/target")).toBe("/a/target");
    expect(artifactPath("dev_artifacts:node:C:\\x\\node_modules")).toBe("C:\\x\\node_modules");
  });

  it("puts each folder in its deepest project", () => {
    expect(projectOf(items[2].id, projects)?.path).toBe("/code/app/web");
    expect(projectOf(items[0].id, projects)?.path).toBe("/code/app");
    expect(projectOf("dev_artifacts:x:/code/apple/target", projects)).toBeNull();
  });

  it("groups rows by project, heaviest project first", () => {
    const rows = artifactRows({ items, projects });
    expect(rows[0].group).toBe("/code/py");
    expect(rows.find((r) => r.id === items[0].id)).toMatchObject({ label: "target", hint: "Cargo", group: "/code/app" });
    expect(rows.map((r) => r.group)).toEqual(["/code/py", "/code/app", "/code/app", "/code/app/web"]);
  });
});

describe("duplicates", () => {
  const groups: DupGroup[] = [
    {
      hash: "h1",
      size: 1000,
      wasted: 2000,
      files: [
        { path: "/d/a", mtime: 1, keep: true },
        { path: "/d/Downloads/a", mtime: 2, keep: false },
        { path: "/d/b/a", mtime: 3, keep: false },
      ],
    },
    { hash: "h2", size: 10, wasted: 10, files: [{ path: "/x", mtime: 1, keep: false }, { path: "/y", mtime: 1, keep: true }] },
  ];

  it("keeps the scan's choice and selects every copy", () => {
    const keeps = initialKeeps(groups);
    expect(keeps.get("h1")).toBe("/d/a");
    expect(keeps.get("h2")).toBe("/y");
    const sel = defaultDupSelection(groups, keeps);
    expect([...sel].sort()).toEqual([trashId("/d/Downloads/a"), trashId("/d/b/a"), trashId("/x")].sort());
    expect(dupSelectedBytes(groups, sel)).toBe(2010);
    expect(dupSelectedCount(groups, sel)).toBe(3);
  });

  it("moving Keep recomputes that group only", () => {
    const keeps = initialKeeps(groups);
    let sel = defaultDupSelection(groups, keeps);
    sel = toggleCopy(sel, "/x"); // untick in the other group
    const r = setKeep(groups, keeps, sel, "h1", "/d/b/a");
    expect(r.keeps.get("h1")).toBe("/d/b/a");
    expect(r.selected.has(trashId("/d/b/a"))).toBe(false);
    expect(r.selected.has(trashId("/d/a"))).toBe(true);
    expect(r.selected.has(trashId("/d/Downloads/a"))).toBe(true);
    expect(r.selected.has(trashId("/x"))).toBe(false);
    // Unknown group or file: nothing changes.
    expect(setKeep(groups, keeps, sel, "nope", "/x").selected).toEqual(sel);
    expect(setKeep(groups, keeps, sel, "h1", "/elsewhere").keeps.get("h1")).toBe("/d/a");
  });

  it("plans the file the scan chose to keep when the person switches, and clones only groups that kept its choice", () => {
    const keeps = initialKeeps(groups);
    const r = setKeep(groups, keeps, defaultDupSelection(groups, keeps), "h1", "/d/b/a");
    const plan = dupPlanIds(groups, r.selected);
    expect(plan.heldBack).toBe(0);
    expect(plan.ids.sort()).toEqual([trashId("/d/Downloads/a"), trashId("/d/a"), trashId("/x")].sort());
    expect(cloneIdsFor(groups, r.keeps, r.selected)).toEqual([cloneId("/x")]);
    expect(unclonableCount(groups, r.keeps, r.selected)).toBe(2);
    const d = defaultDupSelection(groups, keeps);
    expect(cloneIdsFor(groups, keeps, d).length).toBe(3);
    expect(unclonableCount(groups, keeps, d)).toBe(0);
  });
});

describe("empty folders and roots", () => {
  it("groups empty folders by parent", () => {
    const rows = emptyRows([
      { id: "empty_folders:/r/d/e", label: "e", detail: "/r/d/e", group: "empty_folders", bytes: 0, risk: "reversible" },
      { id: "empty_folders:/r/a", label: "a", detail: "/r/a", group: "empty_folders", bytes: 0, risk: "reversible" },
    ]);
    expect(rows.map((r) => r.group)).toEqual(["/r", "/r/d"]);
    expect(rows[0].detail).toBe("/r/a");
  });

  it("adds roots without repeats", () => {
    expect(addRoots(["/a"], ["/b", "/a", "/c"])).toEqual(["/a", "/b", "/c"]);
    expect(addRoots(["/a"], "/b")).toEqual(["/a", "/b"]);
    expect(addRoots(["/a"], null)).toEqual(["/a"]);
  });
});
