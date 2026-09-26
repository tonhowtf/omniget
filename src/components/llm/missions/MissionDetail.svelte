<script lang="ts">
  /**
   * One mission: where it stands in plain words, what "done" means and which
   * criteria have evidence, the tasks, the budget (known spend plus calls of
   * unknown cost, never shown as free), a box to correct the context, and
   * what you can do next. A blocked mission shows the diagnosis summary and
   * its suggested actions first. Infrastructure stays collapsed.
   */
  import { tick } from "svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { getAgents } from "$lib/stores/llm-store.svelte";
  import { answerAsk, getAsks, getJobs, pollAsks } from "$lib/stores/llm-jobs-store.svelte";
  import {
    acceptCriterion, actionPlan, budgetLimits, budgetRatio, cancelMission, completionKey, deleteMission,
    errorText, getLastMissionEvent, getMission, getMissionTick, isFinal, missionActions, missionDiagnostics,
    missionStateKey, missionTone, noteMission, pauseMission, resumeMission, reviseCriteria, runActionCommand,
    shortTime, specSummary, spendLine, startMission, taskStateKey, taskTone, unblockTask, verdictRows,
    verdictSummary, verifyMission, criterionStatusKey, criterionTitleLine, taskTitleLine, purposeKey,
    effectStateKey, originKey, missionAsks, isAbsolutePath, shortPath, budgetBlockKind, budgetRevision, budgetErrorKey,
    reviseBudget, minutesUsed, taskEffectUnknown, isExternalMission, externalClientName, mcpClientNames, effectKindKey,
    authOriginKey,
    type Criterion, type CriterionStatus, type MissionAction, type MissionAsk, type MissionDetail, type SuggestedAction,
  } from "$lib/stores/assist-missions-store.svelte";
  import CriteriaEditor from "./CriteriaEditor.svelte";
  import TaskLog from "./TaskLog.svelte";

  // Receipt statuses that have a translated label (mission.status.*); anything else is shown as sent.
  const RECEIPT_STATUSES = ["pass", "fail", "unknown", "missing", "stale", "awaiting_human"];

  let { missionId, ondeleted }: { missionId: string; ondeleted?: () => void } = $props();

  let detail = $state<MissionDetail | null>(null);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state("");
  let notice = $state<string | null>(null);
  let note = $state("");
  let decisionNote = $state("");
  let editing = $state(false);
  let draftCriteria = $state<Criterion[]>([]);
  let reviseReason = $state("");
  let openLog = $state<string | null>(null);
  let confirming = $state<{ label: string; run: () => Promise<unknown>; yes: string; no: string } | null>(null);
  let exportPath = $state("");
  let answering = $state<string | null>(null);
  let noteBox = $state<HTMLTextAreaElement | null>(null);
  // Inline "raise the limits" form (U1): blank fields keep their value.
  let budgetOpen = $state(false);
  let bfTokens = $state("");
  let bfMinutes = $state("");
  let bfError = $state("");
  let clientNames = $state<Record<string, string>>({});

  let agents = $derived(getAgents());
  let m = $derived(detail?.mission ?? null);
  let rows = $derived(detail ? verdictRows(detail.verdict, detail.criteria) : []);
  let summary = $derived(verdictSummary(detail?.verdict ?? null));
  let actions = $derived(m ? missionActions(m.state) : []);
  let spend = $derived(m ? spendLine(m.spent) : null);
  let ratio = $derived(m ? budgetRatio(m.spent, m.budget) : null);
  let resumable = $derived(!!m && (m.state === "paused" || m.state === "blocked" || m.state === "partial"));
  let activeReceipts = $derived(detail?.receipts.filter((r) => !r.invalidated_ms) ?? []);
  let staleReceipts = $derived(detail?.receipts.filter((r) => r.invalidated_ms) ?? []);
  // Tool approvals this mission's turns wait for (the chat does not drive them).
  let limitKind = $derived(m ? budgetBlockKind(m.block) : null);
  let external = $derived(!!m && isExternalMission(m));
  let clientName = $derived(m && external ? externalClientName(m, clientNames) : null);
  let approvals = $derived(
    detail ? missionAsks(getAsks(), getJobs() ?? [], detail.mission, detail.tasks.flatMap((t) => t.job_ids)) : [],
  );

  async function answer(a: MissionAsk, allow: boolean, always: boolean) {
    if (answering) return;
    answering = a.tool_call_id;
    try {
      await answerAsk({ ...a, preview: a.preview ?? "" }, allow, always);
    } finally {
      answering = null;
      void pollAsks();
    }
  }

  function titleOf(c: { id: string; title: string; origin: Criterion["origin"] | null }): string {
    const spec = detail?.criteria.find((d) => d.id === c.id)?.spec;
    const line = c.origin ? criterionTitleLine({ id: c.id, title: c.title, origin: c.origin, spec }) : null;
    return line ? $t(line.key, line.params) : c.title;
  }

  function taskTitle(title: string): string {
    const line = taskTitleLine(title);
    return line ? $t(line.key, line.params) : title;
  }

  async function load(id: string) {
    try {
      const d = await getMission(id);
      if (id !== missionId) return;
      detail = d;
      error = "";
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    const id = missionId;
    loading = true;
    detail = null;
    void load(id);
  });

  $effect(() => {
    if (external) void mcpClientNames().then((n) => (clientNames = n));
  });

  // Live: an event for this mission refreshes it quietly.
  $effect(() => {
    const n = getMissionTick();
    const ev = getLastMissionEvent();
    if (n > 0 && ev?.mission_id === missionId && !busy) void load(missionId);
  });

  function agentName(id: string | null | undefined): string {
    if (!id) return "—";
    return agents.find((a) => a.id === id)?.name ?? id;
  }

  async function act(fn: () => Promise<unknown>, done?: string) {
    if (busy) return;
    busy = true;
    error = "";
    notice = null;
    confirming = null;
    try {
      await fn();
      if (done) notice = done;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
    await load(missionId);
  }

  function ask(label: string, run: () => Promise<unknown>, yes = $t("mission.confirm.yes"), no = $t("mission.confirm.back")) {
    confirming = { label, run, yes, no };
  }

  function runAction(a: MissionAction) {
    if (!m) return;
    const id = m.id;
    switch (a) {
      case "start": return void act(() => startMission(id));
      case "pause": return void act(() => pauseMission(id));
      case "resume": return void act(() => resumeMission(id, note).then(() => (note = "")));
      // The check runs in the background; its result arrives as a mission event.
      case "verify": return void act(async () => {
        const r = await verifyMission(id);
        notice = $t(r.already_running ? "mission.detail.verify_already_running" : "mission.detail.verify_started");
      });
      case "cancel": return ask($t("mission.confirm.cancel"), () => cancelMission(id), $t("mission.confirm.cancel_yes"), $t("mission.confirm.keep"));
      case "delete":
        return ask($t("mission.confirm.delete"), async () => {
          await deleteMission(id);
          ondeleted?.();
        }, $t("mission.confirm.delete_yes"), $t("mission.confirm.keep"));
    }
  }

  async function suggested(a: SuggestedAction) {
    if (!detail) return;
    const plan = actionPlan(a, detail.mission.id, detail.tasks);
    const go = async () => {
      if (plan.kind === "command") await runActionCommand(plan);
      else if (plan.kind === "route") await goto(plan.route);
      else if (plan.kind === "local") {
        if (plan.local === "edit_criteria") startEditing();
        else if (plan.local === "edit_context") { await tick(); noteBox?.focus(); }
        else openBudget();
      }
    };
    // The limits form is its own confirmation: it opens at once and asks before changing anything.
    if (plan.confirm && !(plan.kind === "local" && plan.local === "edit_form")) {
      ask(a.label || $t(`mission.suggested.${a.id}`), go);
      return;
    }
    if (plan.kind === "command") await act(go);
    else await go();
  }

  function openBudget() {
    bfTokens = "";
    bfMinutes = "";
    bfError = "";
    budgetOpen = true;
  }

  async function saveBudget() {
    if (!m || busy) return;
    const rev = budgetRevision({ tokens: bfTokens, minutes: bfMinutes });
    if (rev.error !== null) {
      bfError = $t(rev.error);
      return;
    }
    const id = m.id;
    const resume = m.state === "blocked" || m.state === "partial" || m.state === "paused";
    busy = true;
    bfError = "";
    error = "";
    notice = null;
    try {
      await reviseBudget(id, rev.budget, resume);
      budgetOpen = false;
      notice = $t(resume ? "mission.budget_form.done_resumed" : "mission.budget_form.done");
    } catch (e) {
      const raw = errorText(e);
      const key = budgetErrorKey(raw);
      bfError = key ? $t(key) : raw;
    } finally {
      busy = false;
    }
    await load(id);
  }

  function startEditing() {
    if (!detail) return;
    draftCriteria = structuredClone($state.snapshot(detail.criteria)) as Criterion[];
    reviseReason = "";
    editing = true;
  }

  function saveCriteria() {
    if (!m || !reviseReason.trim()) return;
    const id = m.id;
    const next = $state.snapshot(draftCriteria) as Criterion[];
    void act(async () => {
      await reviseCriteria(id, next, reviseReason.trim());
      editing = false;
    });
  }

  async function exportDiagnostics() {
    if (!m) return;
    const id = m.id;
    let path: string | null = null;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const r = await save({ defaultPath: `mission-${id}.json`, filters: [{ name: "JSON", extensions: ["json"] }] });
      path = typeof r === "string" ? r : null;
    } catch {
      error = $t("mission.detail.picker_failed");
      return;
    }
    if (!path) return;
    const chosen = path;
    void act(() => missionDiagnostics(id, chosen), $t("mission.detail.exported", { path: chosen }));
  }

  /** The same export to a path typed by hand (no native dialog). */
  function exportTyped() {
    if (!m) return;
    const id = m.id;
    const typed = exportPath.trim();
    if (!isAbsolutePath(typed)) {
      error = $t("mission.path.err_absolute");
      return;
    }
    const chosen = /\.json$/i.test(typed) ? typed : `${typed.replace(/[\\/]+$/, "")}/mission-${id}.json`;
    void act(() => missionDiagnostics(id, chosen).then(() => (exportPath = "")), $t("mission.detail.exported", { path: chosen }));
  }

  function evidenceText(ev: unknown): string {
    if (ev == null) return "";
    return typeof ev === "string" ? ev : JSON.stringify(ev, null, 2);
  }
</script>

{#if loading && !detail}
  <p class="hint pad" role="status">{$t("mission.detail.loading")}</p>
{:else if !detail || !m}
  <div class="pad">
    <p class="error" role="alert">{error || $t("mission.detail.not_found")}</p>
  </div>
{:else}
  <div class="detail" aria-busy={busy}>
    {#if m.block}
      <div class="notice warn" role="region" aria-label={$t("mission.detail.blocked_title")}>
        <strong class="block-summary">{m.block.summary}</strong>
        {#if m.block.suggested_actions.length}
          <div class="actions">
            {#each m.block.suggested_actions as a (a.id)}
              <button type="button" class="button" disabled={busy} title={a.effect} onclick={() => void suggested(a)}>
                {a.label || $t(`mission.suggested.${a.id}`)}{#if a.needs_authorization}{" · "}{$t("mission.detail.asks_first")}{/if}
              </button>
            {/each}
            {#if limitKind && !m.block.suggested_actions.some((a) => a.id === "raise_budget") && !budgetOpen}
              <button type="button" class="button" disabled={busy} onclick={openBudget}>{$t("mission.budget_form.open")}</button>
            {/if}
          </div>
        {:else if limitKind && !budgetOpen}
          <div class="actions">
            <button type="button" class="button" disabled={busy} onclick={openBudget}>{$t("mission.budget_form.open")}</button>
          </div>
        {/if}
        {#if budgetOpen}
          <form class="budget-form" aria-label={$t("mission.budget_form.title")} onsubmit={(e) => { e.preventDefault(); void saveBudget(); }}>
            <strong>{$t("mission.budget_form.title")}</strong>
            <p class="hint">{$t(external ? "mission.budget_form.hint_external" : "mission.budget_form.hint")}</p>
            <div class="bf-grid">
              <label class="field">
                <span class="field-label">{$t("mission.budget_form.tokens")}</span>
                <span class="dim">
                  {m.budget.tokens != null ? $t("mission.budget_form.tokens_now", { spent: m.spent.tokens, limit: m.budget.tokens }) : $t("mission.budget_form.tokens_unlimited", { spent: m.spent.tokens })}
                </span>
                <input class="input" type="text" inputmode="numeric" bind:value={bfTokens} placeholder={m.budget.tokens != null ? String(m.budget.tokens * 2) : "50000"} autocomplete="off" />
              </label>
              <label class="field">
                <span class="field-label">{$t("mission.budget_form.minutes")}</span>
                <span class="dim">
                  {m.budget.max_minutes != null ? $t("mission.budget_form.minutes_now", { spent: minutesUsed(m.spent), limit: m.budget.max_minutes }) : $t("mission.budget_form.minutes_unlimited", { spent: minutesUsed(m.spent) })}
                </span>
                <input class="input" type="text" inputmode="numeric" bind:value={bfMinutes} placeholder={m.budget.max_minutes != null ? String(m.budget.max_minutes * 2) : "30"} autocomplete="off" />
              </label>
            </div>
            <p class="hint">{$t(limitKind === "minutes" ? "mission.budget_form.why_minutes" : "mission.budget_form.keep_blank")}</p>
            {#if bfError}<p class="error" role="alert">{bfError}</p>{/if}
            <div class="actions">
              <button type="submit" class="button active" disabled={busy || (!bfTokens.trim() && !bfMinutes.trim())}>
                {$t(m.state === "blocked" || m.state === "partial" || m.state === "paused" ? "mission.budget_form.submit_resume" : "mission.budget_form.submit")}
              </button>
              <button type="button" class="button" onclick={() => (budgetOpen = false)}>{$t("mission.cancel")}</button>
            </div>
          </form>
        {/if}
        <details class="adv">
          <summary>{$t("mission.detail.block_details")}</summary>
          <dl>
            <dt>{$t("mission.detail.block_code")}</dt><dd><code>{m.block.code}</code> · {m.block.stage}</dd>
            <dt>{$t("mission.detail.block_retryable")}</dt><dd>{$t(m.block.retryable ? "mission.yes" : "mission.no")}</dd>
            {#if m.block.correlation_id}<dt>{$t("mission.detail.correlation")}</dt><dd><code>{m.block.correlation_id}</code></dd>{/if}
            {#if m.block.evidence_refs.length}<dt>{$t("mission.detail.evidence_refs")}</dt><dd><code>{m.block.evidence_refs.join(", ")}</code></dd>{/if}
          </dl>
          {#if m.block.detail != null}<pre class="pre">{evidenceText(m.block.detail)}</pre>{/if}
        </details>
      </div>
    {/if}

    {#if approvals.length}
      <div class="notice approval" role="region" aria-label={$t("mission.approval.title")}>
        <strong>{$t("mission.approval.title")}</strong>
        <span class="hint">{$t("mission.approval.hint")}</span>
        {#each approvals as a (a.tool_call_id)}
          <div class="approval-item">
            <div class="crit-line">
              <span class="tag warn">{a.tool}</span>
              <span class="dim">{agentName(a.agent)}</span>
            </div>
            {#if a.preview}<pre class="pre">{a.preview}</pre>{/if}
            <div class="actions">
              <button type="button" class="button active" disabled={answering !== null} onclick={() => void answer(a, true, false)}>{$t("mission.approval.once")}</button>
              <button type="button" class="button" disabled={answering !== null} onclick={() => void answer(a, true, true)}>{$t("mission.approval.always")}</button>
              <button type="button" class="button" disabled={answering !== null} onclick={() => void answer(a, false, false)}>{$t("mission.approval.deny")}</button>
            </div>
          </div>
        {/each}
      </div>
    {/if}

    {#if confirming}
      <div class="notice confirm" role="alertdialog" aria-label={confirming.label}>
        <span>{confirming.label}</span>
        <div class="actions">
          <button type="button" class="button active" disabled={busy} onclick={() => { const c = confirming; if (c) void act(c.run); }}>{confirming.yes}</button>
          <button type="button" class="button" onclick={() => (confirming = null)}>{confirming.no}</button>
        </div>
      </div>
    {/if}
    {#if notice}<p class="notice" role="status">{notice}</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}

    <div class="field-label">{$t("mission.detail.objective")}</div>
    <div class="text">{m.objective}</div>
    <p class="hint">
      <span class="pill {missionTone(m.state)}">{$t(missionStateKey(m.state))}</span>
      {#if external}<span class="tag ext" title={$t("mission.external.hint")}>{clientName ? $t("mission.external.badge", { name: clientName }) : $t("mission.external.badge_unnamed")}</span>{/if}
      {m.room_id ? $t("mission.detail.by_group") : $t("mission.detail.by_bot", { name: agentName(m.bot_id) })}
      {#if m.started_ms}{" · "}{$t("mission.detail.started", { when: shortTime(m.started_ms) })}{/if}
      {#if m.finished_ms}{" · "}{$t("mission.detail.finished", { when: shortTime(m.finished_ms) })}{/if}
    </p>
    {#if m.result?.summary}
      <div class="field-label">{$t("mission.detail.result")}</div>
      <div class="text">{m.result.summary}</div>
    {/if}

    <section class="sec" aria-labelledby="crit-{m.id}">
      <h3 id="crit-{m.id}">{$t("mission.detail.criteria_title")}</h3>
      <p class="verdict-line">
        <strong>{$t(summary.key, summary.params)}</strong>
        {#if detail.verdict}<span class="tag">{$t(completionKey(detail.verdict.completion))}</span>{/if}
      </p>
      <p class="hint">{$t("mission.detail.done_rule")}</p>
      {#if rows.length === 0}
        <p class="hint">{$t("mission.criteria.empty")}</p>
      {:else}
        <ul class="crit-list">
          {#each rows as r (r.id)}
            {@const def = detail.criteria.find((c) => c.id === r.id)}
            <li>
              <div class="crit-line">
                <span class="pill {r.tone}">{$t(r.statusKey)}</span>
                <span class="crit-title">{titleOf(r)}</span>
                <span class="tag">{$t(`mission.severity.${r.severity}`)}</span>
                <span class="tag">{$t(`mission.kind.${r.kind}`)}</span>
                {#if !r.executable}<span class="tag warn">{$t("mission.criteria.not_executable")}</span>{/if}
                {#if r.origin && r.origin !== "user"}<span class="tag">{$t(originKey(r.origin, m.auth_origin))}</span>{/if}
              </div>
              {#if def}<code class="spec">{specSummary(def)}</code>{/if}
              {#if r.detail}<p class="hint">{r.detail}</p>{/if}
              {#if r.needsDecision && !isFinal(m.state)}
                <div class="decide">
                  <span class="hint">{def?.kind === "human" ? String(def.spec?.prompt ?? "") : $t("mission.detail.decide_hint")}</span>
                  <div class="actions">
                    <button type="button" class="button active" disabled={busy} onclick={() => act(() => acceptCriterion(m!.id, r.id, true, decisionNote).then(() => (decisionNote = "")))}>{$t("mission.detail.accept")}</button>
                    <button type="button" class="button" disabled={busy} onclick={() => act(() => acceptCriterion(m!.id, r.id, false, decisionNote).then(() => (decisionNote = "")))}>{$t("mission.detail.reject")}</button>
                    <input class="input grow" type="text" bind:value={decisionNote} placeholder={$t("mission.detail.decision_note")} aria-label={$t("mission.detail.decision_note")} />
                  </div>
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
      {#if editing}
        <div class="edit">
          <CriteriaEditor bind:criteria={draftCriteria} disabled={busy} />
          <label class="field">
            <span class="field-label">{$t("mission.detail.revise_reason")}</span>
            <input class="input" type="text" bind:value={reviseReason} />
          </label>
          <p class="hint">{$t("mission.detail.revise_hint")}</p>
          <div class="actions">
            <button type="button" class="button active" disabled={busy || !reviseReason.trim()} onclick={saveCriteria}>{$t("mission.detail.revise_save")}</button>
            <button type="button" class="button" onclick={() => (editing = false)}>{$t("mission.cancel")}</button>
          </div>
        </div>
      {:else if !isFinal(m.state)}
        <div class="actions">
          <button type="button" class="button" disabled={busy} onclick={startEditing}>{$t("mission.detail.revise")}</button>
        </div>
      {/if}
    </section>

    <section class="sec" aria-labelledby="tasks-{m.id}">
      <h3 id="tasks-{m.id}">{$t("mission.detail.tasks_title")}</h3>
      {#if detail.tasks.length === 0}
        <p class="hint">{$t("mission.detail.tasks_empty")}</p>
      {:else}
        <ol class="task-list">
          {#each detail.tasks as task (task.id)}
            <li>
              <div class="crit-line">
                <span class="pill {taskTone(task.state)}">{$t(taskStateKey(task.state))}</span>
                <span class="crit-title">{task.title ? taskTitle(task.title) : task.input}</span>
                {#if task.bot_id}<span class="tag">{agentName(task.bot_id)}</span>{/if}
                {#if task.attempts > 0}<span class="dim">{$t("mission.detail.attempts", { count: task.attempts, max: task.max_attempts })}</span>{/if}
                <button type="button" class="button small" aria-expanded={openLog === task.id} onclick={() => (openLog = openLog === task.id ? null : task.id)}>{$t("mission.detail.log")}</button>
              </div>
              {#if task.error}<p class="text err">{task.error}</p>{/if}
              {#if task.result && task.state === "done"}<p class="hint clamp">{task.result}</p>{/if}
              {#if task.state === "blocked" && !taskEffectUnknown(task, detail.effects)}
                <p class="hint">{$t("mission.detail.blocked_before_call")}</p>
              {:else if task.state === "blocked"}
                <div class="decide">
                  <span class="hint">{$t("mission.detail.unblock_hint")}</span>
                  <div class="actions">
                    <button type="button" class="button" disabled={busy} onclick={() => act(() => unblockTask(m!.id, task.id, true, note))}>{$t("mission.detail.unblock_confirmed")}</button>
                    <button type="button" class="button" disabled={busy} onclick={() => act(() => unblockTask(m!.id, task.id, false, note))}>{$t("mission.detail.unblock_retry")}</button>
                    <button type="button" class="button" disabled={busy} onclick={() => act(() => unblockTask(m!.id, task.id, null, note))}>{$t("mission.detail.unblock_unknown")}</button>
                  </div>
                </div>
              {/if}
              {#if openLog === task.id}<TaskLog missionId={m.id} taskId={task.id} />{/if}
            </li>
          {/each}
        </ol>
      {/if}
    </section>

    <section class="sec" aria-labelledby="budget-{m.id}">
      <h3 id="budget-{m.id}">{$t("mission.detail.budget_title")}</h3>
      {#if spend}<p class="text">{$t(spend.key, spend.params)}</p>{/if}
      {#if m.spent.unknown_cost_calls > 0}<p class="hint">{$t("mission.budget.unknown_note")}</p>{/if}
      <p class="hint">{$t("mission.budget.usage", { tokens: m.spent.tokens, turns: m.spent.turns })}</p>
      {#each budgetLimits(m.budget) as l (l.key)}<p class="hint">{$t(l.key, l.params)}</p>{/each}
      {#if budgetLimits(m.budget).length === 0}<p class="hint">{$t("mission.budget.no_limit")}</p>{/if}
      {#if ratio != null}<meter class="meter" min="0" max="1" value={ratio} aria-label={$t("mission.detail.budget_title")}></meter>{/if}
      <details class="adv">
        <summary>{$t("mission.detail.budget_details")}</summary>
        {#if Object.keys(m.spent.by_purpose).length}
          <ul class="plain">{#each Object.entries(m.spent.by_purpose) as [k, v] (k)}{@const pk = purposeKey(k)}<li>{pk ? $t(pk) : k}: {$t("mission.budget.tokens_n", { count: v })}</li>{/each}</ul>
        {/if}
        {#if detail.pool}
          <dl>
            <dt>{$t("mission.detail.pool_in_flight")}</dt><dd>{detail.pool.in_flight}</dd>
            <dt>{$t("mission.detail.pool_reserved")}</dt>
            <dd>
              ${detail.pool.reserved_usd.toFixed(4)} · {detail.pool.reserved_tokens} tokens
              {#if detail.pool.reserved_unknown_cost > 0}{" · "}{$t("mission.budget.reserved_unknown", { calls: detail.pool.reserved_unknown_cost })}{/if}
            </dd>
          </dl>
        {/if}
      </details>
    </section>

    <section class="sec" aria-labelledby="ctx-{m.id}">
      <h3 id="ctx-{m.id}">{$t("mission.detail.context_title")}</h3>
      <p class="hint">{$t("mission.detail.context_hint")}</p>
      <textarea class="input" rows="2" bind:this={noteBox} bind:value={note} placeholder={$t("mission.detail.note_placeholder")} aria-label={$t("mission.detail.context_title")}></textarea>
      <div class="actions">
        <button type="button" class="button" disabled={busy || !note.trim()} onclick={() => act(() => noteMission(m!.id, note.trim()).then(() => (note = "")), $t("mission.detail.note_saved"))}>{$t("mission.detail.add_note")}</button>
        {#if resumable && note.trim()}
          <button type="button" class="button active" disabled={busy} onclick={() => act(() => resumeMission(m!.id, note).then(() => (note = "")))}>{$t("mission.detail.resume_with_note")}</button>
        {/if}
      </div>
      {#if detail.context_preview}
        <details class="adv"><summary>{$t("mission.detail.context_preview")}</summary><pre class="pre">{detail.context_preview}</pre></details>
      {/if}
    </section>

    <section class="sec" aria-labelledby="ev-{m.id}">
      <h3 id="ev-{m.id}">{$t("mission.detail.evidence_title")}</h3>
      {#if detail.receipts.length === 0}
        <p class="hint">{$t("mission.detail.evidence_empty")}</p>
      {:else}
        <ul class="plain receipts">
          {#each [...activeReceipts, ...staleReceipts] as r (r.id)}
            {@const rc = detail.criteria.find((c) => c.id === r.criterion_id)}
            <li class:struck={!!r.invalidated_ms}>
              <details>
                <summary>
                  <span class="badge {r.status === 'pass' ? 'good' : r.status === 'fail' ? 'bad' : ''}">{RECEIPT_STATUSES.includes(r.status) ? $t(criterionStatusKey(r.status as CriterionStatus)) : r.status}</span>
                  <span class="rc-title">{rc ? titleOf(rc) : r.criterion_id}</span>
                  <span class="dim">{r.verifier} · v{r.criterion_version} · {shortTime(r.created_ms)}</span>
                  {#if r.invalidated_ms}<span class="dim">· {$t("mission.detail.invalidated", { reason: r.invalidated_reason ?? "—" })}</span>{/if}
                </summary>
                <dl>
                  {#if r.exit_code != null}<dt>{$t("mission.detail.exit_code")}</dt><dd>{r.exit_code}</dd>{/if}
                  {#if r.artifact_ref}<dt>{$t("mission.detail.artifact")}</dt><dd><code>{r.artifact_ref}</code>{#if r.artifact_digest}{" · "}<code>{r.artifact_digest.slice(0, 12)}</code>{/if}</dd>{/if}
                  {#if r.confidence != null}<dt>{$t("mission.detail.confidence")}</dt><dd>{Math.round(r.confidence * 100)}%</dd>{/if}
                </dl>
                {#if r.evidence != null}<pre class="pre">{evidenceText(r.evidence)}</pre>{/if}
              </details>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <div class="actions main-actions">
      {#each actions as a (a)}
        <button type="button" class="button" class:active={a === "start" || a === "resume"} disabled={busy} onclick={() => runAction(a)}>{$t(`mission.action.${a}`)}</button>
      {/each}
      <button type="button" class="button" disabled={busy} onclick={exportDiagnostics}>{$t("mission.detail.export")}</button>
      {#if m.bot_id}<a class="link" href={`/llm?agent=${encodeURIComponent(m.bot_id)}`}>{$t("mission.detail.open_chat")}</a>{/if}
    </div>
    <form class="actions typed" onsubmit={(e) => { e.preventDefault(); exportTyped(); }}>
      <input class="input grow" type="text" bind:value={exportPath} placeholder={$t("mission.path.export_placeholder")} aria-label={$t("mission.path.export_label")} autocomplete="off" spellcheck={false} />
      <button type="submit" class="button" disabled={busy || !exportPath.trim()}>{$t("mission.path.export_submit")}</button>
    </form>

    <details class="adv">
      <summary>{$t("mission.detail.advanced")}</summary>
      <dl>
        <dt>ID</dt><dd><code>{m.id}</code> · rev {m.revision}</dd>
        <dt>{$t("mission.detail.adv_origin")}</dt><dd>{#if authOriginKey(m.auth_origin)}{$t(authOriginKey(m.auth_origin)!)}{#if external && clientName}{" · "}{clientName}{/if}{:else}<code>{m.auth_origin}</code>{/if}</dd>
        <dt>{$t("mission.detail.adv_workspace")}</dt><dd><code class="path" title={m.workspace ?? ""}>{m.workspace ? shortPath(m.workspace, 56) : "—"}</code></dd>
        <dt>{$t("mission.detail.adv_context")}</dt><dd>{m.context_kind}</dd>
        <dt>{$t("mission.detail.adv_criteria_version")}</dt><dd>{m.criteria_version}</dd>
        <dt>{$t("mission.detail.adv_replay")}</dt><dd>{$t(m.allow_replay ? "mission.yes" : "mission.no")}</dd>
        <dt>{$t("mission.detail.adv_driving")}</dt><dd>{$t(detail.driving ? "mission.yes" : "mission.no")}</dd>
        {#if m.pins}<dt>{$t("mission.detail.adv_pins")}</dt><dd><code>{JSON.stringify(m.pins)}</code></dd>{/if}
      </dl>
      {#if detail.checkpoints.length}
        <div class="field-label">{$t("mission.detail.adv_checkpoints")}</div>
        <ul class="plain">{#each detail.checkpoints as c (c.seq)}<li>#{c.seq} · {c.reason} · {shortTime(c.created_ms)}</li>{/each}</ul>
      {/if}
      {#if detail.effects.length}
        <div class="field-label">{$t("mission.detail.adv_effects")}</div>
        <ul class="plain">{#each detail.effects as e (e.key)}{@const ek = effectStateKey(e.state)}{@const kk = effectKindKey(e.kind)}<li title={e.detail ?? ""}>{kk ? $t(kk) : e.kind} · {ek ? $t(ek) : e.state}</li>{/each}</ul>
      {/if}
      {#if detail.events.length}
        <div class="field-label">{$t("mission.detail.adv_events")}</div>
        <pre class="pre">{detail.events.map((e) => `${e.seq} ${e.kind} ${JSON.stringify(e.payload)}`).join("\n")}</pre>
      {/if}
    </details>
  </div>
{/if}

<style>
  .detail { display: flex; flex-direction: column; gap: var(--space-2); padding: var(--space-2) var(--space-3) var(--space-3); min-width: 0; }
  .pad { padding: var(--space-2) var(--space-3); }
  .sec { display: flex; flex-direction: column; gap: var(--space-2); padding-top: var(--space-2); border-top: var(--hairline) solid var(--separator); }
  h3 { margin: 0; font-size: var(--text-base); font-weight: 600; }
  .notice { margin: 0; padding: var(--space-2) var(--space-3); border-radius: var(--radius-sm); background: var(--fill-2); font-size: var(--text-sm); display: flex; flex-direction: column; gap: var(--space-2); }
  .notice.warn { background: color-mix(in srgb, var(--orange) 12%, transparent); }
  .notice.confirm { background: var(--accent-soft); }
  .notice.approval { background: color-mix(in srgb, var(--orange) 16%, transparent); border: var(--hairline) solid color-mix(in srgb, var(--orange) 45%, transparent); }
  .approval-item { display: flex; flex-direction: column; gap: 6px; padding-top: var(--space-2); border-top: var(--hairline) solid var(--separator); }
  .typed { max-width: 560px; }
  /* Middle-truncated by shortPath (the folder name stays visible); wraps instead of cutting the end. */
  .path { display: inline-block; max-width: 100%; overflow-wrap: anywhere; vertical-align: bottom; }
  .budget-form { display: flex; flex-direction: column; gap: var(--space-2); padding-top: var(--space-2); border-top: var(--hairline) solid var(--separator); }
  .bf-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: var(--space-2) var(--space-3); }
  .tag.ext { color: var(--accent-text); background: var(--accent-soft); }
  .block-summary { font-size: var(--text-base); overflow-wrap: anywhere; }
  .hint { margin: 0; font-size: var(--text-sm); color: var(--text-dim); display: flex; flex-wrap: wrap; gap: 4px 8px; align-items: center; }
  .dim { font-size: var(--text-xs); color: var(--text-dim); }
  .text { font-size: var(--text-sm); white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; margin: 0; }
  .text.err, .error { color: var(--red); font-size: var(--text-sm); margin: 0; overflow-wrap: anywhere; }
  .clamp { display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
  .verdict-line { margin: 0; display: flex; flex-wrap: wrap; gap: var(--space-2); align-items: center; font-size: var(--text-sm); }
  .crit-list, .task-list, .plain { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-2); min-width: 0; }
  .crit-list li, .task-list li { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .crit-line { display: flex; flex-wrap: wrap; gap: 6px var(--space-2); align-items: center; font-size: var(--text-sm); }
  .crit-title { font-weight: 500; overflow-wrap: anywhere; }
  .spec { font-family: var(--font-mono); font-size: var(--text-xs); color: var(--text-muted); overflow-wrap: anywhere; }
  .decide { display: flex; flex-direction: column; gap: 6px; padding: var(--space-2); border-radius: var(--radius-sm); background: color-mix(in srgb, var(--orange) 8%, transparent); }
  .edit { display: flex; flex-direction: column; gap: var(--space-2); }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .grow { flex: 1 1 180px; min-width: 0; }
  textarea.input { resize: vertical; height: auto; font-family: inherit; }
  .actions { display: flex; flex-wrap: wrap; gap: var(--space-2); align-items: center; }
  .main-actions { padding-top: var(--space-2); border-top: var(--hairline) solid var(--separator); }
  .link { color: var(--accent-text); font-size: var(--text-sm); }
  .meter { width: 100%; max-width: 320px; }
  .tag, .badge { flex-shrink: 0; padding: 0 6px; border-radius: 999px; font-size: var(--text-xs); color: var(--text-muted); background: var(--fill-2); }
  .badge { font-weight: 600; }
  .tag.warn { color: var(--orange); background: color-mix(in srgb, var(--orange) 14%, transparent); }
  .badge.good { color: var(--green); background: color-mix(in srgb, var(--green) 14%, transparent); }
  .badge.bad { color: var(--red); background: color-mix(in srgb, var(--red) 14%, transparent); }
  .pill { display: inline-flex; align-items: center; gap: 6px; flex-shrink: 0; padding: 1px 8px; border-radius: 999px; font-size: var(--text-xs); font-weight: 600; white-space: nowrap; color: var(--text-muted); background: var(--fill-2); }
  .pill.blue { color: var(--blue); background: color-mix(in srgb, var(--blue) 14%, transparent); }
  .pill.orange { color: var(--orange); background: color-mix(in srgb, var(--orange) 16%, transparent); }
  .pill.green { color: var(--green); background: color-mix(in srgb, var(--green) 14%, transparent); }
  .pill.red { color: var(--red); background: color-mix(in srgb, var(--red) 14%, transparent); }
  .receipts summary { display: flex; flex-wrap: wrap; gap: 6px var(--space-2); align-items: center; cursor: pointer; font-size: var(--text-sm); }
  .struck .rc-title { text-decoration: line-through; color: var(--text-dim); }
  .rc-title { overflow-wrap: anywhere; }
  .pre { margin: 0; padding: var(--space-2); max-height: 260px; overflow: auto; border-radius: var(--radius-sm); background: var(--fill-2); font-family: var(--font-mono); font-size: var(--text-xs); white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
  .adv summary { cursor: pointer; font-size: var(--text-sm); color: var(--accent-text); }
  .adv dl, .receipts dl { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: var(--space-1) var(--space-3); margin: var(--space-2) 0; font-size: var(--text-xs); }
  dt { color: var(--text-dim); }
  dd { margin: 0; overflow-wrap: anywhere; }
  summary:focus-visible, .button:focus-visible, .link:focus-visible { outline: var(--focus-ring); outline-offset: 2px; border-radius: var(--radius-sm); }
</style>
