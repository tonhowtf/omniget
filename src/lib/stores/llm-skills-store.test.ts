import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

type SkillsStore = typeof import("./llm-skills-store.svelte");

let store: SkillsStore;

beforeAll(async () => {
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("$derived", <T>(value: T) => value);
  store = await import("./llm-skills-store.svelte");
});

afterEach(() => {
  store.resetSkillsStore();
  invoke.mockReset();
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe("source shapes", () => {
  it("reads a bare string source", () => {
    expect(store.sourceKind("git")).toBe("git");
    expect(store.sourceKind("Zip")).toBe("zip");
  });

  it("reads an externally tagged source", () => {
    expect(store.sourceKind({ git: "https://example.test/x" })).toBe("git");
    expect(store.sourceKind({ dir: "/tmp/x" })).toBe("dir");
  });

  it("reads an internally tagged source", () => {
    expect(store.sourceKind({ source: "catalog", id: "openrouter-models" })).toBe("catalog");
    expect(store.sourceKind({ kind: "zip", path: "/tmp/a.zip" })).toBe("zip");
  });

  it("returns null for an unknown shape and falls back to the local label", () => {
    expect(store.sourceKind({ whatever: 1 })).toBeNull();
    expect(store.sourceKind(null)).toBeNull();
    expect(store.sourceLabelKey({ whatever: 1 })).toBe("llm.skills.source_local");
    expect(store.sourceLabelKey({ git: "u" })).toBe("llm.skills.source_git");
  });

  it("pulls the url or path out of the source, nested included", () => {
    expect(store.sourceDetail({ source: "git", url: "https://a.test/r" })).toBe("https://a.test/r");
    expect(store.sourceDetail({ dir: "/home/x/skill" })).toBe("/home/x/skill");
    expect(store.sourceDetail({ git: { url: "https://b.test/r" } })).toBe("https://b.test/r");
    expect(store.sourceDetail("git")).toBe("");
    expect(store.sourceDetail(null)).toBe("");
  });
});

describe("pure helpers", () => {
  it("lists the agents that have a skill active", () => {
    const agents = [
      { name: "Ada", skills: ["a", "b"] },
      { name: "Bob", skills: [] },
      { name: "Cid" },
      { name: "Dee", skills: ["b"] },
    ];
    expect(store.agentsUsingSkill(agents, "b")).toEqual(["Ada", "Dee"]);
    expect(store.agentsUsingSkill(agents, "zzz")).toEqual([]);
  });

  it("matches a catalogue entry against the installed list by name", () => {
    const installed = [{ name: "openrouter-models", description: "" }];
    expect(store.isInstalled(installed, { name: "openrouter-models", description: "" })).toBe(true);
    expect(store.isInstalled(installed, { name: "openrouter-tts", description: "" })).toBe(false);
  });

  it("maps every ERR_SKILL_* code of core/skills/mod.rs to an i18n key", () => {
    expect(store.skillErrorKey("ERR_SKILL_GIT: git not found")).toBe("llm.skills.err_git");
    expect(store.skillErrorKey("ERR_SKILL_ZIP: traversal")).toBe("llm.skills.err_zip");
    expect(store.skillErrorKey("ERR_SKILL_PARSE")).toBe("llm.skills.err_parse");
    expect(store.skillErrorKey("ERR_SKILL_NAME")).toBe("llm.skills.err_name");
    expect(store.skillErrorKey("ERR_SKILL_DESCRIPTION")).toBe("llm.skills.err_description");
    expect(store.skillErrorKey("ERR_SKILL_NOT_FOUND")).toBe("llm.skills.err_not_found");
    expect(store.skillErrorKey("ERR_SKILL_TOO_BIG")).toBe("llm.skills.err_too_big");
    expect(store.skillErrorKey("ERR_SKILL_PATH")).toBe("llm.skills.err_path");
    expect(store.skillErrorKey("ERR_SKILL_IO: no such file")).toBe("llm.skills.err_io");
    expect(store.skillErrorKey("boom")).toBe("llm.skills.err_generic");
  });

  it("reads the real SkillSource shape of core/skills/mod.rs", () => {
    expect(store.sourceKind({ kind: "dir", from: "/home/x/s" })).toBe("dir");
    expect(store.sourceDetail({ kind: "dir", from: "/home/x/s" })).toBe("/home/x/s");
    expect(store.sourceKind({ kind: "zip", from: "/tmp/s.zip" })).toBe("zip");
    expect(
      store.sourceDetail({ kind: "git", url: "https://a.test/r", subdir: "skills/x" }),
    ).toBe("https://a.test/r");
    expect(store.sourceKind({ kind: "unknown" })).toBeNull();
    expect(store.sourceLabelKey({ kind: "unknown" })).toBe("llm.skills.source_local");
  });

  it("flags an entry the catalogue does not license (plan D-5)", () => {
    expect(store.isUnlicensed({ name: "a", description: "d" })).toBe(true);
    expect(store.isUnlicensed({ name: "a", description: "d", license: null })).toBe(true);
    expect(store.isUnlicensed({ name: "a", description: "d", license: "" })).toBe(true);
    expect(store.isUnlicensed({ name: "a", description: "d", license: "MIT" })).toBe(false);
  });

  it("asks for a second press before installing an unlicensed entry", () => {
    const entry = { name: "openrouter-tts", description: "d" };
    const first = store.catalogPress(entry, null);
    expect(first).toEqual({ confirming: "openrouter-tts", install: false });
    const second = store.catalogPress(entry, "openrouter-tts");
    expect(second).toEqual({ confirming: null, install: true });
    // Arming one card and pressing another only moves the warning.
    expect(store.catalogPress(entry, "other-skill")).toEqual({
      confirming: "openrouter-tts",
      install: false,
    });
  });

  it("installs a licensed entry on the first press", () => {
    const entry = { name: "x", description: "d", license: "MIT" };
    expect(store.catalogPress(entry, null)).toEqual({ confirming: null, install: true });
  });

});

describe("the security scan", () => {
  const clean = { status: "scanned", score: 12, severity: "LOW", recommendation: "SAFE" } as const;
  const risky = {
    status: "scanned",
    score: 85,
    severity: "CRITICAL",
    recommendation: "DO_NOT_INSTALL",
    findings: 3,
  } as const;

  it("ports SkillSpector's own threshold and keeps the comparison exclusive", () => {
    // skillspector/constants.py: RISK_THRESHOLD = 50, and cli.py exits on `> 50`.
    expect(store.SCAN_RISK_THRESHOLD).toBe(50);
    const at = (score: number) => ({ status: "scanned", score, recommendation: "CAUTION" }) as const;
    expect(store.isScanHighRisk(at(50))).toBe(false);
    expect(store.isScanHighRisk(at(51))).toBe(true);
  });

  it("reads a high score as high risk, not the other way round", () => {
    expect(store.scanScore(clean)).toBe(12);
    expect(store.scanScore(risky)).toBe(85);
    expect(store.isScanHighRisk(clean)).toBe(false);
    expect(store.isScanHighRisk(risky)).toBe(true);
  });

  it("honours DO_NOT_INSTALL even under the threshold", () => {
    expect(
      store.isScanHighRisk({ status: "scanned", score: 10, recommendation: "DO_NOT_INSTALL" }),
    ).toBe(true);
  });

  it("treats a missing or failed scan as unknown, never as risky and never as safe", () => {
    for (const scan of [
      null,
      undefined,
      { status: "not_scanned" } as const,
      { status: "failed", reason: "timed out" } as const,
    ]) {
      expect(store.isScanHighRisk(scan)).toBe(false);
      expect(store.scanScore(scan)).toBeNull();
    }
    expect(store.scanBadgeKey(null)).toBe("llm.skills.scan_none");
    expect(store.scanBadgeKey({ status: "not_scanned" })).toBe("llm.skills.scan_none");
    expect(store.scanBadgeKey({ status: "failed", reason: "x" })).toBe("llm.skills.scan_failed");
    expect(store.scanBadgeTone({ status: "failed", reason: "x" })).toBe("warn");
    expect(store.scanBadgeTone(null)).toBe("plain");
  });

  it("badges a clean scan without ever calling it safe", () => {
    expect(store.scanBadgeKey(clean)).toBe("llm.skills.scan_ok");
    expect(store.scanBadgeTone(clean)).toBe("plain");
    expect(store.scanBadgeKey(risky)).toBe("llm.skills.scan_risky");
    expect(store.scanBadgeTone(risky)).toBe("risk");
  });

  it("finds the scan of an installed skill by name", () => {
    const installed = [{ name: "a", description: "d", scan: risky }];
    expect(store.scanOf(installed, "a")).toBe(risky);
    expect(store.scanOf(installed, "b")).toBeNull();
    expect(store.scanOf(installed, undefined)).toBeNull();
  });

  it("asks for a second press on a risky skill, like the unlicensed catalogue does", () => {
    expect(store.scanPress("a", risky, null)).toEqual({ confirming: "a", proceed: false });
    expect(store.scanPress("a", risky, "a")).toEqual({ confirming: null, proceed: true });
    // Arming one row and pressing another only moves the warning.
    expect(store.scanPress("a", risky, "b")).toEqual({ confirming: "a", proceed: false });
  });

  it("does not ask twice for a skill nothing flagged", () => {
    expect(store.scanPress("a", clean, null)).toEqual({ confirming: null, proceed: true });
    expect(store.scanPress("a", null, null)).toEqual({ confirming: null, proceed: true });
    expect(store.scanPress("a", { status: "failed", reason: "x" }, null)).toEqual({
      confirming: null,
      proceed: true,
    });
  });

  it("maps the quarantine error to its own key", () => {
    expect(store.skillErrorKey("ERR_SKILL_PENDING: gone")).toBe("llm.skills.err_pending");
  });
});

describe("the quarantine flow", () => {
  const risky = {
    status: "scanned",
    score: 85,
    severity: "CRITICAL",
    recommendation: "DO_NOT_INSTALL",
  } as const;

  it("parks a risky install instead of listing it", async () => {
    invoke.mockResolvedValue({
      manifest: { name: "bad-skill", description: "d" },
      scan: risky,
      needs_confirm: ".pending-abc",
    });
    const ok = await store.installFromDir("/tmp/bad");
    // Not an install: the caller must not close its dialog on this.
    expect(ok).toBe(false);
    expect(store.getSkills()).toEqual([]);
    expect(store.getLastInstalled()).toBeNull();
    // And no error either: nothing went wrong, a decision is owed.
    expect(store.getSkillsErrorKey()).toBeNull();
    const pending = store.getPendingInstall();
    expect(pending?.token).toBe(".pending-abc");
    expect(pending?.manifest.name).toBe("bad-skill");
    expect(store.isScanHighRisk(pending?.scan)).toBe(true);
  });

  it("installs the parked copy on confirm, by token", async () => {
    invoke.mockResolvedValue({
      manifest: { name: "bad-skill", description: "d" },
      scan: risky,
      needs_confirm: ".pending-abc",
    });
    await store.installFromDir("/tmp/bad");
    invoke.mockResolvedValue({ name: "bad-skill", description: "d", scan: risky });
    const ok = await store.confirmPendingInstall();
    expect(ok).toBe(true);
    expect(invoke).toHaveBeenLastCalledWith("llm_skills_confirm_install", {
      token: ".pending-abc",
    });
    expect(store.getSkills().map((s) => s.name)).toEqual(["bad-skill"]);
    // The verdict stays on the card after the user accepted it.
    expect(store.scanScore(store.getSkills()[0].scan)).toBe(85);
    expect(store.getPendingInstall()).toBeNull();
  });

  it("drops the parked copy on discard and installs nothing", async () => {
    invoke.mockResolvedValue({
      manifest: { name: "bad-skill", description: "d" },
      needs_confirm: ".pending-abc",
    });
    await store.installFromDir("/tmp/bad");
    invoke.mockResolvedValue(null);
    const ok = await store.discardPendingInstall();
    expect(ok).toBe(true);
    expect(invoke).toHaveBeenLastCalledWith("llm_skills_discard_install", {
      token: ".pending-abc",
    });
    expect(store.getSkills()).toEqual([]);
    expect(store.getPendingInstall()).toBeNull();
  });

  it("keeps a clean outcome object working like a plain install", async () => {
    invoke.mockResolvedValue({
      manifest: { name: "good-skill", description: "d" },
      scan: { status: "scanned", score: 0, recommendation: "SAFE" },
    });
    expect(await store.installFromDir("/tmp/good")).toBe(true);
    expect(store.getSkills().map((s) => s.name)).toEqual(["good-skill"]);
    expect(store.scanScore(store.getSkills()[0].scan)).toBe(0);
    expect(store.getPendingInstall()).toBeNull();
  });

  it("still accepts a bare manifest, which is what a backend with no scan sends", async () => {
    invoke.mockResolvedValue({ name: "plain", description: "d" });
    expect(await store.installFromDir("/tmp/plain")).toBe(true);
    expect(store.getSkills().map((s) => s.name)).toEqual(["plain"]);
  });

  it("picks an undecided install back up from the backend", async () => {
    invoke.mockResolvedValue([
      { token: ".pending-xyz", manifest: { name: "left-over", description: "d", scan: risky } },
    ]);
    await store.loadPendingInstall();
    expect(store.getPendingInstall()?.token).toBe(".pending-xyz");
    expect(store.scanScore(store.getPendingInstall()?.scan)).toBe(85);
  });

  it("probes the scanner once and never throws when the command is missing", async () => {
    invoke.mockResolvedValue({ available: true, name: "skillspector", threshold: 50 });
    await store.loadScanner();
    await store.loadScanner();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(store.isScannerAvailable()).toBe(true);

    store.resetSkillsStore();
    invoke.mockRejectedValue("ERR_UNKNOWN_COMMAND");
    await store.loadScanner();
    expect(store.isScannerAvailable()).toBe(false);
  });
});

describe("loading", () => {
  it("shows what the backend returned", async () => {
    invoke.mockResolvedValue([{ name: "x", description: "d" }]);
    await store.loadSkills();
    expect(store.getSkills()).toEqual([{ name: "x", description: "d" }]);
    expect(store.isSkillsAvailable()).toBe(true);
  });

  it("reports a real error while keeping the backend marked as wired", async () => {
    invoke.mockRejectedValue("ERR_SKILL_PARSE: bad frontmatter");
    await store.loadSkills();
    expect(store.isSkillsAvailable()).toBe(true);
    expect(store.getSkillsErrorKey()).toBe("llm.skills.err_parse");
    expect(store.getSkills()).toEqual([]);
  });

  it("treats a null answer as unavailable and invents no skills", async () => {
    invoke.mockResolvedValue(null);
    await store.loadSkills();
    expect(store.isSkillsAvailable()).toBe(false);
    expect(store.getSkills()).toEqual([]);
  });

  it("loads once per session unless forced", async () => {
    invoke.mockResolvedValue([]);
    await store.loadSkills();
    await store.loadSkills();
    expect(invoke).toHaveBeenCalledTimes(1);
    await store.loadSkills(true);
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it("shares one in-flight request between concurrent callers", async () => {
    invoke.mockResolvedValue([]);
    await Promise.all([store.loadSkills(), store.loadSkills(), store.loadSkills()]);
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("shows an empty showcase and the error when the catalogue cannot be read", async () => {
    invoke.mockRejectedValue("ERR_SKILL_IO: offline");
    await store.loadCatalog();
    expect(store.getCatalog()).toEqual([]);
    expect(store.getSkillsErrorKey()).toBe("llm.skills.err_io");
  });

  it("keeps the backend showcase when there is one", async () => {
    invoke.mockResolvedValue([{ name: "a", description: "d", url: "u" }]);
    await store.loadCatalog();
    expect(store.getCatalog()).toHaveLength(1);
  });
});

describe("installing", () => {
  it("sends the folder path and appends the manifest", async () => {
    invoke.mockResolvedValue({ name: "release-notes", description: "d" });
    const ok = await store.installFromDir("/tmp/release-notes");
    expect(ok).toBe(true);
    expect(invoke).toHaveBeenCalledWith("llm_skills_install_dir", { path: "/tmp/release-notes" });
    expect(store.getSkills().map((s) => s.name)).toEqual(["release-notes"]);
    expect(store.getLastInstalled()).toBe("release-notes");
  });

  it("sends the zip path", async () => {
    invoke.mockResolvedValue({ name: "z", description: "d" });
    await store.installFromZip("/tmp/z.zip");
    expect(invoke).toHaveBeenCalledWith("llm_skills_install_zip", { path: "/tmp/z.zip" });
  });

  it("sends url, rev and subdir for a git install", async () => {
    invoke.mockResolvedValue({ name: "g", description: "d" });
    await store.installFromGit("https://a.test/r");
    expect(invoke).toHaveBeenCalledWith("llm_skills_install_git", {
      url: "https://a.test/r",
      rev: null,
      subdir: null,
    });
    await store.installFromGit("https://a.test/r", "abc", "skills/x");
    expect(invoke).toHaveBeenLastCalledWith("llm_skills_install_git", {
      url: "https://a.test/r",
      rev: "abc",
      subdir: "skills/x",
    });
  });

  it("installs a showcase entry by name, keeping the repo and the pin in Rust", async () => {
    invoke.mockResolvedValue({ name: "openrouter-tts", description: "d" });
    const entry = { name: "openrouter-tts", description: "d", repo_url: "https://github.com/OpenRouterTeam/skills" };
    await store.installCatalogEntry(entry);
    expect(invoke).toHaveBeenCalledWith("llm_skills_install_catalog", {
      name: "openrouter-tts",
    });
  });

  it("replaces an entry of the same name instead of duplicating it", async () => {
    invoke.mockResolvedValue([{ name: "dup", description: "old" }]);
    await store.loadSkills();
    invoke.mockResolvedValue({ name: "dup", description: "new" });
    await store.installFromDir("/tmp/dup");
    expect(store.getSkills()).toHaveLength(1);
    expect(store.getSkills()[0].description).toBe("new");
  });

  it("maps a failed install to an error key and changes no list", async () => {
    invoke.mockRejectedValue("ERR_SKILL_GIT");
    const ok = await store.installFromGit("https://a.test/r");
    expect(ok).toBe(false);
    expect(store.getSkills()).toEqual([]);
    expect(store.getSkillsErrorKey()).toBe("llm.skills.err_git");
  });

  it("treats a stub answer (null manifest) as unavailable", async () => {
    invoke.mockResolvedValue(null);
    const ok = await store.installFromDir("/tmp/x");
    expect(ok).toBe(false);
    expect(store.isSkillsAvailable()).toBe(false);
    expect(store.getSkillsErrorKey()).toBe("llm.skills.err_unavailable");
  });

  it("clears `busy` after an install, failed or not", async () => {
    invoke.mockRejectedValue("boom");
    await store.installFromDir("/tmp/x");
    expect(store.getBusySkill()).toBeNull();
  });

  it("refuses a showcase entry with no name without calling the backend", async () => {
    const ok = await store.installCatalogEntry({ name: "", description: "d" });
    expect(ok).toBe(false);
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe("removing", () => {
  it("drops the skill and calls the command", async () => {
    invoke.mockResolvedValue([{ name: "a", description: "" }, { name: "b", description: "" }]);
    await store.loadSkills();
    invoke.mockResolvedValue(null);
    const ok = await store.removeSkill("a");
    expect(ok).toBe(true);
    expect(store.getSkills().map((s) => s.name)).toEqual(["b"]);
    expect(invoke).toHaveBeenLastCalledWith("llm_skills_remove", { name: "a" });
  });

  it("puts the skill back when the backend refuses", async () => {
    invoke.mockResolvedValue([{ name: "a", description: "" }]);
    await store.loadSkills();
    invoke.mockRejectedValue("ERR_SKILL_NOT_FOUND");
    const ok = await store.removeSkill("a");
    expect(ok).toBe(false);
    expect(store.getSkills().map((s) => s.name)).toEqual(["a"]);
    expect(store.getSkillsErrorKey()).toBe("llm.skills.err_not_found");
  });
});

describe("store lifecycle", () => {
  it("clears the error and the success line on demand", async () => {
    invoke.mockResolvedValue({ name: "ok", description: "d" });
    await store.installFromDir("/tmp/ok");
    invoke.mockRejectedValue("boom");
    await store.installFromDir("/tmp/x");
    expect(store.getSkillsErrorKey()).not.toBeNull();
    expect(store.getLastInstalled()).toBe("ok");
    store.clearSkillsNotice();
    expect(store.getSkillsErrorKey()).toBeNull();
    expect(store.getLastInstalled()).toBeNull();
  });

  it("resets every field", async () => {
    invoke.mockResolvedValue([{ name: "a", description: "" }]);
    await store.loadSkills();
    store.resetSkillsStore();
    expect(store.getSkills()).toEqual([]);
    expect(store.getCatalog()).toEqual([]);
    expect(store.getBusySkill()).toBeNull();
    expect(store.getLastInstalled()).toBeNull();
    expect(store.isSkillsLoading()).toBe(false);
  });
});
