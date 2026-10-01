import { invoke } from "@tauri-apps/api/core";

type SkillSource = string | Record<string, unknown>;

export interface SkillManifest {
  name: string;
  description: string;
  allowed_tools?: string[];
  path?: string;
  source?: SkillSource | null;
  license?: string | null;
  compatibility?: string | null;
  metadata?: Record<string, string>;
  body_bytes?: number;
  scan?: SkillScan | null;
}

export type SkillScan =
  | { status: "not_scanned" }
  | {
      status: "scanned";
      score: number;
      severity?: string;
      recommendation?: string;
      max_issue_severity?: string | null;
      findings?: number;
      issues?: SkillScanIssue[];
      scanner_version?: string | null;
      scanned_at?: string;
      llm?: boolean;
    }
  | { status: "failed"; reason: string };

interface SkillScanIssue {
  id?: string;
  severity?: string;
  category?: string | null;
  message?: string;
  file?: string;
  line?: number | null;
}

interface InstallOutcome {
  manifest: SkillManifest;
  scan?: SkillScan | null;
  needs_confirm?: string | null;
}

export interface PendingInstall {
  token: string;
  manifest: SkillManifest;
  scan: SkillScan;
}

export interface SkillCatalogEntry {
  name: string;
  description: string;
  subdir?: string;
  html_url?: string;
  repo_url?: string;
  commit?: string;
  skill_md_sha256?: string;
  files?: number;
  bytes?: number;
  license?: string | null;
}

type InstallKind = "dir" | "zip" | "git" | "catalog";

const KNOWN_SOURCES: InstallKind[] = ["dir", "zip", "git", "catalog"];

function sourceKind(source: SkillSource | null | undefined): InstallKind | null {
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

export function sourceLabelKey(source: SkillSource | null | undefined): string {
  return `llm.skills.source_${sourceKind(source) ?? "local"}`;
}

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

export function agentsUsingSkill(
  agents: { name: string; skills?: string[] }[],
  skill: string,
): string[] {
  return agents.filter((a) => (a.skills ?? []).includes(skill)).map((a) => a.name);
}

export function isUnlicensed(entry: SkillCatalogEntry): boolean {
  return !entry.license;
}

export function catalogPress(
  entry: SkillCatalogEntry,
  confirming: string | null,
): { confirming: string | null; install: boolean } {
  if (isUnlicensed(entry) && confirming !== entry.name) {
    return { confirming: entry.name, install: false };
  }
  return { confirming: null, install: true };
}

export const SCAN_RISK_THRESHOLD = 50;

export function scanScore(scan: SkillScan | null | undefined): number | null {
  if (!scan || scan.status !== "scanned") return null;
  return typeof scan.score === "number" ? scan.score : null;
}

function isScanHighRisk(scan: SkillScan | null | undefined): boolean {
  if (!scan || scan.status !== "scanned") return false;
  if ((scan.recommendation ?? "").toUpperCase() === "DO_NOT_INSTALL") return true;
  const score = scanScore(scan);
  return score !== null && score > SCAN_RISK_THRESHOLD;
}

export function scanBadgeKey(scan: SkillScan | null | undefined): string {
  if (!scan || scan.status === "not_scanned") return "llm.skills.scan_none";
  if (scan.status === "failed") return "llm.skills.scan_failed";
  return isScanHighRisk(scan) ? "llm.skills.scan_risky" : "llm.skills.scan_ok";
}

export function scanBadgeTone(scan: SkillScan | null | undefined): "risk" | "warn" | "plain" {
  if (!scan || scan.status === "not_scanned") return "plain";
  if (scan.status === "failed") return "warn";
  return isScanHighRisk(scan) ? "risk" : "plain";
}

export function scanOf(
  installed: SkillManifest[],
  name: string | undefined,
): SkillScan | null {
  if (!name) return null;
  return installed.find((s) => s.name === name)?.scan ?? null;
}

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

export function isInstalled(installed: SkillManifest[], entry: SkillCatalogEntry): boolean {
  return installed.some((s) => s.name === entry.name);
}

function skillErrorKey(err: unknown): string {
  const code = String(err ?? "");
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

let skills = $state<SkillManifest[]>([]);
let catalog = $state<SkillCatalogEntry[]>([]);
let loading = $state(false);
let available = $state(true);
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

export function isSkillsAvailable(): boolean {
  return available;
}

export function getBusySkill(): string | null {
  return busy;
}

export function getSkillsErrorKey(): string | null {
  return errorKey;
}

export function getLastInstalled(): string | null {
  return lastInstalled;
}

export function getPendingInstall(): PendingInstall | null {
  return pending;
}

export function isScannerAvailable(): boolean | null {
  return scannerAvailable;
}

export function clearSkillsNotice(): void {
  errorKey = null;
  lastInstalled = null;
}

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
        return;
      }
      available = false;
    })
    .catch((err) => {
      errorKey = skillErrorKey(err);
    })
    .finally(() => {
      loading = false;
      loadedOnce = true;
      inFlight = null;
    });
  return inFlight;
}

export async function loadCatalog(force = false): Promise<void> {
  if (catalogLoadedOnce && !force) return;
  catalogLoadedOnce = true;
  try {
    const list = await invoke<SkillCatalogEntry[] | null>("llm_skills_catalog");
    catalog = Array.isArray(list) ? list : [];
  } catch (err) {
    catalog = [];
    errorKey = skillErrorKey(err);
  }
}

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
  }
}

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
      if (outcome.needs_confirm) {
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
    return false;
  } finally {
    busy = null;
  }
}

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
      return true;
    }
    errorKey = "llm.skills.err_unavailable";
    available = false;
    return false;
  } catch (err) {
    errorKey = skillErrorKey(err);
    return false;
  } finally {
    busy = null;
  }
}

export async function discardPendingInstall(): Promise<boolean> {
  const current = pending;
  if (!current) return false;
  pending = null;
  try {
    await invoke("llm_skills_discard_install", { token: current.token });
    return true;
  } catch (err) {
    errorKey = skillErrorKey(err);
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

export function installFromRepo(spec: string): Promise<boolean> {
  return install("llm_skills_install_repo", { spec }, spec);
}

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
    return false;
  } finally {
    busy = null;
  }
}

interface SkillDependency { raw: string; kind: string; server?: string; tool?: string; name?: string }

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

