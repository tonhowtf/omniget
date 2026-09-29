/**
 * Accounts & quota store (`/llm/accounts`).
 *
 * Reads three things, all on demand — this file never creates a timer, so a
 * closed tab costs zero (briefing §5):
 *   - `llm_accounts_list()` — accounts, fallback chain and rotation settings;
 *   - `llm_accounts_detect()` — which CLIs exist on this machine (button);
 *   - `llm_cli_usage_report()` — the dashboard, read from the CLIs' own JSONL.
 *
 * Nothing here ever sees a credential: an account is an id, a label and the
 * path of an isolated config directory (plan §9.2, `estudos/74` §A.3). The
 * quota number carries its own provenance — `reported` came from an official
 * channel, `estimated` was summed from the JSONL — and the UI always prints
 * which one it is.
 *
 * While `llm_cli_usage_report` is unavailable the page renders an empty state.
 * `fakeSnapshot`/`fakeUsage` feed the tests and the screenshot harness
 * (`?demo=1`) and never fill a real screen.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---------------------------------------------------------------------------
// Contract (mirrors `src-tauri/src/commands/llm/accounts.rs`)
// ---------------------------------------------------------------------------

export type CliKind = "claude" | "codex";
export type QuotaSource = "reported" | "estimated";
export type WindowKind = "five_hour" | "seven_day";
/**
 * What a turn on this account may touch. `read-only` is the default for every
 * account and the only value the backend ever infers; anything it does not
 * recognise parses back to `read-only`, so a write is always a deliberate act.
 */
export type SandboxMode = "read-only" | "write" | "full";

/** Span of each window, in ms. Used for the "runs out at this pace" line. */
export const WINDOW_SPAN_MS: Record<WindowKind, number> = {
  five_hour: 5 * 3_600_000,
  seven_day: 7 * 86_400_000,
};

export interface UsageWindow {
  account_id: string;
  window: WindowKind;
  /**
   * 0..1 of the window already spent, or `null` when no ceiling is known for
   * the plan (`cli_usage` never invents one). The UI then prints the token
   * count and draws no bar.
   */
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
  /** The literal CLI flag behind `sandbox`, printed next to the label. */
  sandbox_flag: string;
  windows: UsageWindow[];
  sessions: number;
  /** Head of the chain: the account the router prefers right now. */
  active: boolean;
  last_error: string | null;
}

export interface Rotation {
  enabled: boolean;
  /** 0..1 of the worst window that triggers a switch. */
  threshold: number;
  cooldown_s: number;
}

export interface AccountsSnapshot {
  accounts: AccountView[];
  /** Account ids in router order. */
  chain: string[];
  rotation: Rotation;
  /** False while no quota source answered: the meters say so. */
  quota_available: boolean;
}

export interface CliDetected {
  cli: CliKind;
  path: string;
  version: string | null;
}

/** One bucket of the report: a day (`YYYY-MM-DD`) or a model name in `key`. */
export interface UsageRow {
  key: string;
  calls: number;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
}

export interface ToolBucket {
  tool: string;
  calls: number;
}

export interface SessionRow {
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
  /** 7 rows (Mon..Sun) × 24 columns (local hour): tool calls. */
  by_tool_hour: number[][];
  top_tools: ToolBucket[];
  sessions: SessionRow[];
  /** Every account's windows, including the `estimated` ones from the sweep. */
  windows: UsageWindow[];
  total_cost_usd: number;
  /** How the scan went, so the tab can show a number instead of a feeling. */
  scanned_files: number;
  scan_ms: number;
}

export type LoadState = "idle" | "loading" | "ready" | "unavailable";

/** One line of the switch timeline, from the `llm://rerouted` event (F2). */
export interface SwitchRow {
  at_ms: number;
  agent: string;
  from: string;
  to: string;
  why: string;
}

/** How many switches the timeline keeps in memory. */
export const MAX_SWITCHES = 20;

// ---------------------------------------------------------------------------
// Pure helpers (tested in llm-accounts-store.test.ts)
// ---------------------------------------------------------------------------

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

/** 0..1, or null when the backend has no ceiling to divide by. */
function clampFraction(v: unknown): number | null {
  if (typeof v !== "number" || !Number.isFinite(v)) return null;
  return v < 0 ? 0 : v > 1 ? 1 : v;
}

/** One window off the wire, with every field defended. */
export function normalizeWindow(raw: unknown, accountId = ""): UsageWindow {
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

/**
 * The mode of one account off the wire. Only the exact word `write` is a
 * write: a missing, misspelled or hostile value reads as `read-only`, the same
 * rule the backend applies (`SandboxMode::parse`). The UI must never draw an
 * account as more restricted than it is, and must never invent a write.
 */
export function normalizeSandbox(raw: unknown): SandboxMode {
  // `full` (2.7): the ceiling that lets a turn whose run mode is full_access run.
  return raw === "write" ? "write" : raw === "full" ? "full" : "read-only";
}

/** A missing field never crashes a meter or a chart. */
export function normalizeSnapshot(raw: unknown): AccountsSnapshot {
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

export function normalizeReport(raw: unknown): CliUsageReport {
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

/**
 * Hand the sweep's windows to the accounts they belong to.
 *
 * `llm_accounts_list` only sees what an official channel already cached
 * (`reported`); the `estimated` ones only exist after a scan. Merging here
 * keeps the cards honest without making the list command scan.
 */
export function mergeWindows(
  accounts: readonly AccountView[],
  windows: readonly UsageWindow[],
): AccountView[] {
  if (windows.length === 0) return accounts as AccountView[];
  return accounts.map((a) => {
    const mine = windows.filter((w) => w.account_id === a.id);
    if (mine.length === 0) return a;
    const byKind = new Map(mine.map((w) => [w.window, w]));
    // A window the account already carried wins only if nothing newer came in.
    for (const w of a.windows) if (!byKind.has(w.window)) byKind.set(w.window, w);
    const order: WindowKind[] = ["five_hour", "seven_day"];
    return {
      ...a,
      windows: order.map((k) => byKind.get(k)).filter((w): w is UsageWindow => !!w),
    };
  });
}

/** Accounts in chain order; anything the chain forgot keeps its own order. */
export function orderByChain(accounts: readonly AccountView[], chain: readonly string[]): AccountView[] {
  const byId = new Map(accounts.map((a) => [a.id, a]));
  const ordered = chain.map((id) => byId.get(id)).filter((a): a is AccountView => !!a);
  const rest = accounts.filter((a) => !chain.includes(a.id));
  return [...ordered, ...rest];
}

/** Move `id` to `to` in the chain (drag and drop). Out-of-range is clamped. */
export function reorder(chain: readonly string[], id: string, to: number): string[] {
  const from = chain.indexOf(id);
  if (from < 0) return [...chain];
  const out = [...chain];
  out.splice(from, 1);
  const at = Math.max(0, Math.min(out.length, to));
  out.splice(at, 0, id);
  return out;
}

/**
 * The window under most pressure: what the router and the card headline use.
 * A window with no known ceiling (`used: null`) has no pressure to compare, so
 * it only wins when nothing else is on the table.
 */
export function worstWindow(windows: readonly UsageWindow[]): UsageWindow | null {
  let worst: UsageWindow | null = null;
  for (const w of windows) {
    if (!worst) {
      worst = w;
      continue;
    }
    if (w.used === null) continue;
    if (worst.used === null || w.used > worst.used) worst = w;
  }
  return worst;
}

/**
 * Epoch ms when a window runs out at the pace observed so far, or null when
 * the pace cannot be read (no reset time, nothing spent yet, or already full).
 *
 * The elapsed part of the window is `span - (resetsAt - now)`; the rate is
 * `used / elapsed`; the remainder `1 - used` at that rate is what is left.
 */
export function exhaustsAt(w: UsageWindow, nowMs: number): number | null {
  if (!w.resets_at_ms || w.used === null) return null;
  const span = WINDOW_SPAN_MS[w.window];
  const elapsed = span - (w.resets_at_ms - nowMs);
  if (!(elapsed > 60_000) || w.used <= 0 || w.used >= 1) return null;
  const rate = w.used / elapsed;
  const left = (1 - w.used) / rate;
  const at = nowMs + left;
  // Running out after the flip is not a projection worth showing.
  return at >= w.resets_at_ms ? null : Math.round(at);
}

/** "2 h 15 min" / "45 min" from a duration in ms. Empty for the past. */
export function humanDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "";
  const mins = Math.round(ms / 60_000);
  const h = Math.floor(mins / 60);
  return h > 0 ? `${h} h ${mins % 60} min` : `${mins} min`;
}

/**
 * Sequential ramp bucket of a heatmap cell: 0 (empty) to 4 (darkest step).
 * One hue, light to dark — never a rainbow (dataviz: sequential = one hue).
 */
export function cellLevel(value: number, max: number): 0 | 1 | 2 | 3 | 4 {
  if (!(value > 0) || !(max > 0)) return 0;
  const r = value / max;
  if (r <= 0.25) return 1;
  if (r <= 0.5) return 2;
  if (r <= 0.75) return 3;
  return 4;
}

/** Largest cell of the grid, the top of the heatmap legend. */
export function gridMax(grid: readonly (readonly number[])[]): number {
  let max = 0;
  for (const row of grid) for (const v of row) if (v > max) max = v;
  return max;
}

/** Total calls in the grid, for the caption under the heatmap. */
export function gridTotal(grid: readonly (readonly number[])[]): number {
  let total = 0;
  for (const row of grid) for (const v of row) total += v;
  return total;
}

/** Newest first, bounded. The timeline holds no history beyond the session. */
export function pushSwitch(list: readonly SwitchRow[], row: SwitchRow, max = MAX_SWITCHES): SwitchRow[] {
  return [row, ...list].slice(0, max);
}

/**
 * Command palette entries: one "switch to <account>" per account, plus the
 * tab itself. `src/routes/+layout.svelte` is shared, so the layout spreads
 * this list instead of knowing anything about accounts.
 */
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

// ---------------------------------------------------------------------------
// Deterministic fake (tests + `?demo=1` screenshots). Never a real screen.
// ---------------------------------------------------------------------------

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
          // No ceiling known for this plan: tokens only, and no bar.
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

export function fakeUsage(nowMs: number): CliUsageReport {
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
      // Weekday office hours are busy, nights and weekends are not.
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

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

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

/**
 * Switch timeline: rides `llm://rerouted`, which the backend emits when the
 * router moves a turn to another candidate. Event-driven, so the tab creates
 * no timer; leaving it unsubscribes.
 */
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
    // No event bridge (browser, tests): the timeline just stays empty.
  }
  return () => stopSwitchLog();
}

export function stopSwitchLog(): void {
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

/** One read of accounts + chain + rotation. Also the refresh button. */
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

/** The dashboard. Runs on tab open and on the refresh button, never on boot. */
export async function loadUsage(days = 30): Promise<void> {
  if (demo) return;
  usageState = usageState === "ready" ? "ready" : "loading";
  try {
    report = normalizeReport(await invoke<CliUsageReport>("llm_cli_usage_report", { days }));
    usageState = "ready";
    usageError = "";
    // The sweep is the only place the `estimated` windows come from, so the
    // cards get them the moment the dashboard lands.
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

/**
 * `share` symlinks the non-identity parts of the user's existing profile
 * (settings, skills, commands, agents, plugins) into the new one and copies
 * `.claude.json` without the account fields — the account-switcher model of
 * `estudos/74` §A.3. The credential is never touched either way.
 */
export async function createAccount(cli: CliKind, label: string, share = true): Promise<void> {
  await call("llm_accounts_create", { cli, label, share });
}

export async function removeAccount(id: string, purge = false): Promise<void> {
  await call("llm_accounts_remove", { id, purge });
}

export async function setDisabled(id: string, disabled: boolean): Promise<void> {
  await call("llm_accounts_set_disabled", { id, disabled });
}

/**
 * Change what a turn on this account may touch. The card confirms before it
 * calls this with `write`; the backend re-checks the word anyway.
 */
export async function setSandbox(id: string, sandbox: SandboxMode): Promise<void> {
  await call("llm_accounts_set_sandbox", { id, sandbox });
}

export async function activateAccount(id: string): Promise<void> {
  await call("llm_accounts_activate", { id });
}

export async function setChain(chain: string[]): Promise<void> {
  // Optimistic: the drag already moved the row, the backend confirms the order.
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

/** Opens a terminal in the account's config dir so the CLI signs itself in. */
export async function loginAccount(id: string): Promise<void> {
  if (demo) return;
  await invoke("llm_accounts_login", { id });
}

/** Screenshot/test mode: deterministic data, no IPC. */
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

/** Test seam: drop every piece of state between cases. */
export function resetAccountsStore(): void {
  stopSwitchLog();
  switches = [];
  snapshot = EMPTY_SNAPSHOT;
  report = EMPTY_REPORT;
  detected = [];
  accountsState = "idle";
  usageState = "idle";
  accountsError = "";
  usageError = "";
  demo = false;
  busy = false;
}
