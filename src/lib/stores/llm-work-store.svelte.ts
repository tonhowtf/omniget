import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { get } from "svelte/store";
import { t } from "$lib/i18n";
import { showToast } from "$lib/stores/toast-store.svelte";
import { addLog } from "$lib/stores/debug-store.svelte";
import {
  getAsks,
  getJobs,
  getJobsReadError,
  pollAsks,
  getLoops,
  getRuns,
  initJobsStore,
} from "$lib/stores/llm-jobs-store.svelte";
import type { MissionRow } from "$lib/stores/assist-missions-store.svelte";
import { threadsStore } from "$lib/central/threads/threads-store.svelte";
import { getBudgetIncidents, getBudgetReadError, initBudgetStore, reloadBudget } from "$lib/stores/llm-budget-store.svelte";
import {
  deriveWork,
  permissionsOfAsks,
  freshNotifications,
  notificationKey,
  rememberSeen,
  
  type WorkItem,
  
} from "$lib/llm/work";

const SEEN_KEY = "omniget.llm.work.notified";

let missions = $state<MissionRow[]>([]);
let missionsError = $state<string | null>(null);
let now = $state(Date.now());
let started = false;
let missionReload: ReturnType<typeof setTimeout> | null = null;
const sessionStart = Date.now();

async function reloadMissions(): Promise<void> {
  try {
    missions = await invoke<MissionRow[]>("assist_mission_list", { filter: { limit: 100 } });
    missionsError = null;
  } catch (e) {
    missionsError = String(e);
    addLog("error", "system", `work: missions list failed: ${String(e)}`);
  }
}

function scheduleMissionReload(): void {
  if (missionReload) return;
  missionReload = setTimeout(() => {
    missionReload = null;
    void reloadMissions();
  }, 150);
}

export function initWorkStore(): void {
  if (started) return;
  started = true;
  initJobsStore();
  initBudgetStore();
  void threadsStore.init();
  void reloadMissions();
  void listen("assist://mission", () => scheduleMissionReload()).catch((e) => {
    missionsError = String(e);
  });
}

export function tickWorkClock(at = Date.now()): void {
  now = at;
}

export function getWorkItems(): WorkItem[] {
  return deriveWork({
    jobs: getJobs() ?? [],
    loops: getLoops() ?? [],
    asks: getAsks() ?? [],
    runs: getRuns() ?? [],
    permissions: permissionsOfAsks(getAsks() ?? []),
    missions: missions ?? [],
    threads: threadsStore.threads ?? [],
    threadApprovals: threadsStore.pendingApprovals ?? [],
    threadInputs: threadsStore.pendingUserInputs ?? [],
    incidents: getBudgetIncidents() ?? [],
    now,
  });
}

export function getWorkErrors(): string[] {
  const out: string[] = [];
  const jobsError = getJobsReadError();
  if (jobsError) out.push(jobsError);
  if (missionsError) out.push(missionsError);
  if (threadsStore.error) out.push(threadsStore.error);
  const budgetError = getBudgetReadError();
  if (budgetError) out.push(budgetError);
  return out;
}

export function retryWork(): void {
  void reloadMissions();
  if (getJobsReadError()) void pollAsks();
  if (threadsStore.error) void threadsStore.init();
  if (getBudgetReadError()) void reloadBudget();
}

function readSeen(): string[] {
  try {
    const raw = localStorage.getItem(SEEN_KEY);
    const list = raw ? (JSON.parse(raw) as unknown) : [];
    return Array.isArray(list) ? list.filter((k): k is string => typeof k === "string") : [];
  } catch {
    return [];
  }
}

function writeSeen(keys: string[]): void {
  try {
    localStorage.setItem(SEEN_KEY, JSON.stringify(keys));
  } catch (e) {
    addLog("warn", "system", `work: could not keep the notified list: ${String(e)}`);
  }
}

function pathOf(href: string): string {
  return href.split("?")[0] || "/";
}

export function announce(items: WorkItem[], path: string): WorkItem[] {
  const seen = readSeen();
  const fresh = freshNotifications(items, new Set(seen), sessionStart);
  if (fresh.length === 0) return [];
  writeSeen(rememberSeen(seen, fresh.map(notificationKey)));
  const tr = get(t);
  const shown = fresh.filter((i) => pathOf(i.href) !== path);
  for (const item of shown.slice(0, 3)) {
    const title = item.title || item.id;
    const key = `llm.work.notify.${item.pill}`;
    showToast(item.pill === "failed" ? "error" : item.section === "done" ? "success" : "info", tr(key, { title }) as string);
  }
  if (shown.length > 3) showToast("info", tr("llm.work.notify.more", { count: shown.length - 3 }) as string);
  return shown;
}
