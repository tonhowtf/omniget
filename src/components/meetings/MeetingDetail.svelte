<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getAgents, loadRoster } from "$lib/stores/llm-store.svelte";
  import {
    agentPrompt,
    botRefresh,
    botStop,
    clock,
    deleteMeeting,
    isBusy,
    meetingMarkdown,
    postToTeams,
    resummarize,
    retranscribe,
    setActionDone,
    speakerHue,
    turns,
    updateMeeting,
    type MeetingDetail,
    type Progress,
  } from "$lib/meetings/api";

  let {
    detail,
    progress,
    onchange,
    ondeleted,
  }: { detail: MeetingDetail; progress: Progress | null; onchange: () => void; ondeleted: () => void } = $props();

  type Tab = "summary" | "transcript" | "notes";
  let tab = $state<Tab>("summary");
  let title = $state("");
  let notes = $state("");
  let editingTitle = $state(false);
  let working = $state(false);
  let query = $state("");
  let botInfo = $state<{ status: string; segments: number } | null>(null);
  let agentOpen = $state(false);
  let agentId = $state("");
  let agentInstruction = $state("");
  let lastId = "";

  let m = $derived(detail.meeting);
  let busy = $derived(isBusy(m.status) || detail.running);
  let agents = $derived(getAgents());
  let lines = $derived(
    turns(detail.segments).filter((s) => !query.trim() || s.text.toLowerCase().includes(query.trim().toLowerCase()) || (s.speaker ?? "").toLowerCase().includes(query.trim().toLowerCase())),
  );

  // Reset the local edit state when another meeting is shown.
  $effect(() => {
    if (m.id !== lastId) {
      lastId = m.id;
      title = m.title;
      notes = m.notes;
      editingTitle = false;
      botInfo = null;
      agentOpen = false;
      tab = m.summary ? "summary" : detail.segments.length ? "transcript" : "summary";
    }
  });

  // A live bot is polled while this meeting is on screen.
  let botTimer: ReturnType<typeof setInterval> | null = null;
  $effect(() => {
    if (botTimer) clearInterval(botTimer);
    botTimer = null;
    if (m.status === "bot_live") {
      const id = m.id;
      const poll = async () => {
        try {
          const r = await botRefresh(id);
          botInfo = { status: r.bot_status, segments: r.live_segments };
          if (r.meeting.status !== "bot_live") onchange();
        } catch (e) {
          botInfo = { status: String(e), segments: 0 };
        }
      };
      void poll();
      botTimer = setInterval(poll, 15000);
    }
  });
  onDestroy(() => {
    if (botTimer) clearInterval(botTimer);
  });

  async function run(fn: () => Promise<unknown>, ok?: string) {
    working = true;
    try {
      await fn();
      if (ok) showToast("success", ok);
      onchange();
    } catch (e) {
      showToast("error", String(e), 9000);
    } finally {
      working = false;
    }
  }

  async function saveTitle() {
    editingTitle = false;
    if (title.trim() && title.trim() !== m.title) await run(() => updateMeeting(m.id, { title: title.trim() }));
  }

  async function saveNotes() {
    if (notes !== m.notes) await run(() => updateMeeting(m.id, { notes }));
  }

  async function copyMarkdown() {
    try {
      await navigator.clipboard.writeText(await meetingMarkdown(m.id));
      showToast("success", $t("meetings.copied") as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function openFolder() {
    try {
      await invoke("open_path_default", { path: detail.folder });
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function remove() {
    if (!confirm($t("meetings.delete_confirm", { title: m.title }) as string)) return;
    try {
      await deleteMeeting(m.id);
      ondeleted();
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function openAgent() {
    agentOpen = !agentOpen;
    if (!agentOpen) return;
    if (!agentInstruction) agentInstruction = $t("meetings.agent_default_instruction") as string;
    await loadRoster();
    if (!agentId && agents.length) agentId = agents[0].id;
  }

  async function sendToAgent() {
    if (!agentId) return;
    await run(async () => {
      const md = await meetingMarkdown(m.id);
      const job = await invoke<{ id: string }>("llm_job_submit", { agentId, prompt: agentPrompt(md, agentInstruction) });
      agentOpen = false;
      showToast("success", $t("meetings.agent_started", { id: job.id.slice(0, 8) }) as string);
    });
  }

  function progressLabel(p: Progress): string {
    if (p.stage === "downloading") return $t("meetings.progress_downloading", { time: clock(p.done * 1000) }) as string;
    if (p.stage === "transcribing" && p.total) return $t("meetings.progress_transcribing", { pct: p.done }) as string;
    return $t(`meetings.status.${m.status}`) as string;
  }

  function sourceLabel(): string {
    const s = m.transcript_source;
    return s ? ($t(`meetings.transcript_source.${s}`) as string) : "";
  }
</script>

<article class="meeting">
  <header class="m-head">
    {#if editingTitle}
      <input
        class="input title-input"
        bind:value={title}
        onblur={saveTitle}
        onkeydown={(e) => {
          if (e.key === "Enter") saveTitle();
          if (e.key === "Escape") {
            title = m.title;
            editingTitle = false;
          }
        }}
        aria-label={$t("meetings.rename") as string}
      />
    {:else}
      <h2>
        <button class="title-btn" onclick={() => (editingTitle = true)} title={$t("meetings.rename") as string}>{m.title}</button>
      </h2>
    {/if}
    <p class="m-meta">
      {new Date(m.created_at_ms).toLocaleString()}
      {#if m.duration_ms}· {clock(m.duration_ms)}{/if}
      {#if m.speakers.length}· {m.speakers.join(", ")}{/if}
      {#if m.language}· {m.language}{/if}
      {#if sourceLabel()}· {sourceLabel()}{/if}
    </p>
  </header>

  {#if busy || m.status === "recording"}
    <div class="state info" role="status">
      <span class="spinner" aria-hidden="true"></span>
      {progress ? progressLabel(progress) : $t(`meetings.status.${m.status}`)}
      {#if progress?.stage === "transcribing" && progress.total}
        <span class="bar"><span style:width="{progress.done}%"></span></span>
      {/if}
    </div>
  {:else if m.status === "bot_live"}
    <div class="state info" role="status">
      <span class="spinner" aria-hidden="true"></span>
      {$t("meetings.bot_live", { status: botInfo?.status || "…", lines: botInfo?.segments ?? 0 })}
      <button class="btn btn-sm btn-secondary" onclick={() => run(() => botStop(m.id))} disabled={working}>{$t("meetings.bot_stop")}</button>
    </div>
  {/if}
  {#if m.error}
    <div class="state error" role="alert">
      <span>{m.error}</span>
    </div>
  {/if}

  <div class="actions">
    {#if m.status === "failed"}
      <button class="btn btn-sm btn-primary" onclick={() => run(() => retranscribe(m.id))} disabled={working || busy}>{$t("meetings.retry")}</button>
    {/if}
    {#if detail.segments.length}
      <button class="btn btn-sm btn-secondary" onclick={() => run(() => resummarize(m.id), $t("meetings.summary_done") as string)} disabled={working || busy}>
        {m.summary ? $t("meetings.resummarize") : $t("meetings.summarize")}
      </button>
    {/if}
    {#if m.status === "ready" && (m.audio || m.source.kind === "capture")}
      <button class="btn btn-sm btn-ghost" onclick={() => run(() => retranscribe(m.id))} disabled={working || busy}>{$t("meetings.retranscribe")}</button>
    {/if}
    {#if m.summary}
      <button class="btn btn-sm btn-secondary" onclick={() => run(() => postToTeams(m.id), $t("meetings.posted") as string)} disabled={working}>{$t("meetings.post_teams")}</button>
      <button class="btn btn-sm btn-secondary" onclick={openAgent} aria-expanded={agentOpen}>{$t("meetings.send_agent")}</button>
    {/if}
    {#if detail.segments.length}
      <button class="btn btn-sm btn-ghost" onclick={copyMarkdown}>{$t("meetings.copy_markdown")}</button>
    {/if}
    <button class="btn btn-sm btn-ghost" onclick={openFolder}>{$t("meetings.open_folder")}</button>
    <button class="btn btn-sm btn-ghost danger" onclick={remove} disabled={busy}>{$t("meetings.delete")}</button>
  </div>

  {#if agentOpen}
    <div class="agent">
      {#if agents.length === 0}
        <p class="hint">{$t("meetings.no_agents")}</p>
      {:else}
        <select class="input" bind:value={agentId} aria-label={$t("meetings.agent") as string}>
          {#each agents as a (a.id)}
            <option value={a.id}>{a.name}</option>
          {/each}
        </select>
        <textarea class="input" rows="3" bind:value={agentInstruction} aria-label={$t("meetings.agent_instruction") as string}></textarea>
        <p class="hint">{$t("meetings.agent_hint")}</p>
        <div class="agent-actions">
          <button class="btn btn-sm btn-primary" onclick={sendToAgent} disabled={working || !agentId || !agentInstruction.trim()}>{$t("meetings.agent_run")}</button>
        </div>
      {/if}
    </div>
  {/if}

  <div class="tabs" role="tablist">
    <button role="tab" aria-selected={tab === "summary"} class:on={tab === "summary"} onclick={() => (tab = "summary")}>{$t("meetings.tab_summary")}</button>
    <button role="tab" aria-selected={tab === "transcript"} class:on={tab === "transcript"} onclick={() => (tab = "transcript")}>
      {$t("meetings.tab_transcript")}{#if detail.segments.length}<span class="count">{detail.segments.length}</span>{/if}
    </button>
    <button role="tab" aria-selected={tab === "notes"} class:on={tab === "notes"} onclick={() => (tab = "notes")}>{$t("meetings.tab_notes")}</button>
  </div>

  {#if tab === "summary"}
    {#if m.summary}
      {@const s = m.summary}
      <section class="summary">
        {#if s.overview}<p class="overview">{s.overview}</p>{/if}
        {#if s.action_items.length}
          <h3>{$t("meetings.action_items")}</h3>
          <ul class="todos">
            {#each s.action_items as a, i (i)}
              <li>
                <label>
                  <input type="checkbox" checked={a.done} onchange={(e) => run(() => setActionDone(m.id, i, (e.currentTarget as HTMLInputElement).checked))} />
                  <span class:done={a.done}>{a.task}</span>
                  {#if a.owner}<span class="chip">@{a.owner}</span>{/if}
                  {#if a.due}<span class="chip due">{a.due}</span>{/if}
                </label>
              </li>
            {/each}
          </ul>
        {/if}
        {#if s.decisions.length}
          <h3>{$t("meetings.decisions")}</h3>
          <ul>{#each s.decisions as d, i (i)}<li>{d}</li>{/each}</ul>
        {/if}
        {#if s.key_points.length}
          <h3>{$t("meetings.key_points")}</h3>
          <ul>{#each s.key_points as d, i (i)}<li>{d}</li>{/each}</ul>
        {/if}
        {#if s.open_questions.length}
          <h3>{$t("meetings.open_questions")}</h3>
          <ul>{#each s.open_questions as d, i (i)}<li>{d}</li>{/each}</ul>
        {/if}
        {#if s.model}<p class="hint">{$t("meetings.summary_by", { model: s.model })}</p>{/if}
      </section>
    {:else}
      <p class="hint pad">{busy ? $t("meetings.summary_pending") : $t("meetings.no_summary")}</p>
    {/if}
  {:else if tab === "transcript"}
    {#if detail.segments.length}
      <input class="input input-search" bind:value={query} placeholder={$t("meetings.search_transcript") as string} />
      <ol class="transcript">
        {#each lines as s, i (i)}
          <li>
            <span class="ts">{clock(s.start_ms)}</span>
            <div>
              {#if s.speaker}<span class="speaker" style:--hue={speakerHue(s.speaker)}>{s.speaker}</span>{/if}
              <p>{s.text}</p>
            </div>
          </li>
        {/each}
      </ol>
    {:else}
      <p class="hint pad">{busy || m.status === "bot_live" ? $t("meetings.transcript_pending") : $t("meetings.no_transcript")}</p>
    {/if}
  {:else}
    <textarea class="input notes" rows="12" bind:value={notes} onblur={saveNotes} placeholder={$t("meetings.notes_placeholder") as string}></textarea>
    <p class="hint">{$t("meetings.notes_hint")}</p>
  {/if}
</article>

<style>
  .meeting {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding-bottom: var(--space-6);
  }
  .m-head h2 {
    margin: 0;
    font-size: var(--text-lg);
  }
  .title-btn {
    all: unset;
    cursor: text;
  }
  .title-btn:focus-visible {
    outline: 2px solid var(--accent);
    border-radius: 4px;
  }
  .title-input {
    font-size: var(--text-lg);
    width: 100%;
  }
  .m-meta {
    margin: 4px 0 0;
    color: var(--text-muted);
    font-size: var(--text-xs);
  }
  .state {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
    flex-wrap: wrap;
  }
  .state.info {
    background: var(--accent-soft);
  }
  .state.error {
    background: color-mix(in srgb, var(--danger) 12%, transparent);
    color: var(--text);
    white-space: pre-wrap;
  }
  .spinner {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid var(--accent);
    border-right-color: transparent;
    animation: spin 0.8s linear infinite;
  }
  .bar {
    flex: 1;
    min-width: 80px;
    height: 4px;
    border-radius: 2px;
    background: var(--fill-2);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .danger {
    color: var(--danger);
  }
  .agent {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }
  .agent-actions {
    display: flex;
    justify-content: flex-end;
  }
  .tabs {
    display: flex;
    gap: var(--space-1);
    border-bottom: 1px solid var(--separator);
  }
  .tabs button {
    border: 0;
    background: transparent;
    color: var(--text-muted);
    padding: var(--space-2) var(--space-3);
    cursor: pointer;
    border-bottom: 2px solid transparent;
    font-size: var(--text-sm);
  }
  .tabs button.on {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .count {
    margin-left: 6px;
    font-size: var(--text-xs);
    color: var(--text-faint);
  }
  .summary h3 {
    font-size: var(--text-sm);
    margin: var(--space-3) 0 var(--space-1);
  }
  .summary ul {
    margin: 0;
    padding-left: 1.2em;
    font-size: var(--text-sm);
    line-height: 1.5;
  }
  .overview {
    font-size: var(--text-md);
    line-height: 1.5;
    margin: 0;
  }
  .todos {
    list-style: none;
    padding-left: 0 !important;
  }
  .todos label {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .done {
    text-decoration: line-through;
    color: var(--text-muted);
  }
  .chip {
    font-size: var(--text-xs);
    padding: 1px 6px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
  }
  .chip.due {
    background: color-mix(in srgb, var(--warning) 20%, transparent);
  }
  .transcript {
    list-style: none;
    margin: var(--space-2) 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .transcript li {
    display: grid;
    grid-template-columns: 56px 1fr;
    gap: var(--space-2);
  }
  .transcript p {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    line-height: 1.5;
  }
  .ts {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-faint);
    padding-top: 2px;
  }
  .speaker {
    font-size: var(--text-xs);
    font-weight: 600;
    color: hsl(var(--hue) 55% 45%);
  }
  :global([data-theme*="dark"]) .speaker {
    color: hsl(var(--hue) 70% 70%);
  }
  .notes {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    font-family: var(--font-body);
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .pad {
    padding: var(--space-3) 0;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
  }
</style>
