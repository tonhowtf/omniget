import { invoke } from "@tauri-apps/api/core";

export type CliKind = "claude" | "codex";
type WindowKind = "five_hour" | "seven_day";

export interface UsageWindow {
  account_id: string;
  window: WindowKind;
  used: number | null;
  used_tokens: number;
  resets_at_ms: number | null;
}

export interface AccountView {
  id: string;
  cli: CliKind;
  label: string;
  disabled: boolean;
  windows: UsageWindow[];
  active: boolean;
  last_error: string | null;
}

interface AccountsSnapshot {
  accounts: AccountView[];
}

interface CliDetected {
  cli: CliKind;
  path: string;
  version: string | null;
}

type LoadState = "idle" | "loading" | "ready" | "unavailable";

const EMPTY_SNAPSHOT: AccountsSnapshot = { accounts: [] };

function clampFraction(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return Math.min(1, Math.max(0, value));
}

function normalizeWindow(raw: unknown, accountId = ""): UsageWindow {
  const window = (raw ?? {}) as Partial<UsageWindow>;
  return {
    account_id: window.account_id || accountId,
    window: window.window === "seven_day" ? "seven_day" : "five_hour",
    used: clampFraction(window.used),
    used_tokens: typeof window.used_tokens === "number" ? window.used_tokens : 0,
    resets_at_ms: window.resets_at_ms ?? null,
  };
}

function normalizeSnapshot(raw: unknown): AccountsSnapshot {
  const accounts = ((raw ?? {}) as Partial<AccountsSnapshot>).accounts ?? [];
  return {
    accounts: accounts.map((account) => ({
      id: account.id,
      cli: account.cli === "codex" ? "codex" : "claude",
      label: account.label || account.id,
      disabled: account.disabled === true,
      active: account.active === true,
      last_error: account.last_error ?? null,
      windows: (account.windows ?? []).map((window) => normalizeWindow(window, account.id)),
    })),
  };
}

function mergeWindows(accounts: readonly AccountView[], windows: readonly UsageWindow[]): AccountView[] {
  return accounts.map((account) => {
    const mine = windows.filter((window) => window.account_id === account.id);
    if (mine.length === 0) return account;
    const byKind = new Map(mine.map((window) => [window.window, window]));
    for (const window of account.windows) if (!byKind.has(window.window)) byKind.set(window.window, window);
    const order: WindowKind[] = ["five_hour", "seven_day"];
    return { ...account, windows: order.map((kind) => byKind.get(kind)).filter((window): window is UsageWindow => !!window) };
  });
}

export function accountPaletteItems(
  accounts: readonly AccountView[],
  labels: { group: string; switchTo: (label: string) => string; openTab: string },
  actions: { activate: (id: string) => void; open: () => void },
): Array<{ id: string; label: string; group: string; keywords: string; action: () => void }> {
  const items = [
    { id: "nav-agent-accounts", label: labels.openTab, group: labels.group, keywords: "accounts quota contas cota claude codex cli", action: actions.open },
  ];
  for (const account of accounts) {
    if (account.disabled) continue;
    items.push({
      id: `agent-account-${account.id}`,
      label: labels.switchTo(account.label),
      group: labels.group,
      keywords: `${account.label} ${account.cli} switch trocar conta account`,
      action: () => actions.activate(account.id),
    });
  }
  return items;
}

let snapshot = $state<AccountsSnapshot>(EMPTY_SNAPSHOT);
let detected = $state<CliDetected[]>([]);
let accountsState = $state<LoadState>("idle");
let accountsError = $state("");
let busy = $state(false);

export function getAccounts(): AccountsSnapshot {
  return snapshot;
}

export function getDetected(): CliDetected[] {
  return detected;
}

export function getAccountsState(): LoadState {
  return accountsState;
}

export function getAccountsError(): string {
  return accountsError;
}

export function isBusy(): boolean {
  return busy;
}

async function call(command: string, args?: Record<string, unknown>): Promise<void> {
  busy = true;
  try {
    snapshot = normalizeSnapshot(await invoke<AccountsSnapshot>(command, args));
    accountsState = "ready";
    accountsError = "";
    void loadWindows();
  } catch (error) {
    accountsError = String(error);
    if (accountsState !== "ready") accountsState = "unavailable";
    throw error;
  } finally {
    busy = false;
  }
}

async function loadWindows(): Promise<void> {
  try {
    const report = await invoke<{ windows?: unknown[] }>("llm_cli_usage_report", { days: 7 });
    const windows = (report?.windows ?? []).map((window) => normalizeWindow(window));
    if (windows.length > 0) snapshot = { accounts: mergeWindows(snapshot.accounts, windows) };
  } catch {
    return;
  }
}

export async function loadAccounts(): Promise<void> {
  accountsState = accountsState === "ready" ? "ready" : "loading";
  try {
    snapshot = normalizeSnapshot(await invoke<AccountsSnapshot>("llm_accounts_list"));
    accountsState = "ready";
    accountsError = "";
    void loadWindows();
  } catch (error) {
    accountsError = String(error);
    if (accountsState !== "ready") {
      snapshot = EMPTY_SNAPSHOT;
      accountsState = "unavailable";
    }
  }
}

export async function detectClis(): Promise<void> {
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

export async function activateAccount(id: string): Promise<void> {
  await call("llm_accounts_activate", { id });
}

export async function loginAccount(id: string): Promise<void> {
  await invoke("llm_accounts_login", { id });
}
