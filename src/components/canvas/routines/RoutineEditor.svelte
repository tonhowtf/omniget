<script lang="ts">
  // Routines of a workspace: a prompt typed into a terminal on a schedule
  // (once, every N minutes, daily, weekly). `&&` alone on a line splits it
  // into steps sent one after the other; a pre-run script's output fills
  // {{output}}; busy terminals are skipped. With no terminal it is a reminder.
  import { onDestroy, onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { errorText } from "$lib/canvas/floors/api";
  import {
    EVENT_ROUTINE,
    clearRoutineHistory,
    dayLabel,
    intervalSecs,
    projectRuns,
    routineHistory,
    deleteRoutine,
    describeSchedule,
    listRoutines,
    runRoutineNow,
    saveRoutine,
    setRoutineEnabled,
    splitSteps,
    type End,
    type HistoryItem,
    type Routine,
    type Schedule,
  } from "$lib/canvas/routines/api";

  interface Props {
    workspaceId: string;
    floorId: string;
    /** Terminal nodes of the current floor that can be targets. */
    nodes: Array<{ id: string; title: string }>;
    /** Open the editor for a new routine aimed at this node. */
    targetNodeId?: string;
    onClose: () => void;
  }
  let { workspaceId, floorId, nodes, targetNodeId, onClose }: Props = $props();

  let routines = $state<Routine[]>([]);
  let editing = $state<Routine | null>(null);
  let creating = $state(false);
  let error = $state("");
  let busy = $state(false);

  // form
  let name = $state("");
  let nodeId = $state("");
  let prompt = $state("");
  let kind = $state<Schedule["kind"]>("daily");
  let at = $state("");
  let every = $state(60);
  let unit = $state<"s" | "m" | "h">("m");
  let showHistory = $state(false);
  let history = $state<HistoryItem[]>([]);
  let time = $state("09:00");
  let days = $state<number[]>([1, 2, 3, 4, 5]);
  let endKind = $state<End["kind"]>("never");
  let endCount = $state(10);
  let endDate = $state("");
  let preRun = $state("");
  let skipIfBusy = $state(true);
  let notify = $state(false);

  const tr = (k: string, v?: Record<string, unknown>) => $t(k, v) as string;
  // The literal "{{output}}" shown in the label. Passed escaped: the i18n
  // parser re-parses its own output, so a raw "{{output}}" value recursed
  // forever and froze the window when the form opened.
  const OUTPUT_TOKEN = "\\{\\{output\\}\\}";
  const steps = $derived(splitSteps(prompt).length);

  async function refresh() {
    try {
      routines = await listRoutines(workspaceId);
      if (showHistory) history = await routineHistory(workspaceId);
    } catch (e) {
      error = errorText(e);
    }
  }

  const UNIT_SECS = { s: 1, m: 60, h: 3600 } as const;
  const MIN_SECS = 10;
  const everySecs = $derived(Math.round(every * UNIT_SECS[unit]));

  function setEvery(secs: number) {
    if (secs % 3600 === 0) {
      unit = "h";
      every = secs / 3600;
    } else if (secs % 60 === 0) {
      unit = "m";
      every = secs / 60;
    } else {
      unit = "s";
      every = secs;
    }
  }

  function codeLabel(h: { status: string; code?: string }): string {
    const key = h.code ?? h.status;
    return tr(`canvas.routines.code.${key}`, { default: tr(`canvas.routines.status.${h.status}`, { default: h.status }) });
  }

  function upcoming(r: Routine): string {
    if (!r.enabled || !r.nextRunAt) return "";
    const first = new Date(r.nextRunAt);
    const base = first < new Date() ? new Date() : first;
    return projectRuns(r.schedule, base, 3)
      .map((d) => d.toLocaleString([], { weekday: "short", hour: "2-digit", minute: "2-digit" }))
      .join(" · ");
  }

  async function toggleHistory() {
    showHistory = !showHistory;
    if (showHistory) await refresh();
  }

  let unlisten: UnlistenFn | null = null;
  onMount(async () => {
    unlisten = await listen<Routine | { id: string; deleted: true }>(EVENT_ROUTINE, () => refresh());
    if (targetNodeId) startNew(targetNodeId);
  });
  onDestroy(() => unlisten?.());
  $effect(() => {
    void workspaceId;
    refresh();
  });

  function localInput(iso?: string): string {
    const d = iso ? new Date(iso) : new Date(Date.now() + 3600_000);
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
  }

  function startNew(target?: string) {
    editing = null;
    creating = true;
    name = "";
    nodeId = target ?? nodes[0]?.id ?? "";
    prompt = "";
    kind = "daily";
    at = localInput();
    every = 60;
    unit = "m";
    time = "09:00";
    days = [1, 2, 3, 4, 5];
    endKind = "never";
    preRun = "";
    skipIfBusy = true;
    notify = false;
  }

  function edit(r: Routine) {
    editing = r;
    creating = true;
    name = r.name;
    nodeId = r.nodeId ?? "";
    prompt = r.prompt;
    kind = r.schedule.kind;
    at = localInput(r.schedule.kind === "once" ? r.schedule.at : undefined);
    if (r.schedule.kind === "interval") setEvery(intervalSecs(r.schedule));
    else {
      every = 60;
      unit = "m";
    }
    time = r.schedule.kind === "daily" || r.schedule.kind === "weekly" ? r.schedule.time : "09:00";
    days = r.schedule.kind === "weekly" ? [...r.schedule.days] : [1, 2, 3, 4, 5];
    endKind = r.end.kind;
    endCount = r.end.kind === "afterCount" ? r.end.count : 10;
    endDate = r.end.kind === "onDate" ? localInput(r.end.date) : "";
    preRun = r.preRun ?? "";
    skipIfBusy = r.skipIfBusy;
    notify = r.notify;
  }

  function schedule(): Schedule {
    switch (kind) {
      case "once":
        return { kind, at: new Date(at).toISOString() };
      case "interval": {
        const secs = Math.max(MIN_SECS, everySecs);
        return { kind, minutes: Math.floor(secs / 60), seconds: secs };
      }
      case "daily":
        return { kind, time };
      case "weekly":
        return { kind, days: [...days].sort(), time };
    }
  }

  function end(): End {
    if (endKind === "afterCount") return { kind: "afterCount", count: Math.max(1, Math.round(endCount)) };
    if (endKind === "onDate" && endDate) return { kind: "onDate", date: new Date(endDate).toISOString() };
    return { kind: "never" };
  }

  async function save() {
    busy = true;
    error = "";
    try {
      await saveRoutine({
        id: editing?.id,
        name: name.trim() || prompt.trim().split("\n")[0].slice(0, 40) || "Routine",
        workspaceId,
        floorId: editing?.floorId ?? floorId,
        nodeId: nodeId || null,
        prompt,
        schedule: schedule(),
        end: end(),
        preRun: preRun.trim() || null,
        skipIfBusy,
        notify,
        enabled: editing?.enabled ?? true,
      });
      creating = false;
      editing = null;
      refresh();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function act(f: () => Promise<unknown>) {
    error = "";
    try {
      await f();
      refresh();
    } catch (e) {
      error = errorText(e);
    }
  }

  function targetName(r: Routine): string {
    if (!r.nodeId) return tr("canvas.routines.reminder", { default: "Reminder" });
    return nodes.find((n) => n.id === r.nodeId)?.title ?? tr("canvas.routines.otherFloor", { default: "terminal on another floor" });
  }
</script>

<div class="panel" data-canvas-chrome role="dialog" aria-label={$t("canvas.routines.title", { default: "Routines" })}>
  <header>
    <h3>{$t("canvas.routines.title", { default: "Routines" })}</h3>
    <div class="row">
      {#if !creating}
        <button type="button" class:on={showHistory} onclick={toggleHistory}>{$t("canvas.routines.history", { default: "History" })}</button>
        <button type="button" class="primary" onclick={() => startNew()}>+ {$t("canvas.routines.new", { default: "New routine" })}</button>
      {/if}
      <button type="button" class="icon" onclick={onClose} aria-label={$t("canvas.routines.close", { default: "Close" })}>✕</button>
    </div>
  </header>

  {#if creating}
    <div class="form">
      <label><span>{$t("canvas.routines.name", { default: "Name" })}</span><input bind:value={name} /></label>
      <label>
        <span>{$t("canvas.routines.target", { default: "Terminal" })}</span>
        <select bind:value={nodeId}>
          <option value="">{$t("canvas.routines.noTarget", { default: "None (reminder only)" })}</option>
          {#each nodes as n (n.id)}<option value={n.id}>{n.title}</option>{/each}
        </select>
      </label>
      <label>
        <span>{nodeId ? $t("canvas.routines.prompt", { default: "Prompt" }) : $t("canvas.routines.message", { default: "Message" })}</span>
        <textarea rows="5" bind:value={prompt} placeholder={$t("canvas.routines.promptHint", { default: "Run the tests and report failures\n&&\nFix the first failure" })}></textarea>
        {#if steps > 1}<em class="muted">{$t("canvas.routines.steps", { default: "{{count}} steps, sent one after the other", count: steps })}</em>{/if}
      </label>
      <label>
        <span>{$t("canvas.routines.when", { default: "When" })}</span>
        <select bind:value={kind}>
          <option value="once">{$t("canvas.routines.kind.once", { default: "Once" })}</option>
          <option value="interval">{$t("canvas.routines.kind.interval", { default: "Every…" })}</option>
          <option value="daily">{$t("canvas.routines.kind.daily", { default: "Every day" })}</option>
          <option value="weekly">{$t("canvas.routines.kind.weekly", { default: "Every week" })}</option>
        </select>
      </label>
      {#if kind === "once"}
        <input type="datetime-local" bind:value={at} />
      {:else if kind === "interval"}
        <div class="row start">
          <input type="number" min="1" bind:value={every} />
          <select bind:value={unit}>
            <option value="s">{$t("canvas.routines.unit.seconds", { default: "seconds" })}</option>
            <option value="m">{$t("canvas.routines.minutes", { default: "minutes" })}</option>
            <option value="h">{$t("canvas.routines.unit.hours", { default: "hours" })}</option>
          </select>
        </div>
        {#if everySecs < MIN_SECS}<em class="error">{$t("canvas.routines.minInterval", { default: "The shortest interval is {{count}} seconds.", count: MIN_SECS })}</em>{/if}
      {:else}
        {#if kind === "weekly"}
          <div class="days">
            {#each [0, 1, 2, 3, 4, 5, 6] as d (d)}
              <button type="button" class:on={days.includes(d)} onclick={() => (days = days.includes(d) ? days.filter((x) => x !== d) : [...days, d])}>{dayLabel(tr, d)}</button>
            {/each}
          </div>
        {/if}
        <input type="time" bind:value={time} />
      {/if}
      {#if kind !== "once"}
        <label>
          <span>{$t("canvas.routines.ends", { default: "Ends" })}</span>
          <select bind:value={endKind}>
            <option value="never">{$t("canvas.routines.end.never", { default: "Never" })}</option>
            <option value="afterCount">{$t("canvas.routines.end.afterCount", { default: "After a number of runs" })}</option>
            <option value="onDate">{$t("canvas.routines.end.onDate", { default: "On a date" })}</option>
          </select>
        </label>
        {#if endKind === "afterCount"}<input type="number" min="1" bind:value={endCount} />{/if}
        {#if endKind === "onDate"}<input type="datetime-local" bind:value={endDate} />{/if}
      {/if}
      <details>
        <summary>{$t("canvas.routines.advanced", { default: "Pre-run script and options" })}</summary>
        <label>
          <span>{$t("canvas.routines.preRun", { default: "Pre-run script (must exit 0; its output fills {{output}})", output: OUTPUT_TOKEN })}</span>
          <input class="mono" bind:value={preRun} placeholder="git log --oneline -5" />
        </label>
        <label class="check"><input type="checkbox" bind:checked={skipIfBusy} />{$t("canvas.routines.skipIfBusy", { default: "Skip if the terminal is busy" })}</label>
        <label class="check"><input type="checkbox" bind:checked={notify} />{$t("canvas.routines.notify", { default: "Notify me when it runs" })}</label>
      </details>
      <div class="row">
        <button type="button" onclick={() => { creating = false; editing = null; }}>{$t("canvas.routines.cancel", { default: "Cancel" })}</button>
        <button type="button" class="primary" disabled={busy || !prompt.trim() || (kind === "weekly" && !days.length) || (kind === "interval" && everySecs < MIN_SECS)} onclick={save}>{$t("canvas.routines.save", { default: "Save" })}</button>
      </div>
    </div>
  {/if}
  {#if error}<p class="error">{error}</p>{/if}

  {#if showHistory && !creating}
    <div class="histall">
      <div class="row">
        <strong>{$t("canvas.routines.historyAll", { default: "Every run in this workspace" })}</strong>
        <button type="button" disabled={!history.length} onclick={() => act(() => clearRoutineHistory(workspaceId))}>{$t("canvas.routines.clearHistory", { default: "Clear history" })}</button>
      </div>
      <ul class="hist">
        {#each history.slice(0, 50) as h, i (i)}
          <li class={h.status}>
            {new Date(h.at).toLocaleString()} · <b>{h.routineName}</b>{h.removed ? ` (${tr("canvas.routines.removed", { default: "finished" })})` : ""} · {codeLabel(h)}{h.lateSecs ? ` · ${tr("canvas.routines.late", { default: "{{count}} s late", count: h.lateSecs })}` : ""}{h.message ? ` · ${h.message}` : ""}
          </li>
        {:else}
          <li class="muted">{$t("canvas.routines.historyEmpty", { default: "Nothing ran yet." })}</li>
        {/each}
      </ul>
    </div>
  {/if}

  <ul class="list">
    {#each routines as r (r.id)}
      <li class:off={!r.enabled}>
        <div class="head">
          <label class="check" title={r.enabled ? $t("canvas.routines.disable", { default: "Pause" }) : $t("canvas.routines.enable", { default: "Resume" })}>
            <input type="checkbox" checked={r.enabled} onchange={() => act(() => setRoutineEnabled(r.id, !r.enabled))} />
          </label>
          <button type="button" class="name" onclick={() => edit(r)}>{r.name}</button>
          <button type="button" class="icon" title={$t("canvas.routines.runNow", { default: "Run now" })} onclick={() => act(() => runRoutineNow(r.id))}>▶</button>
          <button type="button" class="icon" title={$t("canvas.routines.delete", { default: "Delete" })} onclick={() => act(() => deleteRoutine(r.id))}>✕</button>
        </div>
        <div class="muted">
          {describeSchedule(tr, r.schedule)} → {targetName(r)}
          {#if r.nextRunAt}· {$t("canvas.routines.next", { default: "next {{when}}", when: new Date(r.nextRunAt).toLocaleString() })}{/if}
          {#if r.createdBy}· {$t("canvas.routines.byAgent", { default: "made by an agent" })}{/if}
        </div>
        {#if upcoming(r)}<div class="muted upcoming">{$t("canvas.routines.upcoming", { default: "Upcoming: {{when}}", when: upcoming(r) })}</div>{/if}
        {#if r.history.length}
          <details>
            <summary class="muted">{$t("canvas.routines.history", { default: "History" })} ({r.runCount})</summary>
            <ul class="hist">
              {#each r.history.slice(0, 10) as h, i (i)}
                <li class={h.status}>{new Date(h.at).toLocaleString()} · {codeLabel(h)}{h.lateSecs ? ` · ${tr("canvas.routines.late", { default: "{{count}} s late", count: h.lateSecs })}` : ""}{h.message ? ` · ${h.message}` : ""}</li>
              {/each}
            </ul>
          </details>
        {/if}
      </li>
    {:else}
      {#if !creating}<li class="muted">{$t("canvas.routines.empty", { default: "No routines in this workspace." })}</li>{/if}
    {/each}
  </ul>
</div>

<style>
  .panel {
    width: min(420px, calc(100vw - 32px));
    max-height: 75vh;
    overflow: auto;
    background: var(--popup-bg, var(--surface));
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: 0 12px 36px rgba(0, 0, 0, 0.3);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 13px;
  }
  header,
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .row.start {
    justify-content: flex-start;
  }
  h3 {
    margin: 0;
    font-size: 15px;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  label > span {
    color: var(--text-muted);
    font-size: 12px;
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  input:not([type="checkbox"]),
  select,
  textarea {
    font: inherit;
    color: var(--text);
    background: var(--input-bg, transparent);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 5px 8px;
    min-width: 0;
  }
  textarea {
    resize: vertical;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
  }
  .mono {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
  }
  .histall {
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .upcoming {
    font-size: 11px;
  }
  .days {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .days .on {
    border-color: var(--accent);
    color: var(--accent);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .list > li {
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .list > li.off {
    opacity: 0.6;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .name {
    flex: 1;
    text-align: left;
    border: none;
    font-weight: 600;
    padding: 2px 4px;
  }
  .hist {
    margin: 4px 0 0;
    padding-left: 16px;
    font-size: 12px;
  }
  .hist .failed {
    color: var(--danger);
  }
  .hist .skipped {
    color: var(--text-muted);
  }
  .muted {
    color: var(--text-muted);
    font-size: 12px;
  }
  .error {
    color: var(--danger);
    margin: 0;
  }
  button {
    font: inherit;
    color: inherit;
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 4px 10px;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .icon {
    border: none;
    padding: 2px 6px;
  }
  .primary {
    background: var(--accent);
    color: var(--on-accent, #fff);
    border-color: var(--accent);
  }
</style>
