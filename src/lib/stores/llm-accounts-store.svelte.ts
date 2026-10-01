import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type CliKind = "claude" | "codex";
type QuotaSource = "reported" | "estimated";
type WindowKind = "five_hour" | "seven_day";
export type SandboxMode = "read-only" | "write" | "full";

const WINDOW_SPAN_MS: Record<WindowKind, number> = {
  five_hour: 5 * 3_600_000,
  seven_day: 7 * 86_400_000,
};

export interface UsageWindow {
  account_id: string;
  window: WindowKind;
  used: number | null;
  used_tokens: number;
  resets_at_ms: number | null;
  source: QuotaSource;
}

export interface AccountView {
  id: string;
  cli: CliKind;
  label: string;
  config_dir: string;
  disabled: boolean;
  sandbox: SandboxMode;
  sandbox_flag: string;
  windows: UsageWindow[];
  sessions: number;
  active: boolean;
  last_error: string | null;
}

export interface Rotation {
  enabled: boolean;
  threshold: number;
  cooldown_s: number;
}

export interface AccountsSnapshot {
  accounts: AccountView[];
  chain: string[];
  rotation: Rotation;
  quota_available: boolean;
}

export interface CliDetected {
  cli: CliKind;
  path: string;
  version: string | null;
}

export interface UsageRow {
  key: string;
  calls: number;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
}

interface ToolBucket {
  tool: string;
  calls: number;
}

interface SessionRow {
  id: string;
  account_id: string;
  project: string;
  model: string;
  started_ms: number;
  ended_ms: number | null;
  turns: number;
  cost_usd: number;
}

export interface CliUsageReport {
  by_day: UsageRow[];
  by_model: UsageRow[];
  by_tool_hour: number[][];
  top_tools: ToolBucket[];
  sessions: SessionRow[];
  windows: UsageWindow[];
  total_cost_usd: number;
  scanned_files: number;
  scan_ms: number;
}

export type LoadState = "idle" | "loading" | "ready" | "unavailable";

interface SwitchRow {
  at_ms: number;
  agent: string;
  from: string;
  to: string;
  why: string;
}

const MAX_SWITCHES = 20;

const EMPTY_SNAPSHOT: AccountsSnapshot = {
  accounts: [],
  chain: [],
  rotation: { enabled: false, threshold: 0.9, cooldown_s: 300 },
  quota_available: false,
};

const EMPTY_REPORT: CliUsageReport = {
  by_day: [],
  by_model: [],
  by_tool_hour: [],
  top_tools: [],
  sessions: [],
  windows: [],
  total_cost_usd: 0,
  scanned_files: 0,
  scan_ms: 0,
};

function clampFraction(v: unknown): number | null {
  if (typeof v !== "number" || !Number.isFinite(v)) return null;
  return v < 0 ? 0 : v > 1 ? 1 : v;
}

function normalizeWindow(raw: unknown, accountId = ""): UsageWindow {
  const w = (raw ?? {}) as Partial<UsageWindow>;
  return {
    account_id: w.account_id || accountId,
    window: w.window === "seven_day" ? "seven_day" : "five_hour",
    used: clampFraction(w.used),
    used_tokens: typeof w.used_tokens === "number" ? w.used_tokens : 0,
    resets_at_ms: w.resets_at_ms ?? null,
    source: w.source === "reported" ? "reported" : "estimated",
  };
}

function normalizeSandbox(raw: unknown): SandboxMode {
  return raw === "write" ? "write" : raw === "full" ? "full" : "read-only";
}

function normalizeSnapshot(raw: unknown): AccountsSnapshot {
  const r = (raw ?? {}) as Partial<AccountsSnapshot>;
  const accounts = (r.accounts ?? []).map((a) => ({
    ...a,
    cli: a.cli === "codex" ? ("codex" as const) : ("claude" as const),
    label: a.label || a.id,
    disabled: a.disabled === true,
    sandbox: normalizeSandbox(a.sandbox),
    sandbox_flag: typeof a.sandbox_flag === "string" ? a.sandbox_flag : "",
    sessions: a.sessions ?? 0,
    active: a.active === true,
    last_error: a.last_error ?? null,
    windows: (a.windows ?? []).map((w) => normalizeWindow(w, a.id)),
  })) as AccountView[];
  const ids = new Set(accounts.map((a) => a.id));
  const chain = (r.chain ?? []).filter((id) => ids.has(id));
  for (const a of accounts) if (!chain.includes(a.id)) chain.push(a.id);
  return {
    accounts,
    chain,
    rotation: {
      enabled: r.rotation?.enabled === true,
      threshold: r.rotation?.threshold ?? 0.9,
      cooldown_s: r.rotation?.cooldown_s ?? 300,
    },
    quota_available: r.quota_available === true,
  };
}

function normalizeReport(raw: unknown): CliUsageReport {
  const r = (raw ?? {}) as Partial<CliUsageReport>;
  const grid = (r.by_tool_hour ?? []).map((row) =>
    Array.from({ length: 24 }, (_, h) => (Number.isFinite(row?.[h]) ? row[h] : 0)),
  );
  return {
    by_day: r.by_day ?? [],
    by_model: r.by_model ?? [],
    by_tool_hour: grid,
    top_tools: r.top_tools ?? [],
    sessions: r.sessions ?? [],
    windows: (r.windows ?? []).map((w) => normalizeWindow(w)),
    total_cost_usd: r.total_cost_usd ?? 0,
    scanned_files: r.scanned_files ?? 0,
    scan_ms: r.scan_ms ?? 0,
  };
}

function mergeWindows(
  accounts: readonly AccountView[],
  windows: readonly UsageWindow[],
): AccountView[] {
  if (windows.length === 0) return accounts as AccountView[];
  return accounts.map((a) => {
    const mine = windows.filter((w) => w.account_id === a.id);
    if (mine.length === 0) return a;
    const byKind = new Map(mine.map((w) => [w.window, w]));
    for (const w of a.windows) if (!byKind.has(w.window)) byKind.set(w.window, w);
    const order: WindowKind[] = ["five_hour", "seven_day"];
    return {
      ...a,
      windows: order.map((k) => byKind.get(k)).filter((w): w is UsageWindow => !!w),
    };
  });
}

export function orderByChain(accounts: readonly AccountView[], chain: readonly string[]): AccountView[] {
  const byId = new Map(accounts.map((a) => [a.id, a]));
  const ordered = chain.map((id) => byId.get(id)).filter((a): a is AccountView => !!a);
  const rest = accounts.filter((a) => !chain.includes(a.id));
  return [...ordered, ...rest];
}

export function reorder(chain: readonly string[], id: string, to: number): string[] {
  const from = chain.indexOf(id);
  if (from < 0) return [...chain];
  const out = [...chain];
  out.splice(from, 1);
  const at = Math.max(0, Math.min(out.length, to));
  out.splice(at, 0, id);
  return out;
}

export function exhaustsAt(w: UsageWindow, nowMs: number): number | null {
  if (!w.resets_at_ms || w.used === null) return null;
  const span = WINDOW_SPAN_MS[w.window];
  const elapsed = span - (w.resets_at_ms - nowMs);
  if (!(elapsed > 60_000) || w.used <= 0 || w.used >= 1) return null;
  const rate = w.used / elapsed;
  const left = (1 - w.used) / rate;
  const at = nowMs + left;
  return at >= w.resets_at_ms ? null : Math.round(at);
}

export function humanDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "";
  const mins = Math.round(ms / 60_000);
  const h = Math.floor(mins / 60);
  return h > 0 ? `${h} h ${mins % 60} min` : `${mins} min`;
}

export function cellLevel(value: number, max: number): 0 | 1 | 2 | 3 | 4 {
  if (!(value > 0) || !(max > 0)) return 0;
  const r = value / max;
  if (r <= 0.25) return 1;
  if (r <= 0.5) return 2;
  if (r <= 0.75) return 3;
  return 4;
}

export function gridMax(grid: readonly (readonly number[])[]): number {
  let max = 0;
  for (const row of grid) for (const v of row) if (v > max) max = v;
  return max;
}

export function gridTotal(grid: readonly (readonly number[])[]): number {
  let total = 0;
  for (const row of grid) for (const v of row) total += v;
  return total;
}

function pushSwitch(list: readonly SwitchRow[], row: SwitchRow, max = MAX_SWITCHES): SwitchRow[] {
  return [row, ...list].slice(0, max);
}

export function accountPaletteItems(
  accounts: readonly AccountView[],
  labels: { group: string; switchTo: (label: string) => string; openTab: string },
  actions: { activate: (id: string) => void; open: () => void },
): Array<{ id: string; label: string; group: string; keywords: string; action: () => void }> {
  const items = [
    {
      id: "nav-llm-accounts",
      label: labels.openTab,
      group: labels.group,
      keywords: "llm accounts quota contas cota claude codex cli",
      action: actions.open,
    },
  ];
  for (const a of accounts) {
    if (a.disabled) continue;
    items.push({
      id: `llm-account-${a.id}`,
      label: labels.switchTo(a.label),
      group: labels.group,
      keywords: `${a.label} ${a.cli} switch trocar conta account`,
      action: () => actions.activate(a.id),
    });
  }
  return items;
}

function rng(seed: number): () => number {
  let s = seed >>> 0 || 1;
  return () => {
    s ^= s << 13;
    s >>>= 0;
    s ^= s >> 17;
    s ^= s << 5;
    s >>>= 0;
    return s / 0x1_0000_0000;
  };
}

export function fakeSnapshot(nowMs: number): AccountsSnapshot {
  const base = Math.floor(nowMs / 300_000) * 300_000;
  const dir = (id: string) =>
    `/Users/demo/Library/Application Support/wtf.tonho.omniget/llm/profiles/${id}`;
  const win = (
    account_id: string,
    window: WindowKind,
    used: number | null,
    used_tokens: number,
    resets_at_ms: number | null,
    source: QuotaSource,
  ): UsageWindow => ({ account_id, window, used, used_tokens, resets_at_ms, source });
  return {
    accounts: [
      {
        id: "claude-pessoal",
        cli: "claude",
        label: "Claude Max \u00b7 pessoal",
        config_dir: dir("claude-pessoal"),
        disabled: false,
        sandbox: "read-only",
        sandbox_flag: "--permission-mode plan",
        windows: [
          win("claude-pessoal", "five_hour", 0.62, 0, base + 4_200_000, "reported"),
          win("claude-pessoal", "seven_day", 0.41, 0, base + 3 * 86_400_000, "reported"),
        ],
        sessions: 2,
        active: true,
        last_error: null,
      },
      {
        id: "claude-trabalho",
        cli: "claude",
        label: "Claude Max \u00b7 trabalho",
        config_dir: dir("claude-trabalho"),
        disabled: false,
        sandbox: "write",
        sandbox_flag: "--permission-mode acceptEdits",
        windows: [
          win("claude-trabalho", "five_hour", 0.18, 412_000, base + 15_600_000, "estimated"),
          win("claude-trabalho", "seven_day", null, 3_180_000, base + 86_400_000, "estimated"),
        ],
        sessions: 0,
        active: false,
        last_error: null,
      },
      {
        id: "codex-pessoal",
        cli: "codex",
        label: "Codex \u00b7 pessoal",
        config_dir: dir("codex-pessoal"),
        disabled: true,
        sandbox: "read-only",
        sandbox_flag: "--sandbox read-only",
        windows: [win("codex-pessoal", "five_hour", 0.94, 0, base + 900_000, "reported")],
        sessions: 0,
        active: false,
        last_error: "ERR_CLI_RATE",
      },
    ],
    chain: ["claude-pessoal", "claude-trabalho", "codex-pessoal"],
    rotation: { enabled: false, threshold: 0.9, cooldown_s: 300 },
    quota_available: true,
  };
}

function fakeUsage(nowMs: number): CliUsageReport {
  const base = Math.floor(nowMs / 300_000) * 300_000;
  const rand = rng(31);
  const by_day: UsageRow[] = [];
  for (let i = 13; i >= 0; i--) {
    const d = new Date(base - i * 86_400_000);
    by_day.push({
      key: d.toISOString().slice(0, 10),
      calls: 6 + Math.round(rand() * 52),
      input_tokens: Math.round(rand() * 900_000),
      output_tokens: Math.round(rand() * 90_000),
      cost_usd: Math.round(rand() * 640) / 100,
    });
  }
  const grid: number[][] = [];
  for (let day = 0; day < 7; day++) {
    const row: number[] = [];
    for (let hour = 0; hour < 24; hour++) {
      const office = hour >= 9 && hour <= 19 ? 1 : hour >= 20 && hour <= 23 ? 0.35 : 0.05;
      const week = day < 5 ? 1 : 0.3;
      row.push(Math.round(rand() * 46 * office * week));
    }
    grid.push(row);
  }
  const snapshot = fakeSnapshot(nowMs);
  return {
    by_day,
    by_model: [
      { key: "claude-sonnet-4-6", calls: 214, input_tokens: 4_120_000, output_tokens: 318_000, cost_usd: 18.42 },
      { key: "claude-opus-4-6", calls: 38, input_tokens: 610_000, output_tokens: 92_400, cost_usd: 11.07 },
      { key: "claude-haiku-4-6", calls: 176, input_tokens: 2_940_000, output_tokens: 141_000, cost_usd: 1.36 },
    ],
    by_tool_hour: grid,
    top_tools: [
      { tool: "Read", calls: 1_284 },
      { tool: "Bash", calls: 911 },
      { tool: "Edit", calls: 702 },
      { tool: "Grep", calls: 488 },
      { tool: "Write", calls: 233 },
    ],
    sessions: [
      { id: "s-1", account_id: "claude-pessoal", project: "omniget", model: "claude-opus-4-6", started_ms: base - 5_400_000, ended_ms: null, turns: 42, cost_usd: 3.18 },
      { id: "s-2", account_id: "claude-trabalho", project: "omniget-plugin-courses", model: "claude-sonnet-4-6", started_ms: base - 86_400_000, ended_ms: base - 79_000_000, turns: 17, cost_usd: 1.02 },
    ],
    windows: snapshot.accounts.flatMap((a) => a.windows),
    total_cost_usd: 30.85,
    scanned_files: 318,
    scan_ms: 142,
  };
}

let snapshot = $state<AccountsSnapshot>(EMPTY_SNAPSHOT);
let report = $state<CliUsageReport>(EMPTY_REPORT);
let detected = $state<CliDetected[]>([]);
let accountsState = $state<LoadState>("idle");
let usageState = $state<LoadState>("idle");
let accountsError = $state("");
let usageError = $state("");
let demo = $state(false);
let busy = $state(false);
let switches = $state<SwitchRow[]>([]);
let unlistenSwitch: UnlistenFn | null = null;

export function getAccounts(): AccountsSnapshot {
  return snapshot;
}

export function getUsage(): CliUsageReport {
  return report;
}

export function getDetected(): CliDetected[] {
  return detected;
}

export function getAccountsState(): LoadState {
  return accountsState;
}

export function getUsageState(): LoadState {
  return usageState;
}

export function getAccountsError(): string {
  return accountsError;
}

export function getUsageError(): string {
  return usageError;
}

export function isBusy(): boolean {
  return busy;
}

export function getSwitches(): SwitchRow[] {
  return switches;
}

export async function startSwitchLog(): Promise<() => void> {
  if (demo) return () => {};
  try {
    unlistenSwitch = await listen<{ agent?: string; from?: string; to?: string; why?: string }>(
      "llm://rerouted",
      (ev) => {
        const p = ev.payload ?? {};
        switches = pushSwitch(switches, {
          at_ms: Date.now(),
          agent: p.agent ?? "",
          from: p.from ?? "",
          to: p.to ?? "",
          why: p.why ?? "",
        });
      },
    );
  } catch {
  }
  return () => stopSwitchLog();
}

function stopSwitchLog(): void {
  if (unlistenSwitch) {
    unlistenSwitch();
    unlistenSwitch = null;
  }
}

async function call(command: string, args?: Record<string, unknown>): Promise<void> {
  if (demo) return;
  busy = true;
  try {
    const raw = await invoke<AccountsSnapshot>(command, args);
    snapshot = normalizeSnapshot(raw);
    accountsState = "ready";
    accountsError = "";
  } catch (e) {
    accountsError = String(e);
    if (accountsState !== "ready") accountsState = "unavailable";
    throw e;
  } finally {
    busy = false;
  }
}

export async function loadAccounts(): Promise<void> {
  if (demo) return;
  accountsState = accountsState === "ready" ? "ready" : "loading";
  try {
    snapshot = normalizeSnapshot(await invoke<AccountsSnapshot>("llm_accounts_list"));
    accountsState = "ready";
    accountsError = "";
  } catch (e) {
    accountsError = String(e);
    if (accountsState !== "ready") {
      snapshot = EMPTY_SNAPSHOT;
      accountsState = "unavailable";
    }
  }
}

export async function loadUsage(days = 30): Promise<void> {
  if (demo) return;
  usageState = usageState === "ready" ? "ready" : "loading";
  try {
    report = normalizeReport(await invoke<CliUsageReport>("llm_cli_usage_report", { days }));
    usageState = "ready";
    usageError = "";
    if (report.windows.length > 0) {
      snapshot = {
        ...snapshot,
        accounts: mergeWindows(snapshot.accounts, report.windows),
        quota_available: true,
      };
    }
  } catch (e) {
    usageError = String(e);
    report = EMPTY_REPORT;
    usageState = "unavailable";
  }
}

export async function detectClis(): Promise<void> {
  if (demo) return;
  try {
    detected = (await invoke<CliDetected[]>("llm_accounts_detect")) ?? [];
  } catch {
    detected = [];
  }
}

export async function createAccount(cli: CliKind, label: string, share = true): Promise<void> {
  await call("llm_accounts_create", { cli, label, share });
}

export async function removeAccount(id: string, purge = false): Promise<void> {
  await call("llm_accounts_remove", { id, purge });
}

export async function setDisabled(id: string, disabled: boolean): Promise<void> {
  await call("llm_accounts_set_disabled", { id, disabled });
}

export async function setSandbox(id: string, sandbox: SandboxMode): Promise<void> {
  await call("llm_accounts_set_sandbox", { id, sandbox });
}

export async function activateAccount(id: string): Promise<void> {
  await call("llm_accounts_activate", { id });
}

export async function setChain(chain: string[]): Promise<void> {
  snapshot = { ...snapshot, chain };
  await call("llm_accounts_set_chain", { chain });
}

export async function setRotation(rotation: Rotation): Promise<void> {
  if (demo) return;
  try {
    const next = await invoke<Rotation>("llm_accounts_rotation", { rotation });
    snapshot = { ...snapshot, rotation: next ?? rotation };
  } catch (e) {
    accountsError = String(e);
  }
}

export async function loginAccount(id: string): Promise<void> {
  if (demo) return;
  await invoke("llm_accounts_login", { id });
}

export function startAccountsDemo(nowMs = Date.now()): void {
  demo = true;
  snapshot = fakeSnapshot(nowMs);
  report = fakeUsage(nowMs);
  detected = [
    { cli: "claude", path: "/opt/homebrew/bin/claude", version: "2.1.275" },
    { cli: "codex", path: "/opt/homebrew/bin/codex", version: "0.48.0" },
  ];
  accountsState = "ready";
  usageState = "ready";
  switches = [
    { at_ms: nowMs - 900_000, agent: "coordinator", from: "claude-pessoal", to: "claude-trabalho", why: "ERR_CLI_RATE" },
    { at_ms: nowMs - 5_400_000, agent: "researcher", from: "codex-pessoal", to: "claude-pessoal", why: "quota" },
  ];
}

