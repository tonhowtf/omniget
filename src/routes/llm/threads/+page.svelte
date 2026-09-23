<script lang="ts">
  /**
   * Central › Threads: projects → threads sidebar, the thread timeline, the
   * composer with its approval / question / plan drawer, and the right panel
   * (Diff, Terminal, Files, Plan, Tasks, PR). Everything is event-driven from
   * `threads://event` through `threadsStore`; nothing polls. Timers exist only
   * while a turn runs (elapsed labels) or a snooze is about to wake.
   */
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { threadsStore } from "$lib/central/threads/threads-store.svelte";
  import type { AccessMode, ApprovalDecision, PlanRow, ThreadRow } from "$lib/central/threads/types";
  import { threadsUi, type Chip, type PanelTab, type QueuedMessage } from "$lib/central/threads-ui/ui-state.svelte";
  import { indicators } from "$lib/central/threads-ui/indicators.svelte";
  import { changedFilesOf, deriveRows, latestPlan, latestUsage, rateLimitsOf, tasksOf, userPrompts, type Row } from "$lib/central/threads-ui/timeline";
  import { attachmentsOf, composeMessage, recallable } from "$lib/central/threads-ui/composer";
  import { contextFromUsage } from "$lib/central/threads-ui/context-window";
  import { ensureCheckpoint, threadRevertToTurn, threadTerminalCwd } from "$lib/central/threads-ui/api";
  import { ACCESS_MODES, driverName, driverTint } from "$lib/central/threads-ui/drivers";
  import { basename, relativeTo } from "$lib/central/threads-ui/format";
  import { sidebarStatus } from "$lib/central/threads-ui/status";
  import { toFileDiffs } from "$lib/central/threads-ui/work";
  import { reviewCommentToPrompt, fenced } from "$components/central/vcs/diff-model";
  import type { ReviewComment } from "$lib/central/vcs";
  import { vcsError } from "$lib/central/vcs";
  import type { TerminalSelection } from "$lib/central/pty";
  import ThreadsSidebar from "$components/central/threads/ThreadsSidebar.svelte";
  import Timeline from "$components/central/threads/Timeline.svelte";
  import type { TimelineActions } from "$components/central/threads/TimelineRow.svelte";
  import ThreadComposer from "$components/central/threads/ThreadComposer.svelte";
  import ApprovalDrawer from "$components/central/threads/ApprovalDrawer.svelte";
  import QuestionCard from "$components/central/threads/QuestionCard.svelte";
  import ContextMeter from "$components/central/threads/ContextMeter.svelte";
  import RightPanel from "$components/central/threads/RightPanel.svelte";
  import DiffPanel from "$components/central/threads/DiffPanel.svelte";
  import FilesPanel from "$components/central/threads/FilesPanel.svelte";
  import PlanPanel from "$components/central/threads/PlanPanel.svelte";
  import TasksPanel from "$components/central/threads/TasksPanel.svelte";
  import PrPanel from "$components/central/threads/PrPanel.svelte";
  import NewThreadDialog from "$components/central/threads/NewThreadDialog.svelte";
  import RevertDialog from "$components/central/threads/RevertDialog.svelte";
  import UndoToast from "$components/central/threads/UndoToast.svelte";
  import Icon from "$components/central/threads/Icon.svelte";
  import TerminalPanel from "$components/central/terminal/TerminalPanel.svelte";
  import GitActionsButton from "$components/central/vcs/GitActionsButton.svelte";
  import ShortcutsProvider, { type ShortcutHandler } from "$components/central/shortcuts/ShortcutsProvider.svelte";

  // ── Boot ──────────────────────────────────────────────────────────────
  onMount(() => {
    void threadsStore.init();
    void threadsUi.listenUsage();
    const flush = () => threadsUi.flush();
    window.addEventListener("beforeunload", flush);
    return () => window.removeEventListener("beforeunload", flush);
  });
  onDestroy(() => {
    threadsUi.flush();
    threadsUi.stopListening();
    indicators.stop();
  });

  $effect(() => {
    if (!threadsStore.ready) return;
    const ids = threadsStore.threads.map((x) => x.threadId);
    untrack(() => {
      if (!indicatorsStarted) {
        indicatorsStarted = true;
        void indicators.start(ids);
      } else indicators.setThreads(ids);
    });
  });
  let indicatorsStarted = false;

  // ── Selection ─────────────────────────────────────────────────────────
  let selectedId = $derived(threadsUi.selected);
  let thread = $derived(threadsStore.threads.find((x) => x.threadId === selectedId) ?? null);
  let project = $derived(thread ? (threadsStore.projects.find((p) => p.projectId === thread!.projectId) ?? null) : null);
  let cwd = $derived(thread?.worktreePath ?? project?.workspaceRoot ?? null);
  let detail = $derived(selectedId ? threadsStore.details[selectedId] : undefined);
  let inst = $derived(thread ? (threadsStore.instances.find((i) => i.instance.id === thread!.instanceId) ?? null) : null);
  let caps = $derived(inst?.capabilities ?? null);

  // ?thread=<id> deep link.
  $effect(() => {
    const q = page.url.searchParams.get("thread");
    if (q && threadsStore.ready) untrack(() => select(q));
  });

  // Open the selected thread's detail (and drop details of threads not shown).
  let openedFor = "";
  $effect(() => {
    const id = selectedId;
    if (!threadsStore.ready || !id) return;
    if (!threadsStore.threads.some((x) => x.threadId === id)) return;
    untrack(() => {
      if (openedFor === id && threadsStore.details[id]) return;
      const prev = openedFor;
      openedFor = id;
      void threadsStore.openThread(id).then(() => {
        void threadsStore.dispatch({ type: "thread.visit", threadId: id }).catch(() => undefined);
      });
      if (prev && prev !== id) threadsStore.closeThread(prev);
      const th = threadsStore.threads.find((x) => x.threadId === id);
      const root = th ? (th.worktreePath ?? threadsStore.projects.find((p) => p.projectId === th.projectId)?.workspaceRoot) : null;
      if (root && indicators.prs[id] === undefined) void import("$lib/central/threads-ui/api").then((m) => m.prLookup(root).then((pr) => indicators.setPr(id, pr)));
    });
  });

  function select(id: string) {
    threadsUi.select(id);
    sidebarOpen = false;
    void tick().then(() => composer?.focus());
  }

  // ── Timeline rows ─────────────────────────────────────────────────────
  let expandedIds = $derived(selectedId ? threadsUi.setOf("expandedGroups", selectedId) : new Set<string>());
  let rows = $derived<Row[]>(
    selectedId
      ? deriveRows(detail, {
          expandedFolds: threadsUi.setOf("expandedFolds", selectedId),
          collapsedFolds: threadsUi.setOf("collapsedFolds", selectedId),
          expandedGroups: expandedIds,
          interruptedHere: threadsUi.setOf("interruptedHere", selectedId),
          latestTurnId: thread?.latestTurnId ?? null,
          planMode: thread?.interactionMode === "plan",
        })
      : [],
  );
  let running = $derived(!!thread?.activeTurnId);
  let approvals = $derived(threadsStore.pendingApprovals.filter((a) => a.threadId === selectedId).sort((a, b) => a.createdAt.localeCompare(b.createdAt)));
  let question = $derived(threadsStore.pendingUserInputs.filter((a) => a.threadId === selectedId).sort((a, b) => a.createdAt.localeCompare(b.createdAt))[0] ?? null);
  let queue = $derived(selectedId ? threadsUi.queue(selectedId) : []);
  let history = $derived(userPrompts(detail));
  let plans = $derived(latestPlan(detail));
  let taskList = $derived(tasksOf(detail));
  let settledTurns = $derived((detail?.turns ?? []).filter((x) => x.turn.state !== "pending" && x.turn.state !== "running").map((x) => x.turn.ordinal));
  let touched = $derived.by(() => {
    const map = new Map<string, { path: string; status: string; additions: number; deletions: number }>();
    for (const turn of detail?.turns ?? []) for (const f of changedFilesOf(turn).files) {
      const cur = map.get(f.path);
      if (cur) {
        cur.additions += f.additions;
        cur.deletions += f.deletions;
      } else map.set(f.path, { ...f });
    }
    return [...map.values()];
  });

  // Context meter: driver-reported window first, then an estimate.
  let context = $derived.by(() => {
    if (!selectedId) return null;
    const live = threadsUi.liveUsage[selectedId];
    if (live && live.used) return { used: live.used, max: live.max, estimated: false };
    return contextFromUsage(latestUsage(detail), thread?.model ?? null, thread?.driver ?? null);
  });
  let limits = $derived(rateLimitsOf(detail));

  // ── Composer & sending ────────────────────────────────────────────────
  let composer = $state<ReturnType<typeof ThreadComposer> | null>(null);
  let composerText = $state("");
  let banner = $state<string | null>(null);
  let steering = $state<Record<string, boolean>>({});
  let responding = $state(new Set<string>());
  let answering = $state(false);

  async function startTurn(th: ThreadRow, text: string, chips: Chip[]): Promise<boolean> {
    const body = composeMessage(text, chips);
    if (!body.trim()) return false;
    const root = th.worktreePath ?? threadsStore.projects.find((p) => p.projectId === th.projectId)?.workspaceRoot ?? null;
    if (root) await ensureCheckpoint(root, th.threadId, th.turnCount);
    threadsUi.collapseAllFolds(th.threadId);
    try {
      await threadsStore.dispatch({
        type: "thread.turn.start",
        threadId: th.threadId,
        text: body,
        attachments: attachmentsOf(chips),
        model: th.model ?? undefined,
      });
      banner = null;
      return true;
    } catch (e) {
      banner = String(e);
      return false;
    }
  }

  async function send(text: string, chips: Chip[]): Promise<boolean> {
    const th = thread;
    if (!th) return false;
    // A single open question that takes free text: the composer is the answer field.
    if (question && !approvals.length && question.questions.length === 1 && question.questions[0].allowCustomAnswer && text.trim()) {
      await answerQuestion({ [question.questions[0].id]: text.trim() });
      return true;
    }
    if (th.activeTurnId || approvals.length || question) {
      threadsUi.enqueue(th.threadId, text, chips);
      return true;
    }
    return startTurn(th, text, chips);
  }

  /** ⌘Enter / "Send now": interrupt the running turn and send this first. */
  async function steer(text: string, chips: Chip[]): Promise<boolean> {
    const th = thread;
    if (!th) return false;
    if (!th.activeTurnId) return startTurn(th, text, chips);
    threadsUi.queues = { ...threadsUi.queues, [th.threadId]: [{ id: `q_${Date.now()}`, text, chips, createdAt: new Date().toISOString(), hold: false }, ...threadsUi.queue(th.threadId)] };
    steering = { ...steering, [th.threadId]: true };
    await threadsStore.interrupt(th.threadId).catch((e) => (banner = String(e)));
    return true;
  }

  async function sendQueuedNow(q: QueuedMessage) {
    const th = thread;
    if (!th) return;
    threadsUi.removeQueued(th.threadId, q.id);
    await steer(q.text, q.chips);
  }

  function cancelQueued(q: QueuedMessage) {
    if (!thread) return;
    threadsUi.removeQueued(thread.threadId, q.id);
    composer?.insertText(q.text);
    for (const c of q.chips) composer?.addChip(c);
  }

  /** Stop drains the queue back into the composer instead of starting it. */
  async function stop() {
    const th = thread;
    if (!th) return;
    if (th.activeTurnId) threadsUi.markInterrupted(th.threadId, th.activeTurnId);
    if (!steering[th.threadId]) {
      const queued = threadsUi.queue(th.threadId);
      threadsUi.queues = { ...threadsUi.queues, [th.threadId]: [] };
      for (const q of queued) {
        composer?.insertText(q.text);
        for (const c of q.chips) composer?.addChip(c);
      }
    }
    await threadsStore.interrupt(th.threadId).catch((e) => (banner = String(e)));
  }

  // Turn boundaries: release the queue, capture the checkpoint, maybe open the diff.
  let lastActive = $state<Record<string, string | null>>({});
  $effect(() => {
    const snapshot = threadsStore.threads.map((x) => [x.threadId, x.activeTurnId, x.pendingApprovalCount + x.pendingUserInputCount] as const);
    untrack(() => {
      for (const [id, active, pending] of snapshot) {
        const prev = lastActive[id];
        if (prev && !active) void onTurnSettled(id, prev);
        if (!active && !pending && (threadsUi.queues[id]?.length ?? 0) > 0) void releaseQueue(id);
      }
      lastActive = Object.fromEntries(snapshot.map(([id, active]) => [id, active]));
    });
  });

  let releasing = new Set<string>();
  async function releaseQueue(threadId: string) {
    if (releasing.has(threadId)) return;
    const th = threadsStore.threads.find((x) => x.threadId === threadId);
    if (!th || th.activeTurnId) return;
    const next = threadsUi.takeNext(threadId);
    if (!next) return;
    releasing.add(threadId);
    steering = { ...steering, [threadId]: false };
    const ok = await startTurn(th, next.text, next.chips);
    if (!ok) threadsUi.holdHead(threadId, next);
    releasing.delete(threadId);
  }

  async function onTurnSettled(threadId: string, turnId: string) {
    const th = threadsStore.threads.find((x) => x.threadId === threadId);
    if (!th) return;
    const root = th.worktreePath ?? threadsStore.projects.find((p) => p.projectId === th.projectId)?.workspaceRoot;
    // End of turn N: always (re)capture N, so a turn redone after a revert never reuses a stale ref.
    if (root) await ensureCheckpoint(root, threadId, th.turnCount, true);
    if (threadId === threadsUi.selected) {
      void threadsStore.dispatch({ type: "thread.visit", threadId }).catch(() => undefined);
      const turn = threadsStore.details[threadId]?.turns.find((x) => x.turn.turnId === turnId);
      if (turn) {
        const { files } = changedFilesOf(turn);
        const lines = files.reduce((n, f) => n + f.additions + f.deletions, 0);
        if (files.length >= 3 || lines >= 50) threadsUi.proactiveOpen(threadId, "diff", { diffScope: "turn", diffTurn: turn.turn.ordinal });
      }
    }
  }

  // ── Commands from the composer ────────────────────────────────────────
  async function run(cmd: Parameters<typeof threadsStore.dispatch>[0]) {
    try {
      await threadsStore.dispatch(cmd);
      banner = null;
    } catch (e) {
      banner = String(e);
    }
  }

  function onCommand(name: string) {
    const th = thread;
    if (!th) return;
    if (name.startsWith("instance:")) {
      const id = name.slice(9);
      const i = threadsStore.instances.find((x) => x.instance.id === id);
      if (i) void run({ type: "thread.instance.set", threadId: th.threadId, instanceId: i.instance.id, driver: i.instance.driver, model: th.model ?? undefined });
      return;
    }
    if (name.startsWith("model:")) {
      const m = name.slice(6);
      void run({ type: "thread.instance.set", threadId: th.threadId, instanceId: th.instanceId, driver: th.driver, model: m || undefined, agentId: th.agentId ?? undefined });
      return;
    }
    if (name.startsWith("access:")) {
      void run({ type: "thread.runtime-mode.set", threadId: th.threadId, runtimeMode: name.slice(7) as AccessMode });
      return;
    }
    switch (name) {
      case "model":
        composer?.focusModel();
        break;
      case "plan":
      case "default":
        void run({ type: "thread.interaction-mode.set", threadId: th.threadId, interactionMode: name });
        break;
      case "access": {
        const next = ACCESS_MODES[(ACCESS_MODES.indexOf(th.runtimeMode) + 1) % ACCESS_MODES.length];
        void run({ type: "thread.runtime-mode.set", threadId: th.threadId, runtimeMode: next });
        break;
      }
      case "fork":
        void forkFrom(th.turnCount);
        break;
      case "new":
        openNew(th.projectId);
        break;
      case "diff":
        setPanel({ open: true, tab: "diff" });
        break;
      case "terminal":
        setPanel({ open: true, tab: "terminal" });
        break;
      case "compact":
        void send("/compact", []);
        break;
    }
  }

  async function respondApproval(requestId: string, decision: ApprovalDecision) {
    const th = thread;
    if (!th) return;
    responding = new Set([...responding, requestId]);
    try {
      await threadsStore.respondApproval(th.threadId, requestId, decision);
    } catch (e) {
      banner = String(e);
    } finally {
      const next = new Set(responding);
      next.delete(requestId);
      responding = next;
    }
  }

  async function answerQuestion(answers: Record<string, string | string[]>) {
    const th = thread;
    if (!th || !question) return;
    answering = true;
    try {
      await threadsStore.respondUserInput(th.threadId, question.requestId, answers);
      if (question.questions.some((q) => q.allowCustomAnswer) && composerText.trim()) composer?.clear();
    } catch (e) {
      banner = String(e);
    } finally {
      answering = false;
    }
  }

  async function dismissQuestion() {
    const th = thread;
    if (!th || !question) return;
    await threadsStore.respondUserInput(th.threadId, question.requestId, null).catch((e) => (banner = String(e)));
  }

  // ── Plans ─────────────────────────────────────────────────────────────
  const IMPLEMENT = "PLEASE IMPLEMENT THIS PLAN:\n";
  async function implementPlan(plan: PlanRow) {
    const th = thread;
    if (!th) return;
    await run({ type: "thread.interaction-mode.set", threadId: th.threadId, interactionMode: "default" });
    await startTurn({ ...th, interactionMode: "default" }, IMPLEMENT + (plan.markdown ?? ""), []);
  }

  function refinePlan() {
    composer?.focus(true);
  }

  async function implementPlanNew(plan: PlanRow) {
    const th = thread;
    if (!th) return;
    const title = /^\s*#{1,6}\s+(.+)$/m.exec(plan.markdown ?? "")?.[1]?.trim() ?? th.title;
    const newId = `thr_${crypto.randomUUID().replaceAll("-", "").slice(0, 16)}`;
    try {
      await threadsStore.dispatch({
        type: "thread.create",
        threadId: newId,
        projectId: th.projectId,
        title: `${$t("llm.central.threads.plan.implement_prefix")} ${title}`,
        instanceId: th.instanceId,
        driver: th.driver,
        model: th.model ?? undefined,
        runtimeMode: th.runtimeMode,
        interactionMode: "default",
        branch: th.branch ?? undefined,
        worktreePath: th.worktreePath ?? undefined,
      });
      await threadsStore.dispatch({ type: "thread.turn.start", threadId: newId, text: IMPLEMENT + (plan.markdown ?? "") });
      select(newId);
    } catch (e) {
      banner = String(e);
      await threadsStore.dispatch({ type: "thread.delete", threadId: newId }).catch(() => undefined);
    }
  }

  // ── Edit from here / fork ─────────────────────────────────────────────
  let revertRow = $state<Extract<Row, { kind: "user" }> | null>(null);
  let revertBusy = $state(false);
  let revertError = $state<string | null>(null);

  async function confirmRevert(restoreFiles: boolean) {
    const th = thread;
    const row = revertRow;
    if (!th || !row || row.revertTo == null || !cwd) return;
    revertBusy = true;
    revertError = null;
    try {
      await threadRevertToTurn(th.threadId, cwd, row.revertTo, restoreFiles, (n) => threadsStore.revert(th.threadId, n));
      composer?.insertText(recallable(row.message.text));
      revertRow = null;
      await threadsStore.openThread(th.threadId);
    } catch (e) {
      revertError = vcsError(e).message;
    } finally {
      revertBusy = false;
    }
  }

  async function forkFrom(turnCount: number) {
    const th = thread;
    if (!th) return;
    try {
      const id = await threadsStore.fork(th.threadId, turnCount);
      select(id);
    } catch (e) {
      banner = String(e);
    }
  }

  // ── Right panel ───────────────────────────────────────────────────────
  let panel = $derived(selectedId ? threadsUi.panel(selectedId) : null);
  function setPanel(patch: Parameters<typeof threadsUi.setPanel>[1]) {
    if (selectedId) threadsUi.setPanel(selectedId, patch);
  }

  function openFile(path: string) {
    const rel = relativeTo(path, cwd);
    setPanel({ open: true, tab: "diff", diffScope: "total", reveal: rel });
  }

  function openDiff(turnOrdinal: number | null, path?: string | null) {
    setPanel({
      open: true,
      tab: "diff",
      diffScope: turnOrdinal == null ? "total" : "turn",
      diffTurn: turnOrdinal,
      reveal: path ? relativeTo(path, cwd) : null,
    });
  }

  function addTerminalSelection(sel: TerminalSelection) {
    const [a, b] = sel.lines;
    composer?.addChip({
      kind: "terminal",
      label: `terminal:${a}${b !== a ? `-${b}` : ""}`,
      title: sel.text.slice(0, 400),
      value: `Terminal output (lines ${a}-${b}):\n${fenced(sel.text, "text")}`,
    });
  }

  function addReview(c: ReviewComment) {
    composer?.addChip({ kind: "review", label: `${basename(c.path)} ${c.rangeLabel}`, title: c.text || c.path, value: reviewCommentToPrompt(c) });
  }

  let terminalCwd = $state<string | null>(null);
  $effect(() => {
    const id = selectedId;
    const root = cwd;
    if (!id || !root || panel?.tab !== "terminal") return;
    let alive = true;
    void threadTerminalCwd(id, root).then((c) => {
      if (alive) terminalCwd = c;
    });
    return () => {
      alive = false;
    };
  });

  let turnFallback = $derived.by(() => {
    const n = panel?.diffTurn ?? settledTurns.at(-1);
    const turn = detail?.turns.find((x) => x.turn.ordinal === n);
    if (!turn) return null;
    const { patches } = changedFilesOf(turn);
    return patches.length ? toFileDiffs(patches.map((p) => p.patch).join("\n")) : null;
  });

  // Panel width (drag handle), remembered.
  let panelWidth = $state(readWidth());
  function readWidth(): number {
    try {
      return Number(localStorage.getItem("omniget.central.threads.panelWidth")) || 460;
    } catch {
      return 460;
    }
  }
  function startResize(e: PointerEvent) {
    const startX = e.clientX;
    const start = panelWidth;
    const move = (ev: PointerEvent) => (panelWidth = Math.max(320, Math.min(window.innerWidth * 0.7, start + (startX - ev.clientX))));
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      try {
        localStorage.setItem("omniget.central.threads.panelWidth", String(Math.round(panelWidth)));
      } catch {
        /* optional */
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }
  function resizeKey(e: KeyboardEvent) {
    if (e.key === "ArrowLeft") panelWidth = Math.min(window.innerWidth * 0.7, panelWidth + 24);
    else if (e.key === "ArrowRight") panelWidth = Math.max(320, panelWidth - 24);
  }

  // ── New thread / project ──────────────────────────────────────────────
  let newOpen = $state(false);
  let newProject = $state<string | null>(null);
  function openNew(projectId: string | null) {
    newProject = projectId ?? thread?.projectId ?? null;
    newOpen = true;
  }

  async function addProject() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const dir = await open({ directory: true, multiple: false });
      if (!dir || Array.isArray(dir)) return;
      const root = String(dir);
      const existing = threadsStore.projects.find((p) => p.workspaceRoot === root);
      if (existing) {
        openNew(existing.projectId);
        return;
      }
      await threadsStore.createProject(basename(root), root);
      await tick();
      const created = threadsStore.projects.find((p) => p.workspaceRoot === root);
      if (created) openNew(created.projectId);
    } catch (e) {
      banner = String(e);
    }
  }

  async function onCreated(threadId: string, first: string) {
    newOpen = false;
    select(threadId);
    if (first) {
      await tick();
      const th = threadsStore.threads.find((x) => x.threadId === threadId);
      if (th) await startTurn(th, first, []);
    }
  }

  // ── Header actions ────────────────────────────────────────────────────
  let renamingTitle = $state(false);
  let titleDraft = $state("");
  async function commitTitle() {
    renamingTitle = false;
    const th = thread;
    if (!th || !titleDraft.trim() || titleDraft.trim() === th.title) return;
    await run({ type: "thread.rename", threadId: th.threadId, title: titleDraft.trim() });
  }

  let sidebarOpen = $state(false);

  // ── Keyboard: the Central shortcut table (ShortcutsProvider) ──────────
  // Approvals, questions, send/newline, ⌘. inside the composer, ⌘Z of the undo
  // toast and the terminal keys stay native in their components.
  function liveThreads() {
    return threadsStore.threads.filter((x) => !x.archivedAt);
  }
  function step(dir: number) {
    const list = liveThreads();
    if (!list.length) return;
    const i = list.findIndex((x) => x.threadId === selectedId);
    const next = list[(i + dir + list.length) % list.length];
    if (next) select(next.threadId);
  }
  async function toggleThreadFlag(kind: "pin" | "archive" | "snooze") {
    const th = thread;
    if (!th) return;
    if (kind === "pin") {
      const pinned = !!th.pinnedAt;
      await run({ type: pinned ? "thread.unpin" : "thread.pin", threadId: th.threadId });
      threadsUi.showUndo(th.title, () => run({ type: pinned ? "thread.pin" : "thread.unpin", threadId: th.threadId }));
    } else if (kind === "archive") {
      await run({ type: "thread.archive", threadId: th.threadId });
      threadsUi.showUndo($t("llm.central.threads.toast.archived", { title: th.title }) as string, () => run({ type: "thread.unarchive", threadId: th.threadId }));
    } else {
      const until = new Date(Date.now() + 60 * 60 * 1000);
      await run({ type: "thread.snooze", threadId: th.threadId, until: until.toISOString() });
      threadsUi.showUndo($t("llm.central.threads.toast.snoozed", { title: th.title }) as string, () => run({ type: "thread.unsnooze", threadId: th.threadId }));
    }
  }
  let shortcutHandlers = $derived<Partial<Record<string, ShortcutHandler>>>({
    "thread.new": () => openNew(null),
    "thread.newInProject": () => openNew(thread?.projectId ?? null),
    "thread.previous": () => step(-1),
    "thread.next": () => step(1),
    "thread.jump": (_e, n) => {
      const th = n ? liveThreads()[n - 1] : undefined;
      if (th) select(th.threadId);
    },
    "thread.archive": () => void toggleThreadFlag("archive"),
    "thread.pin": () => void toggleThreadFlag("pin"),
    "thread.snooze": () => void toggleThreadFlag("snooze"),
    "thread.rename": () => {
      if (!thread) return;
      titleDraft = thread.title;
      renamingTitle = true;
    },
    "thread.copyId": () => thread && void navigator.clipboard.writeText(thread.threadId).catch(() => undefined),
    "thread.fork": () => onCommand("fork"),
    "project.add": () => void addProject(),
    "sidebar.toggle": () => (sidebarOpen = !sidebarOpen),
    "panel.toggle": () => panel && setPanel({ open: !panel.open }),
    "panel.close": () => setPanel({ open: false }),
    "panel.diff": () => setPanel({ open: true, tab: "diff" }),
    "panel.terminal": () => setPanel({ open: true, tab: "terminal" }),
    "panel.files": () => setPanel({ open: true, tab: "files" }),
    "panel.plan": () => setPanel({ open: true, tab: "plan" }),
    "panel.agents": () => setPanel({ open: true, tab: "tasks" }),
    "panel.pr": () => setPanel({ open: true, tab: "pr" }),
    "composer.focus": () => composer?.focus(true),
    "composer.model": () => composer?.focusModel(),
    "composer.mode": () => onCommand(thread?.interactionMode === "plan" ? "default" : "plan"),
    "composer.access": () => onCommand("access"),
    "turn.interrupt": () => void stop(),
  });

  let actions = $derived<TimelineActions>({
    openFile,
    openDiff,
    editFrom: (row) => {
      revertError = null;
      revertRow = row;
    },
    forkFrom: (n) => void forkFrom(n),
    toggleFold: (id, open) => selectedId && threadsUi.toggleFold(selectedId, id, open),
    toggleGroup: (id) => selectedId && threadsUi.toggleGroup(selectedId, id),
    implementPlan: (p) => void implementPlan(p),
    refinePlan,
    implementPlanNew: (p) => void implementPlanNew(p),
    quote: (s) => composer?.insertText(`> ${s.replace(/\n/g, "\n> ")}`),
    canRevert: caps ? caps.rollback : true,
    canFork: caps ? caps.fork : false,
    busy: running || revertBusy,
  });

  let planReady = $derived(
    !!thread && thread.interactionMode === "plan" && !running && !!plans.proposed && plans.proposed.turnId === thread.latestTurnId && !question,
  );
  let status = $derived(thread ? sidebarStatus(thread) : "idle");
  let errorText = $derived(banner ?? (thread?.lastError && status === "failed" ? thread.lastError : null) ?? threadsStore.error);
</script>

<svelte:head><title>{thread ? `${thread.title} · ` : ""}{$t("llm.central.threads.title")}</title></svelte:head>
<ShortcutsProvider
  handlers={shortcutHandlers}
  context={() => ({ threadOpen: !!thread, panelOpen: !!panel?.open, running, approvalPending: approvals.length > 0, questionPending: !!question })}
  threads={liveThreads().map((x) => ({ id: x.threadId, title: x.title, subtitle: threadsStore.projects.find((p) => p.projectId === x.projectId)?.title ?? "" }))}
  onopenthread={select}
/>

<div class="threads" class:panel-open={!!panel?.open} style:--panel-w="{panelWidth}px">
  <div class="side" class:open={sidebarOpen}>
    <ThreadsSidebar {selectedId} onselect={select} onnewthread={openNew} onnewproject={addProject} />
  </div>

  <main class="main">
    {#if thread}
      <header class="thread-head">
        <button type="button" class="icon-btn only-narrow" aria-label={$t("llm.central.threads.sidebar")} onclick={() => (sidebarOpen = !sidebarOpen)}>
          <Icon name="sidebar-simple" size={16} />
        </button>
        <div class="crumb">
          <button type="button" class="proj" title={$t("llm.central.threads.new_in_project", { project: project?.title ?? "" })} onclick={() => openNew(thread!.projectId)}>
            {project?.title ?? thread.projectId}
          </button>
          <span class="sep">/</span>
          {#if renamingTitle}
            <!-- svelte-ignore a11y_autofocus -->
            <input class="title-input" bind:value={titleDraft} autofocus onblur={commitTitle} onkeydown={(e) => {
              if (e.key === "Enter") commitTitle();
              else if (e.key === "Escape") renamingTitle = false;
            }} aria-label={$t("llm.central.threads.rename")} />
          {:else}
            <h1 class="title" ondblclick={() => {
              titleDraft = thread!.title;
              renamingTitle = true;
            }}>{thread.title || $t("llm.central.threads.untitled")}</h1>
          {/if}
        </div>
        <span class="harness" style:--tint={driverTint(thread.driver, inst)}><span class="dot"></span>{driverName(thread.driver, inst)}</span>
        {#if thread.worktreePath}
          <span class="chip" title={thread.worktreePath}><Icon name="git-branch" size={12} />{thread.branch ?? basename(thread.worktreePath)}</span>
        {/if}
        <span class="grow"></span>
        {#if cwd}<GitActionsButton path={cwd} compact onpr={(p) => indicators.setPr(thread!.threadId, p)} />{/if}
        <button
          type="button"
          class="icon-btn"
          aria-pressed={!!panel?.open}
          title={`${$t("llm.central.threads.panel.toggle")} (⌘J)`}
          aria-label={$t("llm.central.threads.panel.toggle")}
          onclick={() => setPanel({ open: !panel?.open })}
        >
          <Icon name="sidebar-simple" size={16} />
        </button>
      </header>

      <div class="banner-slot">
        {#if errorText}
          <div class="banner" role="alert">
            <Icon name="warning-circle" size={14} />
            <span class="btext">{errorText}</span>
            <button type="button" class="icon-btn small" aria-label={$t("llm.central.threads.dismiss")} onclick={() => {
              banner = null;
              threadsStore.error = null;
            }}><Icon name="x" size={12} /></button>
          </div>
        {/if}
      </div>

      <Timeline
        {rows}
        threadId={thread.threadId}
        {cwd}
        {actions}
        {expandedIds}
        hasMore={detail?.hasMore ?? false}
        loadingOlder={detail?.loading ?? false}
        onloadolder={() => threadsStore.loadOlder(thread!.threadId)}
      >
        {#snippet footer()}
          {#each queue as q, i (q.id)}
            <div class="queued">
              <div class="q-bubble">
                <p>{q.text}</p>
                {#if q.chips.length}<span class="q-meta">{$t("llm.central.threads.queue.context", { count: q.chips.length })}</span>{/if}
              </div>
              <div class="q-actions">
                <span class="q-badge" title={i === 0 ? $t("llm.central.threads.queue.next_hint") : $t("llm.central.threads.queue.after_hint")}>
                  <Icon name="clock" size={11} />{q.hold ? $t("llm.central.threads.queue.held") : $t("llm.central.threads.queue.queued")}
                </span>
                <button type="button" class="ghost" onpointerdown={(e) => e.preventDefault()} onclick={() => sendQueuedNow(q)} title={`${$t("llm.central.threads.queue.send_now")} (⌘↵)`}>
                  <Icon name="arrow-up" size={12} />{$t("llm.central.threads.queue.send_now")}
                </button>
                <button type="button" class="ghost" onpointerdown={(e) => e.preventDefault()} onclick={() => cancelQueued(q)} aria-label={$t("llm.central.threads.queue.cancel")}>
                  <Icon name="x" size={12} />
                </button>
              </div>
            </div>
          {/each}
        {/snippet}
      </Timeline>

      <ThreadComposer
        bind:this={composer}
        bind:text={composerText}
        {thread}
        instances={threadsStore.instances}
        {cwd}
        {running}
        blocked={approvals.length > 0 || !!question}
        {history}
        onsend={send}
        onsteer={steer}
        onstop={stop}
        oncommand={onCommand}
      >
        {#snippet drawer()}
          {#if approvals.length}
            <ApprovalDrawer {approvals} {cwd} {responding} onrespond={respondApproval} />
          {:else if question}
            <QuestionCard input={question} customText={composerText} busy={answering} onsubmit={answerQuestion} ondismiss={dismissQuestion} />
          {:else if planReady && plans.proposed}
            <section class="plan-ready">
              <Icon name="list-checks" size={14} />
              <span class="pr-text">{$t("llm.central.threads.plan.ready")}</span>
              <span class="grow"></span>
              {#if composerText.trim()}
                <span class="muted">{$t("llm.central.threads.plan.refine_hint")}</span>
              {:else}
                <button type="button" class="button primary small" onclick={() => implementPlan(plans.proposed!)}>{$t("llm.central.threads.plan.implement")}</button>
                <button type="button" class="button small" onclick={() => implementPlanNew(plans.proposed!)}>{$t("llm.central.threads.plan.implement_new")}</button>
              {/if}
            </section>
          {/if}
        {/snippet}
        {#snippet meter()}
          <ContextMeter
            used={context?.used ?? null}
            max={context?.max ?? null}
            estimated={context?.estimated ?? false}
            {limits}
            cancompact={!running && (caps?.compaction ?? true)}
            oncompact={() => onCommand("compact")}
          />
        {/snippet}
      </ThreadComposer>
    {:else}
      <div class="hero">
        <button type="button" class="icon-btn only-narrow corner" aria-label={$t("llm.central.threads.sidebar")} onclick={() => (sidebarOpen = !sidebarOpen)}>
          <Icon name="sidebar-simple" size={16} />
        </button>
        <img class="art" src="/emoji/rocket.png" alt="" onerror={(e) => ((e.currentTarget as HTMLImageElement).style.display = "none")} />
        <h1>{$t("llm.central.threads.hero.title")}</h1>
        <p>{$t("llm.central.threads.hero.body")}</p>
        <div class="hero-actions">
          <button type="button" class="button primary" disabled={!threadsStore.projects.length} onclick={() => openNew(null)}>
            <Icon name="note-pencil" size={14} />{$t("llm.central.threads.new_thread")}
          </button>
          <button type="button" class="button" onclick={addProject}><Icon name="folder-simple" size={14} />{$t("llm.central.threads.new_project")}</button>
        </div>
        {#if threadsStore.error}<p class="err">{threadsStore.error}</p>{/if}
      </div>
    {/if}
  </main>

  {#if thread && panel?.open}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div class="resizer" role="separator" aria-orientation="vertical" aria-label={$t("llm.central.threads.panel.resize")} tabindex="0" onpointerdown={startResize} onkeydown={resizeKey}></div>
    <div class="panel-slot">
      {#key thread.threadId}
        <RightPanel
          tab={panel.tab}
          counts={{ tasks: taskList.filter((x) => x.status === "running").length, files: touched.length }}
          available={{ diff: cwd ? null : ($t("llm.central.threads.diff.no_folder") as string), terminal: cwd ? null : ($t("llm.central.threads.diff.no_folder") as string), pr: cwd ? null : ($t("llm.central.threads.diff.no_folder") as string) }}
          letters={!approvals.length && !question}
          onselect={(tab: PanelTab | null) => setPanel({ tab })}
          onclose={() => setPanel({ open: false })}
        >
          {#snippet diff()}
            <DiffPanel
              threadId={thread!.threadId}
              {cwd}
              turns={settledTurns}
              scope={panel!.diffScope}
              turn={panel!.diffTurn}
              reveal={panel!.reveal}
              fallback={turnFallback}
              onscope={(scope, turn) => setPanel({ diffScope: scope, diffTurn: turn })}
              oncomment={addReview}
              onrevert={(n) => {
                const row = rows.find((r): r is Extract<Row, { kind: "user" }> => r.kind === "user" && r.turn?.turn.ordinal === n + 1);
                if (row) {
                  revertError = null;
                  revertRow = row;
                }
              }}
            />
          {/snippet}
          {#snippet terminal()}
            <TerminalPanel storageKey={thread!.threadId} cwd={terminalCwd ?? cwd} onAddToChat={addTerminalSelection} onClosePanel={() => setPanel({ tab: null })} />
          {/snippet}
          {#snippet files()}
            <FilesPanel {cwd} {touched} onopenfile={openFile} />
          {/snippet}
          {#snippet plan()}
            <PlanPanel
              steps={plans.steps}
              proposed={plans.proposed}
              actionable={planReady}
              onimplement={implementPlan}
              onrefine={refinePlan}
              onimplementnew={implementPlanNew}
              onopenfile={openFile}
            />
          {/snippet}
          {#snippet tasks()}
            <TasksPanel tasks={taskList} />
          {/snippet}
          {#snippet pr()}
            <PrPanel threadId={thread!.threadId} {cwd} />
          {/snippet}
        </RightPanel>
      {/key}
    </div>
  {/if}

  <NewThreadDialog open={newOpen} projectId={newProject} onclose={() => (newOpen = false)} oncreated={onCreated} onnewproject={addProject} />
  <RevertDialog
    open={!!revertRow}
    turnCount={revertRow?.revertTo ?? 0}
    worktree={!!thread?.worktreePath}
    busy={revertBusy}
    error={revertError}
    onconfirm={confirmRevert}
    onclose={() => (revertRow = null)}
  />
  <UndoToast />
</div>

<style>
  .threads { flex: 1; min-height: 0; display: grid; grid-template-columns: 280px minmax(0, 1fr); position: relative; overflow: hidden; }
  .threads.panel-open { grid-template-columns: 280px minmax(0, 1fr) 6px var(--panel-w); }
  .side { min-height: 0; display: flex; }
  .main { min-width: 0; min-height: 0; display: flex; flex-direction: column; position: relative; }
  .thread-head { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-bottom: 1px solid var(--separator); min-height: 44px; box-sizing: border-box; flex-shrink: 0; }
  .crumb { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .proj { border: 0; background: transparent; color: var(--text-muted); font-size: 13px; cursor: pointer; padding: 3px 4px; border-radius: 6px; white-space: nowrap; }
  .proj:hover { background: var(--fill-1); color: var(--text); }
  .sep { color: var(--text-muted); }
  .title { margin: 0; font-size: 14px; font-weight: 650; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; cursor: text; }
  .title-input { font-size: 14px; font-weight: 600; border: 1px solid var(--accent); border-radius: 6px; padding: 2px 6px; background: var(--input-bg, var(--surface)); color: var(--text); min-width: 200px; }
  .harness { display: inline-flex; align-items: center; gap: 5px; font-size: 12px; color: var(--text-muted); white-space: nowrap; }
  .harness .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--tint); }
  .chip { display: inline-flex; align-items: center; gap: 4px; font-size: 11.5px; padding: 2px 7px; border-radius: 6px; background: var(--fill-1); color: var(--text-muted); font-family: var(--font-mono); max-width: 180px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .grow { flex: 1; }
  .icon-btn { width: 30px; height: 30px; display: inline-flex; align-items: center; justify-content: center; border-radius: 8px; border: 0; background: transparent; color: var(--text-muted); cursor: pointer; flex-shrink: 0; }
  .icon-btn:hover { background: var(--fill-2); color: var(--text); }
  .icon-btn[aria-pressed="true"] { color: var(--accent-hi); background: var(--accent-soft); }
  .icon-btn.small { width: 22px; height: 22px; }
  .only-narrow { display: none; }
  .banner-slot { position: relative; height: 0; z-index: 8; }
  .banner { position: absolute; left: 50%; transform: translateX(-50%); top: 8px; width: min(720px, calc(100% - 32px)); display: flex; align-items: flex-start; gap: 8px; padding: 8px 10px; border-radius: 12px; background: color-mix(in srgb, var(--red, #ef4444) 10%, var(--popup-bg, var(--surface))); color: var(--red, #ef4444); border: 1px solid color-mix(in srgb, var(--red, #ef4444) 30%, transparent); font-size: 12.5px; box-shadow: 0 6px 20px color-mix(in srgb, #000 12%, transparent); backdrop-filter: blur(10px); }
  .btext { flex: 1; min-width: 0; display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; overflow-wrap: anywhere; }
  .queued { display: flex; flex-direction: column; align-items: flex-end; gap: 4px; padding: 6px 0; }
  .q-bubble { max-width: 80%; border: 1.5px dashed color-mix(in srgb, var(--accent) 55%, var(--separator)); border-radius: 18px; padding: 8px 14px; color: var(--text-muted); }
  .q-bubble p { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; font-size: 14px; }
  .q-meta { font-size: 11px; }
  .q-actions { display: flex; align-items: center; gap: 4px; font-size: 11.5px; color: var(--text-muted); }
  .q-badge { display: inline-flex; align-items: center; gap: 4px; }
  .ghost { display: inline-flex; align-items: center; gap: 4px; border: 0; background: transparent; color: var(--text-muted); font-size: 11.5px; padding: 3px 6px; border-radius: 6px; cursor: pointer; }
  .ghost:hover { background: var(--fill-2); color: var(--text); }
  .plan-ready { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-radius: 16px 16px 0 0; border: 1px solid color-mix(in srgb, var(--purple, #8b5cf6) 40%, var(--separator)); border-bottom: 0; background: color-mix(in srgb, var(--purple, #8b5cf6) 6%, var(--surface, #fff)); color: var(--purple, #8b5cf6); font-size: 13px; }
  .pr-text { font-weight: 600; }
  .muted { color: var(--text-muted); font-size: 12px; }
  .button.small { padding: 4px 10px; font-size: 12px; }
  .resizer { cursor: col-resize; background: transparent; position: relative; }
  .resizer::after { content: ""; position: absolute; inset: 0 2px; border-radius: 2px; transition: background 120ms; }
  .resizer:hover::after, .resizer:focus-visible::after { background: var(--accent-soft); }
  .panel-slot { min-width: 0; min-height: 0; display: flex; flex-direction: column; }
  .panel-slot > :global(*) { flex: 1; }
  .hero { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; padding: 24px; text-align: center; position: relative; }
  .hero .corner { position: absolute; left: 12px; top: 12px; }
  .hero h1 { margin: 0; font-size: 22px; font-weight: 700; }
  .hero p { margin: 0; color: var(--text-muted); max-width: 460px; font-size: 14px; }
  .hero .art { width: 72px; height: 72px; }
  .hero-actions { display: flex; gap: 8px; margin-top: 8px; flex-wrap: wrap; justify-content: center; }
  .hero-actions .button { display: inline-flex; align-items: center; gap: 6px; }
  .err { color: var(--red, #ef4444); font-size: 12.5px; }
  @media (max-width: 1100px) {
    .threads.panel-open { grid-template-columns: 280px minmax(0, 1fr); }
    .threads.panel-open .resizer { display: none; }
    .panel-slot { position: absolute; right: 0; top: 0; bottom: 0; width: min(var(--panel-w), 92%); z-index: 20; box-shadow: -12px 0 32px color-mix(in srgb, #000 18%, transparent); background: var(--surface); }
  }
  @media (max-width: 820px) {
    .threads, .threads.panel-open { grid-template-columns: minmax(0, 1fr); }
    .side { position: absolute; left: 0; top: 0; bottom: 0; width: min(300px, 86%); z-index: 25; background: var(--surface); box-shadow: 12px 0 32px color-mix(in srgb, #000 18%, transparent); transform: translateX(-102%); transition: transform 180ms var(--ease-out, ease); }
    .side.open { transform: none; }
    .only-narrow { display: inline-flex; }
  }
</style>
