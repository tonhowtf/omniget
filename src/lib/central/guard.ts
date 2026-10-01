import { invoke } from "@tauri-apps/api/core";

export type Severity = "info" | "low" | "medium" | "high" | "critical";
export type GuardLevel = "ok" | "warn" | "block";
type GuardValidator =
  | "structural"
  | "integrity"
  | "semantic"
  | "reference"
  | "provenance"
  | "command"
  | "config"
  | "skillspector";

type GuardKind =
  | "agent"
  | "command"
  | "skill"
  | "mcp"
  | "hook"
  | "setting"
  | "statusline"
  | "loop"
  | "workflow"
  | "mod"
  | "plugin"
  | "rule"
  | "template"
  | "sandbox"
  | "script"
  | "other";

interface GuardFinding {
  code: string;
  validator: GuardValidator;
  severity: Severity;
  file: string;
  line?: number;
  column?: number;
  json_path?: string;
  snippet?: string;
  detail: string;
  downgraded: boolean;
}

interface GuardCommand {
  file: string;
  origin: string;
  json_path?: string;
  line?: number;
  context?: string;
  command: string;
  programs: string[];
  codes: string[];
  worst?: Severity;
  network: boolean;
  auto_runs: boolean;
}

interface GuardValidatorSummary {
  validator: GuardValidator;
  score: number;
  errors: number;
  warnings: number;
  infos: number;
}

interface GuardFileDigest {
  path: string;
  sha256: string;
  size: number;
}

export interface GuardReport {
  label: string;
  root?: string;
  kind: GuardKind | string;
  origin_tool?: string;
  detected_tool: string;
  foreign_format?: string;
  files: GuardFileDigest[];
  findings: GuardFinding[];
  commands: GuardCommand[];
  validators: GuardValidatorSummary[];
  score: number;
  level: GuardLevel;
  counts: Partial<Record<Severity, number>>;
  skillspector?: { status: string; score?: number; recommendation?: string; reason?: string } | null;
  block_below: number;
  warn_below: number;
  scanned_at: string;
}

interface GuardProvenance {
  repo?: string | null;
  url?: string | null;
  commit?: string | null;
  author?: string | null;
  license?: string | null;
  version?: string | null;
}

interface GuardOptions {
  block_below?: number;
  warn_below?: number;
  strict?: boolean;
  skillspector?: boolean;
  expected_sha256?: Record<string, string> | null;
  provenance?: GuardProvenance | null;
  project_dir?: string | null;
}

export interface GuardRule {
  code: string;
  validator: GuardValidator;
  severity: Severity;
  en: string;
  pt: string;
}

interface DocStat {
  path: string;
  kind: string;
  name: string;
  title?: string;
  bytes: number;
  lines: number;
  words: number;
  sections: number;
  code_blocks: number;
  tokens: number;
  modified?: string;
}

interface HookStat {
  file: string;
  event: string;
  matcher?: string;
  handler: string;
  command: string;
  program?: string;
  program_found?: boolean;
  script?: string;
  script_found?: boolean;
  enabled: boolean;
  timeout?: number;
  fake_env: string[];
  tokens: number;
}

interface McpStat {
  file: string;
  name: string;
  description?: string;
  transport: "stdio" | "http" | "sse" | string;
  command?: string;
  args: number;
  env: number;
  enabled: boolean;
  category: string;
  complexity: number;
  config_bytes: number;
}

export interface ConfigStats {
  docs: DocStat[];
  hooks: HookStat[];
  hooks_by_event: Record<string, { total: number; enabled: number }>;
  mcps: McpStat[];
  mcp_by_category: Record<string, number>;
  totals: {
    docs: number;
    doc_tokens: number;
    doc_bytes: number;
    hooks: number;
    hooks_enabled: number;
    hooks_missing_program: number;
    hooks_fake_env: number;
    mcps: number;
    mcps_enabled: number;
  };
  errors: { path: string; error: string }[];
}

export type StatsKind = "command" | "rule" | "agent" | "skill" | "memory" | "hook" | "setting" | "mcp";

export function guardScanComponent(args: {
  kind: GuardKind | string;
  files: Record<string, string>;
  originTool?: string | null;
  entry?: string | null;
  label?: string | null;
  options?: GuardOptions | null;
}): Promise<GuardReport> {
  return invoke<GuardReport>("guard_scan_component", {
    kind: args.kind,
    files: args.files,
    originTool: args.originTool ?? null,
    entry: args.entry ?? null,
    label: args.label ?? null,
    options: args.options ?? null,
  });
}

export function guardScanInstalled(args: {
  targetId: string;
  scope: string;
  projectDir?: string | null;
  paths: string[];
  options?: GuardOptions | null;
}): Promise<GuardReport[]> {
  return invoke<GuardReport[]>("guard_scan_installed", {
    targetId: args.targetId,
    scope: args.scope,
    projectDir: args.projectDir ?? null,
    paths: args.paths,
    options: args.options ?? null,
  });
}

export function guardConfigStats(
  pathsByKind: Partial<Record<StatsKind, string[]>>,
  projectDir: string | null = null,
): Promise<ConfigStats> {
  return invoke<ConfigStats>("guard_config_stats", { pathsByKind, projectDir });
}

let rulesCache: Promise<GuardRule[]> | null = null;

export function guardRules(): Promise<GuardRule[]> {
  if (!rulesCache) {
    rulesCache = invoke<GuardRule[]>("guard_rules").catch((e) => {
      rulesCache = null;
      throw e;
    });
  }
  return rulesCache;
}

const SEVERITY_ORDER: Severity[] = ["critical", "high", "medium", "low", "info"];

function severityRank(s: Severity): number {
  return 4 - SEVERITY_ORDER.indexOf(s);
}

export function ruleText(rule: GuardRule | undefined, locale: string | null | undefined, fallback = ""): string {
  if (!rule) return fallback;
  return (locale ?? "").toLowerCase().startsWith("pt") ? rule.pt : rule.en;
}

export function rulesByCode(rules: GuardRule[]): Map<string, GuardRule> {
  return new Map(rules.map((r) => [r.code, r]));
}

export function findingsByValidator(report: GuardReport): [GuardValidator, GuardFinding[]][] {
  const order: GuardValidator[] = [
    "command",
    "config",
    "semantic",
    "skillspector",
    "reference",
    "integrity",
    "structural",
    "provenance",
  ];
  const map = new Map<GuardValidator, GuardFinding[]>();
  for (const f of report.findings) {
    const list = map.get(f.validator) ?? [];
    list.push(f);
    map.set(f.validator, list);
  }
  return order
    .filter((v) => map.has(v))
    .map((v) => [v, (map.get(v) ?? []).sort((a, b) => severityRank(b.severity) - severityRank(a.severity))]);
}

export function visibleFindings(list: GuardFinding[], showInfo: boolean): GuardFinding[] {
  return showInfo ? list : list.filter((f) => f.severity !== "info");
}

export function summarize(reports: GuardReport[]): Record<GuardLevel, number> {
  const out: Record<GuardLevel, number> = { ok: 0, warn: 0, block: 0 };
  for (const r of reports) out[r.level] += 1;
  return out;
}

export function formatTokens(n: number): string {
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1_000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}
