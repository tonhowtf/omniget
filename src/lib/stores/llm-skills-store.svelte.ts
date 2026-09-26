/**
 * State of `/llm/skills`: the installed Agent Skills, the OpenRouterTeam
 * showcase and the three install sources (folder, zip, git).
 *
 * Three rules shape this file:
 *
 * 1. **Nothing runs at rest.** No timer, no listener, no polling: every read is
 *    one `invoke` fired by a mount or by a click, and the result is cached for
 *    the session unless the caller passes `force`.
 * 2. **Nothing reaches the network without a click.** `installFromGit` is the
 *    only path that clones, and it only runs from the install dialog.
 * 3. **`ERR_STUB` is not a failure.** While `core::skills` is a stub every
 *    command answers `ERR_STUB` (and the screenshot harness answers `null`):
 *    the store then shows `DEMO_SKILLS` / `DEMO_CATALOG` so the page is
 *    reviewable before the backend exists, with the demo banner on.
 *
 * The wire shapes mirror `SkillManifest` from `core/skills/manifest.rs`
 * (f3-skills-core). Because that module is being written in parallel, every
 * field this file reads is optional and every unknown shape degrades to a
 * label instead of throwing.
 */
import { invoke } from "@tauri-apps/api/core";

// ── Wire types ──────────────────────────────────────────────────────────

/**
 * `SkillSource` as serde may serialise it: a bare string (`"dir"`), an
 * externally tagged object (`{ git: "https://…" }`) or an internally tagged
 * one (`{ source: "git", url: "https://…" }`). All three are handled.
 */
export type SkillSource = string | Record<string, unknown>;

export interface SkillManifest {
  name: string;
  description: string;
  allowed_tools?: string[];
  path?: string;
  source?: SkillSource | null;
  license?: string | null;
  compatibility?: string | null;
  metadata?: Record<string, string>;
  /** Size of the Markdown body; `inject::open` cuts at 64 KiB. */
  body_bytes?: number;
  /** What the security scan said when this copy was installed. */
  scan?: SkillScan | null;
}

/**
 * `ScanStatus` from `core/skills/scan.rs`, internally tagged.
 *
 * The score is NVIDIA SkillSpector's `risk_assessment.score`: **0–100 where
 * high means risky**, with `> 50` (`RISK_THRESHOLD` in
 * `skillspector/constants.py`) the line the tool itself treats as unsafe. Never
 * read a low score as "this skill is safe": it is one static pass by one tool,
 * run with `--no-llm`.
 */
export type SkillScan =
  | { status: "not_scanned" }
  | {
      status: "scanned";
      score: number;
      /** `LOW | MEDIUM | HIGH | CRITICAL`. */
      severity?: string;
      /** `SAFE | CAUTION | DO_NOT_INSTALL`. */
      recommendation?: string;
      max_issue_severity?: string | null;
      findings?: number;
      issues?: SkillScanIssue[];
      scanner_version?: string | null;
      scanned_at?: string;
      llm?: boolean;
    }
  | { status: "failed"; reason: string };

export interface SkillScanIssue {
  id?: string;
  severity?: string;
  category?: string | null;
  message?: string;
  file?: string;
  line?: number | null;
}

/**
 * What an install command answers: `InstallOutcome` from
 * `core/skills/install.rs`. When `needs_confirm` carries a token the skill is
 * **not** installed — it sits in quarantine until the user confirms or
 * discards it.
 */
export interface InstallOutcome {
  manifest: SkillManifest;
  scan?: SkillScan | null;
  needs_confirm?: string | null;
}

/** An install parked on its scan, waiting for the user's second click. */
export interface PendingInstall {
  token: string;
  manifest: SkillManifest;
  scan: SkillScan;
}

/**
 * One entry of the OpenRouterTeam showcase (`core/skills/catalog.rs`, pinned
 * at a commit). `html_url` is added by `llm_skills_catalog`: on the Rust side
 * it is a method, so plain serde would drop it.
 */
export interface SkillCatalogEntry {
  name: string;
  description: string;
  /** Path of the skill inside the repository. */
  subdir?: string;
  /** Page of this skill at the pinned commit. */
  html_url?: string;
  repo_url?: string;
  commit?: string;
  skill_md_sha256?: string;
  files?: number;
  bytes?: number;
  /**
   * Licence the catalogue declares for this entry. `null`/absent today: the
   * OpenRouterTeam repository declares none (plan D-5), so the showcase warns
   * before installing.
   */
  license?: string | null;
}

export type InstallKind = "dir" | "zip" | "git" | "catalog";

// ── Pure helpers (tested) ───────────────────────────────────────────────

const KNOWN_SOURCES: InstallKind[] = ["dir", "zip", "git", "catalog"];

/** Normalises any of the three serde shapes to `dir | zip | git | catalog`. */
export function sourceKind(source: SkillSource | null | undefined): InstallKind | null {
  if (!source) return null;
  if (typeof source === "string") {
    const key = source.toLowerCase();
    return KNOWN_SOURCES.find((k) => key.includes(k)) ?? null;
  }
  const tagged = source["source"] ?? source["kind"] ?? source["type"];
  if (typeof tagged === "string") {
    const key = tagged.toLowerCase();
    return KNOWN_SOURCES.find((k) => key.includes(k)) ?? null;
  }
  for (const key of Object.keys(source)) {
    const match = KNOWN_SOURCES.find((k) => key.toLowerCase().includes(k));
    if (match) return match;
  }
  return null;
}

/** i18n key for the origin badge. Unknown shapes fall back to `local`. */
export function sourceLabelKey(source: SkillSource | null | undefined): string {
  return `llm.skills.source_${sourceKind(source) ?? "local"}`;
}

/** The url or path behind the origin, when the manifest carries one. */
export function sourceDetail(source: SkillSource | null | undefined): string {
  if (!source || typeof source === "string") return "";
  for (const key of ["url", "from", "path", "repo", "origin"]) {
    const value = source[key];
    if (typeof value === "string" && value) return value;
  }
  for (const value of Object.values(source)) {
    if (typeof value === "string" && value) return value;
    if (value && typeof value === "object") {
      const nested = sourceDetail(value as Record<string, unknown>);
      if (nested) return nested;
    }
  }
  return "";
}

/** Agents whose `skills` list names this skill. Pure: the roster comes in. */
export function agentsUsingSkill(
  agents: { name: string; skills?: string[] }[],
  skill: string,
): string[] {
  return agents.filter((a) => (a.skills ?? []).includes(skill)).map((a) => a.name);
}

/**
 * True when the catalogue declares no licence for this entry (plan D-5). The
 * showcase badges it and asks for a second click before installing.
 */
export function isUnlicensed(entry: SkillCatalogEntry): boolean {
  return !entry.license;
}

/**
 * One press of a showcase Install button, as a pure step machine so the rule
 * is testable without a DOM: an unlicensed entry needs a second press (the
 * first one only arms the warning), a licensed one installs straight away.
 */
export function catalogPress(
  entry: SkillCatalogEntry,
  confirming: string | null,
): { confirming: string | null; install: boolean } {
  if (isUnlicensed(entry) && confirming !== entry.name) {
    return { confirming: entry.name, install: false };
  }
  return { confirming: null, install: true };
}

// ── The security scan (tested) ──────────────────────────────────────────

/**
 * SkillSpector's own threshold, ported from `skillspector/constants.py`
 * (`RISK_THRESHOLD = 50`). The CLI exits non-zero when `score > 50`, so the
 * comparison here is exclusive too.
 */
export const SCAN_RISK_THRESHOLD = 50;

/** The score, or `null` when there is no successful scan to read one from. */
export function scanScore(scan: SkillScan | null | undefined): number | null {
  if (!scan || scan.status !== "scanned") return null;
  return typeof scan.score === "number" ? scan.score : null;
}

/**
 * True when a scan actually ran and came back over the threshold, or with
 * SkillSpector's `DO_NOT_INSTALL` recommendation. A missing scanner and a
 * failed scan are both false: neither is evidence of risk, and neither is
 * evidence of safety.
 */
export function isScanHighRisk(scan: SkillScan | null | undefined): boolean {
  if (!scan || scan.status !== "scanned") return false;
  if ((scan.recommendation ?? "").toUpperCase() === "DO_NOT_INSTALL") return true;
  const score = scanScore(scan);
  return score !== null && score > SCAN_RISK_THRESHOLD;
}

/**
 * Badge for one skill. Deliberately never says "safe": the low band is
 * "scanned", with the number next to it, and everything else says what we do
 * not know.
 */
export function scanBadgeKey(scan: SkillScan | null | undefined): string {
  if (!scan || scan.status === "not_scanned") return "llm.skills.scan_none";
  if (scan.status === "failed") return "llm.skills.scan_failed";
  return isScanHighRisk(scan) ? "llm.skills.scan_risky" : "llm.skills.scan_ok";
}

/** Class the badge uses, so the page does not re-derive the bands. */
export function scanBadgeTone(scan: SkillScan | null | undefined): "risk" | "warn" | "plain" {
  if (!scan || scan.status === "not_scanned") return "plain";
  if (scan.status === "failed") return "warn";
  return isScanHighRisk(scan) ? "risk" : "plain";
}

/** The scan of the installed skill with this name, when there is one. */
export function scanOf(
  installed: SkillManifest[],
  name: string | undefined,
): SkillScan | null {
  if (!name) return null;
  return installed.find((s) => s.name === name)?.scan ?? null;
}

/**
 * One press of an "install anyway" button on a skill whose scan came back
 * risky, as a pure step machine — the same shape as [`catalogPress`], for the
 * same reason: the rule is testable without a DOM.
 *
 * The first press only arms the warning; the second one goes through. A press
 * on a different skill moves the warning instead of confirming anything, so a
 * double-click on the wrong row cannot install what it did not warn about.
 */
export function scanPress(
  name: string,
  scan: SkillScan | null | undefined,
  confirming: string | null,
): { confirming: string | null; proceed: boolean } {
  if (isScanHighRisk(scan) && confirming !== name) {
    return { confirming: name, proceed: false };
  }
  return { confirming: null, proceed: true };
}

/** True when the showcase entry is already installed under the same name. */
export function isInstalled(installed: SkillManifest[], entry: SkillCatalogEntry): boolean {
  return installed.some((s) => s.name === entry.name);
}

export function isUnavailable(err: unknown): boolean {
  return String(err ?? "").includes("ERR_STUB");
}

/**
 * Maps a backend error to an i18n key the page can show. The codes are the
 * `ERR_SKILL_*` constants of `core/skills/mod.rs`; the command sends them as
 * `"CODE: message"`, so a substring match is enough.
 */
export function skillErrorKey(err: unknown): string {
  const code = String(err ?? "");
  if (code.includes("ERR_STUB")) return "llm.skills.err_unavailable";
  if (code.includes("ERR_SKILL_GIT")) return "llm.skills.err_git";
  if (code.includes("ERR_SKILL_ZIP")) return "llm.skills.err_zip";
  if (code.includes("ERR_SKILL_PARSE")) return "llm.skills.err_parse";
  if (code.includes("ERR_SKILL_NAME")) return "llm.skills.err_name";
  if (code.includes("ERR_SKILL_DESCRIPTION")) return "llm.skills.err_description";
  if (code.includes("ERR_SKILL_NOT_FOUND")) return "llm.skills.err_not_found";
  if (code.includes("ERR_SKILL_TOO_BIG")) return "llm.skills.err_too_big";
  if (code.includes("ERR_SKILL_PATH")) return "llm.skills.err_path";
  if (code.includes("ERR_SKILL_PENDING")) return "llm.skills.err_pending";
  if (code.includes("ERR_SKILL_IO")) return "llm.skills.err_io";
  if (code.includes("ERR_SKILL_CHANGED")) return "assist.bots.err.skill_changed";
  if (code.includes("ERR_SKILL_NOT_BOUND")) return "assist.bots.err.skill_not_bound";
  return "llm.skills.err_generic";
}

// ── Demo data (stub backend and screenshots only) ───────────────────────

/**
 * The showcase as published in `OpenRouterTeam/skills` (read on 2026-09-18:
 * 17 folders under `skills/`, one more than the 16 the plan counted).
 * Descriptions are the first sentence of each `SKILL.md` frontmatter.
 * Only used when the backend has no catalogue to answer with.
 */
export const DEMO_CATALOG: SkillCatalogEntry[] = [
  ["create-agent-tui", "Scaffolds a complete agent TUI in TypeScript using @openrouter/agent — like create-react-app for terminal agents."],
  ["create-headless-agent", "Scaffolds a headless agent in TypeScript using @openrouter/agent and Bun — for CLI tools, API servers, queue workers and pipelines."],
  ["install-ori-harness", "Installs Ori and runs an existing coding agent CLI through Ori on OpenRouter, with OAuth sign-in and model selection."],
  ["openrouter-agent-migration", "Migration guide from @openrouter/sdk to @openrouter/agent for callModel, tool() and stop conditions."],
  ["openrouter-analytics", "Answers natural-language questions about OpenRouter usage: spend, request volume, model breakdown, latency and tokens."],
  ["openrouter-analytics-query", "Builds and runs analytics queries against the OpenRouter API, with the full parameter reference."],
  ["openrouter-analytics-schema", "Discovers the OpenRouter analytics schema: metrics, dimensions, filter operators and granularities."],
  ["openrouter-benchmarks", "Queries the OpenRouter Benchmarks API for model rankings and benchmark-backed model selection."],
  ["openrouter-generations", "Retrieves metadata and stored content for one generation: cost, latency, tokens, routing, prompt and completion."],
  ["openrouter-images", "Generates images from text prompts and edits existing images through the OpenRouter Image API."],
  ["openrouter-models", "Queries OpenRouter for models, pricing, context length, capabilities, throughput and provider performance."],
  ["openrouter-oauth", "Implements “Sign in with OpenRouter” with OAuth PKCE — no client registration, no backend, no secrets."],
  ["openrouter-stt", "Transcribes speech to text through the OpenRouter speech-to-text API."],
  ["openrouter-tts", "Generates speech audio from text through the OpenRouter text-to-speech API."],
  ["openrouter-typescript-sdk", "Reference for integrating with OpenRouter through the TypeScript SDK and the agent packages."],
  ["openrouter-video", "Generates videos from text prompts and reference images through the async OpenRouter video API."],
  ["spawn-ori-eval", "Spawns Ori as a subprocess to run a throwaway model eval on a pinned harness and model, then relays the result."],
].map(([name, description]) => ({
  name,
  description,
  subdir: `skills/${name}`,
  repo_url: "https://github.com/OpenRouterTeam/skills",
  html_url: `https://github.com/OpenRouterTeam/skills/tree/main/skills/${name}`,
}));

/** Two installed skills so the list is reviewable with a stub backend. */
export const DEMO_SKILLS: SkillManifest[] = [
  {
    name: "openrouter-models",
    description:
      "Queries OpenRouter for models, pricing, context length, capabilities, throughput and provider performance.",
    allowed_tools: ["Bash", "Read", "WebFetch"],
    path: "~/.omniget/skills/openrouter-models",
    source: { git: "https://github.com/OpenRouterTeam/skills" },
  },
  {
    name: "release-notes",
    description: "Writes the release notes of a version from the commits since the last tag.",
    allowed_tools: ["Bash", "Read"],
    path: "~/.omniget/skills/release-notes",
    source: { dir: "~/projects/skills/release-notes" },
  },
];

// ── State ───────────────────────────────────────────────────────────────

let skills = $state<SkillManifest[]>([]);
let catalog = $state<SkillCatalogEntry[]>([]);
let loading = $state(false);
let available = $state(true);
let demo = $state(false);
let busy = $state<string | null>(null);
let errorKey = $state<string | null>(null);
let lastInstalled = $state<string | null>(null);
let pending = $state<PendingInstall | null>(null);
let scannerAvailable = $state<boolean | null>(null);

let loadedOnce = false;
let catalogLoadedOnce = false;
let scannerLoadedOnce = false;
let inFlight: Promise<void> | null = null;

export function getSkills(): SkillManifest[] {
  return skills;
}

export function getCatalog(): SkillCatalogEntry[] {
  return catalog;
}

export function isSkillsLoading(): boolean {
  return loading;
}

/** False once a command answered `ERR_STUB`. */
export function isSkillsAvailable(): boolean {
  return available;
}

/** True when the lists on screen are `DEMO_*`, not installed skills. */
export function isDemoSkills(): boolean {
  return demo;
}

/** Name of the skill an install or a removal is running for, or `null`. */
export function getBusySkill(): string | null {
  return busy;
}

export function getSkillsErrorKey(): string | null {
  return errorKey;
}

/** Name of the last skill installed in this session (for the success line). */
export function getLastInstalled(): string | null {
  return lastInstalled;
}

/**
 * The install parked on its scan, waiting for a decision. While this is set the
 * skill is **not** installed and is in no prompt.
 */
export function getPendingInstall(): PendingInstall | null {
  return pending;
}

/**
 * Whether a scanner is installed on this machine. `null` until the probe has
 * answered, so the page can tell "no scanner" from "not asked yet".
 */
export function isScannerAvailable(): boolean | null {
  return scannerAvailable;
}

/** Clears the error line and the "installed" line, e.g. on reopening the dialog. */
export function clearSkillsNotice(): void {
  errorKey = null;
  lastInstalled = null;
}

// ── Reads ───────────────────────────────────────────────────────────────

/** Loads the installed skills once per session unless `force`. Never throws. */
export function loadSkills(force = false): Promise<void> {
  if (inFlight) return inFlight;
  if (loadedOnce && !force) return Promise.resolve();
  loading = true;
  errorKey = null;
  inFlight = invoke<SkillManifest[] | null>("llm_skills_list")
    .then((list) => {
      if (Array.isArray(list)) {
        skills = list;
        available = true;
        demo = false;
        return;
      }
      // `null` is what the screenshot harness answers for an unknown command.
      skills = DEMO_SKILLS;
      demo = true;
      available = false;
    })
    .catch((err) => {
      if (isUnavailable(err)) { skills = DEMO_SKILLS; demo = true; }
      else if (demo) { skills = []; demo = false; }
      available = !isUnavailable(err);
      if (!isUnavailable(err)) errorKey = skillErrorKey(err);
    })
    .finally(() => {
      loading = false;
      loadedOnce = true;
      inFlight = null;
    });
  return inFlight;
}

/** Loads the showcase once per session unless `force`. Never throws. */
export async function loadCatalog(force = false): Promise<void> {
  if (catalogLoadedOnce && !force) return;
  catalogLoadedOnce = true;
  try {
    const list = await invoke<SkillCatalogEntry[] | null>("llm_skills_catalog");
    if (Array.isArray(list)) {
      catalog = list;
      return;
    }
    catalog = DEMO_CATALOG;
    demo = true;
  } catch (err) {
    if (isUnavailable(err)) { catalog = DEMO_CATALOG; demo = true; }
    else { catalog = []; errorKey = skillErrorKey(err); }
  }
}

/**
 * Probes the scanner once per session. Never throws: an old backend that does
 * not know the command simply leaves this `false`, which reads as "not
 * scanned" everywhere and claims nothing.
 */
export async function loadScanner(force = false): Promise<void> {
  if (scannerLoadedOnce && !force) return;
  scannerLoadedOnce = true;
  try {
    const probe = await invoke<{ available?: boolean } | null>("llm_skills_scanner");
    scannerAvailable = Boolean(probe?.available);
  } catch {
    scannerAvailable = false;
  }
}

/**
 * Picks up an install the user left undecided — the app can be closed with the
 * dialog open, and a parked folder nobody can name again is worse than a
 * dialog that comes back.
 */
export async function loadPendingInstall(): Promise<void> {
  try {
    const list = await invoke<{ token: string; manifest: SkillManifest }[] | null>(
      "llm_skills_pending",
    );
    const first = Array.isArray(list) ? list[0] : undefined;
    pending = first
      ? {
          token: first.token,
          manifest: first.manifest,
          scan: first.manifest?.scan ?? { status: "not_scanned" },
        }
      : null;
  } catch {
    // An old backend has no pending installs to report. Not an error.
  }
}

// ── Writes (always behind a click) ──────────────────────────────────────

/**
 * Reads what an install command answered. Two shapes are accepted: the
 * `InstallOutcome` object the scanning backend sends, and a bare manifest,
 * which is what the pre-scan backend sent and what the screenshot harness
 * still fakes.
 */
function readOutcome(value: unknown): InstallOutcome | null {
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  const nested = record["manifest"];
  if (nested && typeof nested === "object") {
    return {
      manifest: nested as SkillManifest,
      scan: (record["scan"] as SkillScan | undefined) ?? null,
      needs_confirm: (record["needs_confirm"] as string | null | undefined) ?? null,
    };
  }
  if (typeof record["name"] === "string") {
    return { manifest: value as SkillManifest, scan: null, needs_confirm: null };
  }
  return null;
}

async function install(
  command: string,
  args: Record<string, unknown>,
  label: string,
): Promise<boolean> {
  if (busy) return false;
  busy = label;
  errorKey = null;
  try {
    const outcome = readOutcome(await invoke<unknown>(command, args));
    if (outcome) {
      available = true;
      demo = false;
      if (outcome.needs_confirm) {
        // Parked, not installed: it does not join the list, and the page owes
        // the user a second click before it ever does.
        pending = {
          token: outcome.needs_confirm,
          manifest: outcome.manifest,
          scan: outcome.scan ?? outcome.manifest.scan ?? { status: "not_scanned" },
        };
        return false;
      }
      const manifest = { ...outcome.manifest, scan: outcome.scan ?? outcome.manifest.scan };
      skills = [...skills.filter((s) => s.name !== manifest.name), manifest];
      lastInstalled = manifest.name;
      return true;
    }
    errorKey = "llm.skills.err_unavailable";
    available = false;
    return false;
  } catch (err) {
    errorKey = skillErrorKey(err);
    available = !isUnavailable(err);
    return false;
  } finally {
    busy = null;
  }
}

/**
 * Installs the parked skill the user just read the scan of. Nothing is
 * re-fetched: the backend moves the copy that was scanned, so what lands is
 * what was shown.
 */
export async function confirmPendingInstall(): Promise<boolean> {
  const current = pending;
  if (!current || busy) return false;
  busy = current.manifest.name;
  errorKey = null;
  try {
    const manifest = await invoke<SkillManifest | null>("llm_skills_confirm_install", {
      token: current.token,
    });
    if (manifest && typeof manifest.name === "string") {
      skills = [...skills.filter((s) => s.name !== manifest.name), manifest];
      lastInstalled = manifest.name;
      pending = null;
      available = true;
      demo = false;
      return true;
    }
    errorKey = "llm.skills.err_unavailable";
    available = false;
    return false;
  } catch (err) {
    errorKey = skillErrorKey(err);
    available = !isUnavailable(err);
    return false;
  } finally {
    busy = null;
  }
}

/** Throws the parked install away. The dialog closes either way. */
export async function discardPendingInstall(): Promise<boolean> {
  const current = pending;
  if (!current) return false;
  pending = null;
  try {
    await invoke("llm_skills_discard_install", { token: current.token });
    return true;
  } catch (err) {
    // The folder may already be gone; say so but do not bring the dialog back.
    if (!isUnavailable(err)) errorKey = skillErrorKey(err);
    return false;
  }
}

export function installFromDir(path: string): Promise<boolean> {
  return install("llm_skills_install_dir", { path }, path);
}

export function installFromZip(path: string): Promise<boolean> {
  return install("llm_skills_install_zip", { path }, path);
}

export function installFromGit(
  url: string,
  rev?: string | null,
  subdir?: string | null,
): Promise<boolean> {
  return install(
    "llm_skills_install_git",
    { url, rev: rev ?? null, subdir: subdir ?? null },
    url,
  );
}

/** `owner/repo`, `owner/repo/path` or `owner/repo@skill`, as in `npx skills add`. */
export function installFromRepo(spec: string): Promise<boolean> {
  return install("llm_skills_install_repo", { spec }, spec);
}

/**
 * Installs one showcase entry by name. The repository URL and the pinned
 * commit live in `core/skills/catalog.rs`, never in the UI.
 */
export function installCatalogEntry(entry: SkillCatalogEntry): Promise<boolean> {
  if (!entry.name) return Promise.resolve(false);
  return install("llm_skills_install_catalog", { name: entry.name }, entry.name);
}

export async function removeSkill(name: string): Promise<boolean> {
  if (busy) return false;
  busy = name;
  errorKey = null;
  const before = skills;
  skills = skills.filter((s) => s.name !== name);
  try {
    await invoke("llm_skills_remove", { name });
    available = true;
    return true;
  } catch (err) {
    skills = before;
    errorKey = skillErrorKey(err);
    available = !isUnavailable(err);
    return false;
  } finally {
    busy = null;
  }
}

// ── Version state (hash on disk vs recorded at install) ────────────────

export interface SkillDependency { raw: string; kind: string; server?: string; tool?: string; name?: string }

/** `llm_skills_status`: what is on disk against what OmniGet recorded. */
export interface SkillVersionStatus {
  name: string;
  hash?: string | null;
  recorded_hash?: string | null;
  version?: string | null;
  state: "ok" | "drift" | "unrecorded" | "invalid";
  compatibility?: string | null;
  dependencies?: SkillDependency[];
  bots: string[];
  error?: string;
}

let versionStatus = $state<Record<string, SkillVersionStatus>>({});
let statusInFlight: Promise<void> | null = null;

export function getSkillStatus(name: string): SkillVersionStatus | null {
  return versionStatus[name] ?? null;
}

/** Loads the version state of every installed skill. Never throws. */
export function loadSkillStatus(): Promise<void> {
  if (statusInFlight) return statusInFlight;
  statusInFlight = invoke<SkillVersionStatus[] | null>("llm_skills_status")
    .then((list) => {
      if (!Array.isArray(list)) return;
      versionStatus = Object.fromEntries(list.map((s) => [s.name, s]));
    })
    .catch(() => {})
    .finally(() => { statusInFlight = null; });
  return statusInFlight;
}

/** Repair: accept the files on disk (`accept`) or install again from the recorded origin (`reinstall`). */
export async function repairSkill(name: string, how: "accept" | "reinstall"): Promise<boolean> {
  if (busy) return false;
  busy = name;
  errorKey = null;
  try {
    await invoke(how === "accept" ? "llm_skills_accept" : "llm_skills_reinstall", { name });
    await loadSkills(true);
    await loadSkillStatus();
    return true;
  } catch (err) {
    errorKey = skillErrorKey(err);
    return false;
  } finally {
    busy = null;
  }
}

/** Test seam: drops every bit of state. */
export function resetSkillsStore(): void {
  skills = [];
  catalog = [];
  loading = false;
  available = true;
  demo = false;
  busy = null;
  errorKey = null;
  lastInstalled = null;
  pending = null;
  scannerAvailable = null;
  loadedOnce = false;
  catalogLoadedOnce = false;
  scannerLoadedOnce = false;
  inFlight = null;
  versionStatus = {};
  statusInFlight = null;
}
