// UI-only state of the threads screen, kept outside the engine store:
// per-thread drafts (persisted, debounced), the send queue (memory only: "a
// queued message is a live intent, not a draft"), right-panel state with the
// "never yank the panel" counter, disclosure sets, live context usage from
// `threads://event`, and an undo toast.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ThreadsEventEnvelope } from "$lib/central/threads/types";

export type ChipKind = "file" | "skill" | "pr" | "terminal" | "review" | "attachment" | "command";

export type Chip = {
  id: string;
  kind: ChipKind;
  label: string;
  /** Text sent to the agent for this chip (context block or token). */
  value: string;
  /** Absolute path for attachments. */
  path?: string;
  title?: string;
};

export type Draft = { text: string; chips: Chip[] };

export type QueuedMessage = {
  id: string;
  text: string;
  chips: Chip[];
  createdAt: string;
  /** Held at the head after a failed send or a Stop. */
  hold: boolean;
};

export type PanelTab = "diff" | "terminal" | "files" | "plan" | "tasks" | "pr";

export type PanelState = { open: boolean; tab: PanelTab | null; touched: number; diffScope: "turn" | "total"; diffTurn: number | null; reveal: string | null };

export type UndoToast = { id: number; message: string; undo: () => void | Promise<void> };

const DRAFTS_KEY = "omniget.central.threads.drafts.v1";
const PANEL_KEY = "omniget.central.threads.panel.v1";
const SELECTED_KEY = "omniget.central.threads.selected";
const SIDEBAR_KEY = "omniget.central.threads.sidebar.v1";

function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

function writeJson(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* quota or private mode: drafts stay in memory */
  }
}

export const newId = (p: string) => `${p}_${crypto.randomUUID().replaceAll("-", "").slice(0, 12)}`;

class ThreadsUiState {
  drafts = $state<Record<string, Draft>>(readJson(DRAFTS_KEY, {}));
  queues = $state<Record<string, QueuedMessage[]>>({});
  panels = $state<Record<string, PanelState>>(readJson(PANEL_KEY, {}));
  expandedFolds = $state<Record<string, string[]>>({});
  collapsedFolds = $state<Record<string, string[]>>({});
  expandedGroups = $state<Record<string, string[]>>({});
  interruptedHere = $state<Record<string, string[]>>({});
  liveUsage = $state<Record<string, { used: number; max: number | null }>>({});
  selected = $state<string | null>(readJson<string | null>(SELECTED_KEY, null));
  sidebar = $state<{ collapsed: Record<string, boolean>; showArchived: boolean; snoozedOpen: boolean }>(
    readJson(SIDEBAR_KEY, { collapsed: {}, showArchived: false, snoozedOpen: false }),
  );
  undo = $state<UndoToast | null>(null);

  #draftTimer: ReturnType<typeof setTimeout> | null = null;
  #undoTimer: ReturnType<typeof setTimeout> | null = null;
  #unlisten: UnlistenFn | null = null;
  #listening = false;

  // ── Drafts ────────────────────────────────────────────────────────────

  draft(threadId: string): Draft {
    return this.drafts[threadId] ?? { text: "", chips: [] };
  }

  setDraft(threadId: string, d: Draft) {
    if (!d.text && !d.chips.length) {
      const { [threadId]: _gone, ...rest } = this.drafts;
      this.drafts = rest;
    } else {
      this.drafts = { ...this.drafts, [threadId]: { text: d.text, chips: d.chips } };
    }
    this.#persistDrafts();
  }

  hasDraft(threadId: string): boolean {
    const d = this.drafts[threadId];
    return !!d && (d.text.trim().length > 0 || d.chips.length > 0);
  }

  #persistDrafts() {
    if (this.#draftTimer) clearTimeout(this.#draftTimer);
    this.#draftTimer = setTimeout(() => this.flush(), 300);
  }

  flush() {
    if (this.#draftTimer) clearTimeout(this.#draftTimer);
    this.#draftTimer = null;
    writeJson(DRAFTS_KEY, $state.snapshot(this.drafts));
  }

  // ── Queue ─────────────────────────────────────────────────────────────

  queue(threadId: string): QueuedMessage[] {
    return this.queues[threadId] ?? [];
  }

  enqueue(threadId: string, text: string, chips: Chip[]) {
    const item: QueuedMessage = { id: newId("q"), text, chips, createdAt: new Date().toISOString(), hold: false };
    this.queues = { ...this.queues, [threadId]: [...this.queue(threadId), item] };
  }

  removeQueued(threadId: string, id: string): QueuedMessage | undefined {
    const list = this.queue(threadId);
    const item = list.find((q) => q.id === id);
    this.queues = { ...this.queues, [threadId]: list.filter((q) => q.id !== id) };
    return item;
  }

  takeNext(threadId: string): QueuedMessage | undefined {
    const [head, ...rest] = this.queue(threadId);
    if (!head || head.hold) return undefined;
    this.queues = { ...this.queues, [threadId]: rest };
    return head;
  }

  holdHead(threadId: string, item: QueuedMessage) {
    this.queues = { ...this.queues, [threadId]: [{ ...item, hold: true }, ...this.queue(threadId)] };
  }

  /** Stop drains the queue back into the composer draft. */
  drainToDraft(threadId: string) {
    const list = this.queue(threadId);
    if (!list.length) return;
    const d = this.draft(threadId);
    const text = [...list.map((q) => q.text), d.text].filter((s) => s.trim()).join("\n\n");
    const chips = [...list.flatMap((q) => q.chips), ...d.chips];
    this.queues = { ...this.queues, [threadId]: [] };
    this.setDraft(threadId, { text, chips });
  }

  // ── Panel ─────────────────────────────────────────────────────────────

  panel(threadId: string): PanelState {
    return this.panels[threadId] ?? { open: false, tab: null, touched: 0, diffScope: "turn", diffTurn: null, reveal: null };
  }

  /** A user choice: bumps `touched` so proactive opens stop. */
  setPanel(threadId: string, patch: Partial<PanelState>, byUser = true) {
    const cur = this.panel(threadId);
    const next = { ...cur, ...patch, touched: byUser ? cur.touched + 1 : cur.touched };
    this.panels = { ...this.panels, [threadId]: next };
    writeJson(PANEL_KEY, $state.snapshot(this.panels));
  }

  /** Proactive open (e.g. diff after a big turn): refused once the user touched the panel. */
  proactiveOpen(threadId: string, tab: PanelTab, patch: Partial<PanelState> = {}): boolean {
    const cur = this.panel(threadId);
    if (cur.touched > 0) return false;
    this.setPanel(threadId, { open: true, tab, ...patch }, false);
    return true;
  }

  // ── Disclosure sets ───────────────────────────────────────────────────

  #toggle(map: "expandedFolds" | "collapsedFolds" | "expandedGroups" | "interruptedHere", threadId: string, id: string, on?: boolean) {
    const cur = new Set(this[map][threadId] ?? []);
    const want = on ?? !cur.has(id);
    if (want) cur.add(id);
    else cur.delete(id);
    this[map] = { ...this[map], [threadId]: [...cur] };
  }

  setOf(map: "expandedFolds" | "collapsedFolds" | "expandedGroups" | "interruptedHere", threadId: string): Set<string> {
    return new Set(this[map][threadId] ?? []);
  }

  toggleFold(threadId: string, foldId: string, currentlyOpen: boolean) {
    if (currentlyOpen) {
      this.#toggle("expandedFolds", threadId, foldId, false);
      this.#toggle("collapsedFolds", threadId, foldId, true);
    } else {
      this.#toggle("collapsedFolds", threadId, foldId, false);
      this.#toggle("expandedFolds", threadId, foldId, true);
    }
  }

  toggleGroup(threadId: string, id: string) {
    this.#toggle("expandedGroups", threadId, id);
  }

  markInterrupted(threadId: string, turnId: string) {
    this.#toggle("interruptedHere", threadId, turnId, true);
  }

  /** Starting the next turn collapses the previous one if it was expanded. */
  collapseAllFolds(threadId: string) {
    this.expandedFolds = { ...this.expandedFolds, [threadId]: [] };
  }

  // ── Selection and sidebar ─────────────────────────────────────────────

  select(threadId: string | null) {
    this.selected = threadId;
    writeJson(SELECTED_KEY, threadId);
  }

  saveSidebar() {
    writeJson(SIDEBAR_KEY, $state.snapshot(this.sidebar));
  }

  // ── Undo toast ────────────────────────────────────────────────────────

  showUndo(message: string, undo: () => void | Promise<void>, ms = 6000) {
    if (this.#undoTimer) clearTimeout(this.#undoTimer);
    const id = Date.now();
    this.undo = { id, message, undo };
    this.#undoTimer = setTimeout(() => {
      if (this.undo?.id === id) this.undo = null;
    }, ms);
  }

  async runUndo() {
    const u = this.undo;
    this.undo = null;
    if (u) await u.undo();
  }

  dismissUndo() {
    this.undo = null;
  }

  // ── Live usage (driver-reported window) ───────────────────────────────

  async listenUsage() {
    if (this.#listening) return;
    this.#listening = true;
    this.#unlisten = await listen<ThreadsEventEnvelope>("threads://event", (e) => {
      const ev = e.payload.event;
      if (ev.type === "thread.turn-usage") {
        const u = ev.payload.usage;
        if (u.usedTokens != null || u.maxTokens != null) {
          const prev = this.liveUsage[ev.payload.threadId];
          this.liveUsage = {
            ...this.liveUsage,
            [ev.payload.threadId]: { used: u.usedTokens ?? prev?.used ?? 0, max: u.maxTokens ?? prev?.max ?? null },
          };
        }
      } else if (ev.type === "thread.activity-appended" && ev.payload.activity.kind === "thread.token-usage.updated") {
        const usage = ev.payload.activity.payload.usage as { usedTokens?: number; maxTokens?: number } | undefined;
        if (usage && (usage.usedTokens != null || usage.maxTokens != null)) {
          const prev = this.liveUsage[ev.payload.threadId];
          this.liveUsage = {
            ...this.liveUsage,
            [ev.payload.threadId]: { used: usage.usedTokens ?? prev?.used ?? 0, max: usage.maxTokens ?? prev?.max ?? null },
          };
        }
      }
    });
  }

  stopListening() {
    this.#unlisten?.();
    this.#unlisten = null;
    this.#listening = false;
  }
}

export const threadsUi = new ThreadsUiState();
