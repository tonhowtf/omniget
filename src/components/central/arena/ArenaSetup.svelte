<script lang="ts">
  // Nova arena: projeto (repo git), branch base, prompt (ou o plano aprovado
  // de uma thread), as ferramentas que competem (instância + modelo por
  // coluna), modo de acesso e o comando de verificação opcional.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { threadsStore } from "$lib/central/threads/threads-store.svelte";
  import type { AccessMode, PlanRow, TurnsPage } from "$lib/central/threads/types";
  import { ACCESS_MODES, MODEL_HINTS, driverName, driverTint } from "$lib/central/threads-ui/drivers";
  import { vcsBranches, vcsStatus, type BranchInfo } from "$lib/central/vcs";
  import { arenaCreate, arenaError, promptFromPlan, type ArenaView, type EntrySpec } from "$lib/central/arena";
  import Icon from "$components/central/threads/Icon.svelte";

  let {
    planThreadId = null,
    planId = null,
    projectId: initialProject = null,
    oncreated,
    oncancel,
  }: {
    planThreadId?: string | null;
    planId?: string | null;
    projectId?: string | null;
    oncreated: (a: ArenaView) => void;
    oncancel?: () => void;
  } = $props();

  const LAST_KEY = "omniget.central.arena.last.v1";
  type Row = { key: string; instanceId: string; driver: string; model: string };
  type Last = { projectId?: string; rows?: Row[]; mode?: AccessMode; verify?: string };

  function readLast(): Last {
    try {
      return JSON.parse(localStorage.getItem(LAST_KEY) ?? "{}") as Last;
    } catch {
      return {};
    }
  }
  const last = readLast();

  let projectId = $state(untrack(() => initialProject) ?? last.projectId ?? "");
  let title = $state("");
  let prompt = $state("");
  let baseBranch = $state("");
  let branches = $state<BranchInfo[]>([]);
  let isRepo = $state<boolean | null>(null);
  let mode = $state<AccessMode>(last.mode ?? "auto-accept-edits");
  let verify = $state(last.verify ?? "");
  let timeoutMin = $state(10);
  let rows = $state<Row[]>(last.rows ?? []);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let sourceThread = $state<string | null>(untrack(() => planThreadId));
  let sourcePlan = $state<string | null>(untrack(() => planId));
  let planTitle = $state<string | null>(null);
  let planPickerOpen = $state(false);

  let projects = $derived(threadsStore.projects.filter((p) => p.workspaceRoot));
  let project = $derived(projects.find((p) => p.projectId === projectId) ?? null);
  let instances = $derived(threadsStore.instances.filter((i) => i.instance.enabled !== false));
  let canStart = $derived(!!project && isRepo === true && prompt.trim().length > 0 && rows.length > 0 && !busy);

  $effect(() => {
    if (!projectId && projects.length) projectId = projects[0].projectId;
  });

  // Branches do projeto escolhido.
  $effect(() => {
    const root = project?.workspaceRoot;
    if (!root) return;
    isRepo = null;
    void (async () => {
      try {
        const st = await vcsStatus(root);
        isRepo = st.is_repo;
        if (!st.is_repo) return;
        branches = (await vcsBranches(root)).filter((b) => !b.remote);
        if (!baseBranch || !branches.some((b) => b.name === baseBranch)) baseBranch = st.branch ?? "";
      } catch {
        isRepo = false;
      }
    })();
  });

  // Instâncias sumidas da lista (conta removida) saem das linhas lembradas.
  $effect(() => {
    if (!instances.length) return;
    const ids = new Set(instances.map((i) => i.instance.id));
    const kept = untrack(() => rows).filter((r) => ids.has(r.instanceId));
    if (kept.length !== untrack(() => rows).length) rows = kept;
  });

  // Plano aprovado de uma thread → prompt.
  $effect(() => {
    const tid = sourceThread;
    if (!tid) return;
    const pid = untrack(() => sourcePlan);
    void loadPlan(tid, pid);
  });

  async function loadPlan(threadId: string, wanted: string | null) {
    try {
      const page = await invoke<TurnsPage>("threads_turns_page", { threadId, beforeTurn: null, limit: 50 });
      const plans: PlanRow[] = page.turns.flatMap((d) => d.plans).filter((p) => (p.markdown ?? "").trim());
      const plan = (wanted && plans.find((p) => p.planId === wanted)) || plans[plans.length - 1];
      const thread = threadsStore.threads.find((x) => x.threadId === threadId);
      if (thread && !initialProject) projectId = thread.projectId;
      if (!plan) {
        error = $t("llm.central.arena.setup.no_plan") as string;
        return;
      }
      sourcePlan = plan.planId;
      planTitle = /^\s*#{1,6}\s+(.+)$/m.exec(plan.markdown ?? "")?.[1]?.trim() ?? thread?.title ?? null;
      prompt = promptFromPlan(plan.markdown ?? "", $t("llm.central.arena.setup.plan_lead") as string);
      if (!title && planTitle) title = planTitle;
    } catch (e) {
      error = arenaError(e).message;
    }
  }

  function addRow(instanceId: string) {
    const inst = instances.find((i) => i.instance.id === instanceId);
    if (!inst || rows.length >= 8) return;
    rows = [...rows, { key: crypto.randomUUID(), instanceId, driver: inst.instance.driver, model: "" }];
  }

  function removeRow(key: string) {
    rows = rows.filter((r) => r.key !== key);
  }

  function instanceOf(id: string) {
    return instances.find((i) => i.instance.id === id) ?? null;
  }

  let threadsWithWork = $derived(
    threadsStore.threads
      .filter((x) => !x.archivedAt && x.turnCount > 0)
      .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
      .slice(0, 40),
  );

  async function start() {
    if (!canStart || !project) return;
    busy = true;
    error = null;
    try {
      const entries: EntrySpec[] = rows.map((r) => ({
        instanceId: r.instanceId,
        driver: r.driver,
        model: r.model.trim() || null,
      }));
      const arena = await arenaCreate({
        projectId: project.projectId,
        title: title.trim() || null,
        prompt: prompt.trim(),
        baseBranch: baseBranch || null,
        verifyCommand: verify.trim() || null,
        verifyTimeoutSecs: Math.max(1, timeoutMin) * 60,
        runtimeMode: mode,
        entries,
        sourceThreadId: sourceThread,
        sourcePlanId: sourcePlan,
      });
      try {
        localStorage.setItem(LAST_KEY, JSON.stringify({ projectId, rows, mode, verify } satisfies Last));
      } catch {
        /* storage off */
      }
      oncreated(arena);
    } catch (e) {
      error = arenaError(e).message;
    } finally {
      busy = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      void start();
    }
  }
</script>

<section class="setup" aria-label={$t("llm.central.arena.setup.title")}>
  <header class="head">
    <h2>{$t("llm.central.arena.setup.title")}</h2>
    <p class="lede">{$t("llm.central.arena.setup.lede")}</p>
  </header>

  <div class="grid">
    <label class="field">
      <span>{$t("llm.central.arena.setup.project")}</span>
      <select bind:value={projectId} disabled={busy}>
        {#each projects as p (p.projectId)}
          <option value={p.projectId}>{p.title}</option>
        {/each}
      </select>
      {#if project}<small class="mono">{project.workspaceRoot}</small>{/if}
      {#if !projects.length}<small>{$t("llm.central.arena.setup.no_projects")}</small>{/if}
      {#if isRepo === false}<small class="warn">{$t("llm.central.arena.setup.not_repo")}</small>{/if}
    </label>

    <label class="field">
      <span>{$t("llm.central.arena.setup.base")}</span>
      <select bind:value={baseBranch} disabled={busy || !isRepo}>
        {#each branches as b (b.name)}
          <option value={b.name}>{b.name}{b.current ? " ●" : ""}</option>
        {/each}
      </select>
      <small>{$t("llm.central.arena.setup.base_hint")}</small>
    </label>
  </div>

  <label class="field">
    <span>{$t("llm.central.arena.setup.name")}</span>
    <input type="text" bind:value={title} placeholder={$t("llm.central.arena.setup.name_placeholder")} disabled={busy} />
  </label>

  <div class="field">
    <div class="row-head">
      <span>{$t("llm.central.arena.setup.prompt")}</span>
      <div class="grow"></div>
      {#if planTitle}
        <span class="plan-chip"><Icon name="list-checks" size={13} /> {planTitle}</span>
        <button type="button" class="ghost" onclick={() => { sourceThread = null; sourcePlan = null; planTitle = null; }}>
          <Icon name="x" size={12} />
        </button>
      {/if}
      <button type="button" class="ghost" onclick={() => (planPickerOpen = !planPickerOpen)} aria-expanded={planPickerOpen}>
        <Icon name="list-checks" size={13} /> {$t("llm.central.arena.setup.from_plan")}
      </button>
    </div>
    {#if planPickerOpen}
      <div class="plan-picker">
        {#each threadsWithWork as th (th.threadId)}
          <button type="button" class="pick" onclick={() => { planPickerOpen = false; sourcePlan = null; sourceThread = th.threadId; }}>
            <span class="dot" style:--tint={driverTint(th.driver)}></span>
            <span class="pick-title">{th.title}</span>
            <span class="muted">{th.turnCount}</span>
          </button>
        {:else}
          <p class="muted">{$t("llm.central.arena.setup.no_threads")}</p>
        {/each}
      </div>
    {/if}
    <!-- svelte-ignore a11y_autofocus -->
    <textarea
      bind:value={prompt}
      rows="7"
      placeholder={$t("llm.central.arena.setup.prompt_placeholder")}
      disabled={busy}
      onkeydown={onKey}
      autofocus
    ></textarea>
  </div>

  <div class="field">
    <div class="row-head">
      <span>{$t("llm.central.arena.setup.contenders")}</span>
      <span class="muted">{rows.length}/8</span>
    </div>
    <ul class="rows">
      {#each rows as r (r.key)}
        {@const inst = instanceOf(r.instanceId)}
        <li class="entry" style:--tint={driverTint(r.driver, inst)}>
          <span class="dot"></span>
          <span class="name">{driverName(r.driver, inst)}</span>
          {#if inst && !inst.available}
            <span class="warn small" title={inst.unavailableReason ?? ""}>{$t("llm.central.arena.setup.unavailable")}</span>
          {/if}
          <input
            type="text"
            class="model"
            list={`arena-models-${r.driver}`}
            bind:value={r.model}
            placeholder={$t("llm.central.arena.setup.model_placeholder")}
            disabled={busy}
          />
          <button type="button" class="icon-btn" aria-label={$t("llm.central.arena.setup.remove")} onclick={() => removeRow(r.key)} disabled={busy}>
            <Icon name="x" size={13} />
          </button>
        </li>
      {/each}
    </ul>
    <div class="add">
      {#each instances as inst (inst.instance.id)}
        <button
          type="button"
          class="add-chip"
          style:--tint={driverTint(inst.instance.driver, inst)}
          disabled={busy || rows.length >= 8}
          title={inst.available ? inst.driverLabel : (inst.unavailableReason ?? "")}
          class:off={!inst.available}
          onclick={() => addRow(inst.instance.id)}
        >
          <Icon name="plus" size={12} />
          <span class="dot"></span>
          {driverName(inst.instance.driver, inst)}
        </button>
      {:else}
        <p class="muted">{$t("llm.central.arena.setup.no_instances")}</p>
      {/each}
    </div>
    {#each Object.entries(MODEL_HINTS) as [driver, hints] (driver)}
      <datalist id={`arena-models-${driver}`}>
        {#each hints as h (h)}<option value={h}></option>{/each}
      </datalist>
    {/each}
    <datalist id="arena-models-native"><option value="fake/fake"></option></datalist>
  </div>

  <div class="grid">
    <label class="field">
      <span>{$t("llm.central.arena.setup.access")}</span>
      <select bind:value={mode} disabled={busy}>
        {#each ACCESS_MODES as m (m)}
          <option value={m}>{$t(`llm.central.arena.access.${m}`)}</option>
        {/each}
      </select>
      <small>{$t("llm.central.arena.setup.access_hint")}</small>
    </label>
    <label class="field">
      <span>{$t("llm.central.arena.setup.verify")}</span>
      <input type="text" class="mono" bind:value={verify} placeholder="npm test" disabled={busy} />
      <small>
        {$t("llm.central.arena.setup.verify_hint")}
        <input type="number" class="mins" min="1" max="120" bind:value={timeoutMin} disabled={busy} aria-label={$t("llm.central.arena.setup.timeout")} />
        {$t("llm.central.arena.setup.minutes")}
      </small>
    </label>
  </div>

  {#if error}<p class="err" role="alert">{error}</p>{/if}

  <footer class="foot">
    {#if oncancel}
      <button type="button" class="button" onclick={oncancel} disabled={busy}>{$t("llm.central.arena.cancel")}</button>
    {/if}
    <div class="grow"></div>
    <span class="muted">⌘↩</span>
    <button type="button" class="button active" onclick={start} disabled={!canStart}>
      {#if busy}<Icon name="circle-notch" size={14} />{:else}<Icon name="play" size={14} />{/if}
      {$t("llm.central.arena.setup.start", { count: rows.length })}
    </button>
  </footer>
</section>

<style>
  .setup { max-width: 860px; margin: 0 auto; padding: 20px 24px 32px; display: flex; flex-direction: column; gap: 14px; }
  .head h2 { margin: 0; font-size: var(--text-xl); font-weight: 700; letter-spacing: var(--track-tight); }
  .lede { margin: 4px 0 0; color: var(--text-muted); font-size: var(--text-base); }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 14px; }
  .field { display: flex; flex-direction: column; gap: 5px; min-width: 0; }
  .field > span, .row-head > span:first-child { font-size: var(--text-sm); font-weight: 600; }
  .field small { color: var(--text-muted); font-size: var(--text-xs); }
  .field small.warn, .warn { color: var(--warning); }
  .warn.small { font-size: var(--text-xs); }
  .mono { font-family: var(--font-mono); }
  select, input[type="text"], input[type="number"], textarea {
    font: inherit; font-size: var(--text-base); color: var(--text); background: var(--input-bg, var(--surface));
    border: 1px solid var(--input-border, var(--separator)); border-radius: var(--radius-md); padding: 6px 9px; min-width: 0;
  }
  textarea { resize: vertical; min-height: 120px; line-height: 1.45; }
  .mins { width: 56px; padding: 2px 6px; margin: 0 4px; }
  .row-head { display: flex; align-items: center; gap: 6px; }
  .grow { flex: 1; }
  .muted { color: var(--text-muted); font-size: var(--text-sm); }
  .ghost { display: inline-flex; align-items: center; gap: 4px; border: 0; background: transparent; color: var(--text-muted); font-size: var(--text-sm); padding: 3px 7px; border-radius: 6px; cursor: pointer; }
  .ghost:hover { background: var(--fill-2); color: var(--text); }
  .plan-chip { display: inline-flex; align-items: center; gap: 4px; font-size: var(--text-xs); padding: 2px 8px; border-radius: 999px; background: color-mix(in srgb, var(--purple) 14%, transparent); color: var(--purple); max-width: 260px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .plan-picker { display: flex; flex-direction: column; max-height: 220px; overflow: auto; border: 1px solid var(--separator); border-radius: var(--radius-md); padding: 4px; }
  .pick { display: flex; align-items: center; gap: 8px; border: 0; background: transparent; color: var(--text); text-align: left; padding: 6px 8px; border-radius: 6px; cursor: pointer; font-size: var(--text-base); }
  .pick:hover { background: var(--fill-1); }
  .pick-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rows { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .entry { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border: 1px solid var(--separator); border-left: 3px solid var(--tint); border-radius: var(--radius-md); }
  .entry .name { font-weight: 600; font-size: var(--text-base); min-width: 120px; }
  .entry .model { flex: 1; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--tint, var(--text-muted)); flex-shrink: 0; }
  .add { display: flex; flex-wrap: wrap; gap: 6px; }
  .add-chip { display: inline-flex; align-items: center; gap: 5px; border: 1px dashed var(--separator); background: transparent; color: var(--text); font-size: var(--text-sm); padding: 4px 9px; border-radius: 999px; cursor: pointer; }
  .add-chip:hover:not(:disabled) { border-color: var(--tint); background: var(--fill-1); }
  .add-chip.off { opacity: 0.6; }
  .icon-btn { width: 26px; height: 26px; display: inline-flex; align-items: center; justify-content: center; border: 0; border-radius: 6px; background: transparent; color: var(--text-muted); cursor: pointer; }
  .icon-btn:hover { background: var(--fill-2); color: var(--text); }
  .foot { display: flex; align-items: center; gap: 10px; }
  .foot .button { display: inline-flex; align-items: center; gap: 6px; }
  .err { color: var(--error); font-size: var(--text-sm); margin: 0; overflow-wrap: anywhere; }
</style>
