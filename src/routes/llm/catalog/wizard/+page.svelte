<script lang="ts">
  /**
   * Project wizard: detects the stack of a folder, suggests the pieces of the
   * catalog's 14 project templates plus a generated AGENTS.md, lets the user
   * pick (only what is picked is installed), installs into the tools found
   * for this project through the agentkit plan, then offers a review Job.
   */
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import ProjectPicker from "$components/central/catalog/ProjectPicker.svelte";
  import PlanDialog from "$components/central/catalog/PlanDialog.svelte";
  import RunnerPicker from "$components/central/run/RunnerPicker.svelte";
  import { errText, type InstallPlan } from "$lib/central/catalog";
  import {
    runWizardDetect,
    runWizardPlan,
    runWizardReview,
    type RunnerSpec,
    type WizardPiece,
    type WizardSession,
  } from "$components/central/run/run-api";

  const KINDS: WizardPiece["kind"][] = ["rule", "command", "agent", "setting", "hook", "mcp"];

  let project = $state<string | null>(null);
  let session = $state<WizardSession | null>(null);
  let extra = $state<string[]>([]);
  let selected = $state<string[]>([]);
  let targets = $state<string[]>([]);
  let agentsMd = $state("");
  let plan = $state<InstallPlan | null>(null);
  let applied = $state(false);
  let busy = $state(false);
  let error = $state("");
  let reviewer = $state<RunnerSpec>({ kind: "agent", id: "omni" });
  let reviewJob = $state<string | null>(null);

  let byKind = $derived.by(() => {
    const m = new Map<string, WizardPiece[]>();
    for (const p of session?.pieces ?? []) {
      const list = m.get(p.kind) ?? [];
      list.push(p);
      m.set(p.kind, list);
    }
    return KINDS.filter((k) => m.has(k)).map((k) => [k, m.get(k)!] as const);
  });
  let mdOn = $derived(selected.includes("wizard:project/rule/agents-md"));

  async function detect() {
    if (!project) return;
    busy = true;
    error = "";
    plan = null;
    applied = false;
    reviewJob = null;
    try {
      const s = await runWizardDetect(project, extra);
      session = s;
      selected = s.pieces.filter((p) => p.selected && p.available).map((p) => p.id);
      targets = s.targets.filter((x) => x.suggested).map((x) => x.id);
      agentsMd = s.agents_md;
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  function toggle(list: string[], id: string): string[] {
    return list.includes(id) ? list.filter((x) => x !== id) : [...list, id];
  }

  function toggleKind(kind: string, on: boolean) {
    const ids = (session?.pieces ?? []).filter((p) => p.kind === kind && p.available).map((p) => p.id);
    selected = on ? [...new Set([...selected, ...ids])] : selected.filter((x) => !ids.includes(x));
  }

  async function makePlan() {
    if (!session) return;
    busy = true;
    error = "";
    try {
      plan = await runWizardPlan({ sessionId: session.id, selected, targets, agentsMd: mdOn ? agentsMd : null });
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  async function review() {
    if (!project) return;
    busy = true;
    try {
      const job = await runWizardReview(project, reviewer);
      reviewJob = job.id;
      showToast("success", $t("llm.central.run.wizard.review_started"));
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  function short(id: string): string {
    return id.replace("cct:templates/", "");
  }
</script>

<svelte:head><title>{$t("llm.central.run.wizard.title")}</title></svelte:head>

<div class="page page-wide wizard">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("llm.central.run.wizard.title")}</h1>
      <p class="page-lede">{$t("llm.central.run.wizard.lede")}</p>
    </div>
    <a class="button" href="/llm/catalog">{$t("llm.central.run.wizard.back_catalog")}</a>
  </header>

  <section class="surface-card block">
    <h2 class="section-header-title">1 · {$t("llm.central.run.wizard.step_project")}</h2>
    <ProjectPicker bind:value={project} />
    <div><button type="button" class="button active" disabled={!project || busy} onclick={detect}>{$t("llm.central.run.wizard.detect")}</button></div>
  </section>

  {#if session}
    <section class="surface-card block">
      <h2 class="section-header-title">2 · {$t("llm.central.run.wizard.step_stack")}</h2>
      <dl class="facts">
        <dt>{$t("llm.central.run.wizard.languages")}</dt>
        <dd>{session.stack.languages.map((l) => l.id).join(", ") || "—"}</dd>
        <dt>{$t("llm.central.run.wizard.frameworks")}</dt>
        <dd>{session.stack.frameworks.map((l) => l.id).join(", ") || "—"}</dd>
        <dt>{$t("llm.central.run.wizard.commands")}</dt>
        <dd class="mono">
          {[session.stack.build_command, session.stack.test_command, session.stack.lint_command].filter(Boolean).join(" · ") || "—"}
        </dd>
        {#if session.stack.agent_files.length}
          <dt>{$t("llm.central.run.wizard.existing")}</dt>
          <dd class="mono">{session.stack.agent_files.join(", ")}</dd>
        {/if}
      </dl>
      <div class="field-label">{$t("llm.central.run.wizard.templates")}</div>
      <div class="chips">
        {#each session.all_templates as id (id)}
          {@const auto = session.templates.includes(id) && !extra.includes(id)}
          <label class="chip" class:on={session.templates.includes(id)}>
            <input type="checkbox" checked={session.templates.includes(id)} disabled={auto} onchange={() => { extra = toggle(extra, id); void detect(); }} />
            {short(id)}
          </label>
        {/each}
      </div>
      {#if session.warnings.length}
        <details><summary class="hint">{$t("llm.central.run.wizard.warnings", { count: String(session.warnings.length) })}</summary>
          <ul class="hint">{#each session.warnings as w, i (i)}<li>{w}</li>{/each}</ul>
        </details>
      {/if}
    </section>

    <section class="surface-card block">
      <h2 class="section-header-title">3 · {$t("llm.central.run.wizard.step_pieces")}</h2>
      {#each byKind as [kind, pieces] (kind)}
        {@const avail = pieces.filter((p) => p.available)}
        <div class="group">
          <label class="group-head">
            <input type="checkbox" checked={avail.length > 0 && avail.every((p) => selected.includes(p.id))} onchange={(e) => toggleKind(kind, (e.currentTarget as HTMLInputElement).checked)} />
            <strong>{$t(`llm.central.run.wizard.kind.${kind}`)}</strong>
            <span class="dim">{pieces.filter((p) => selected.includes(p.id)).length}/{pieces.length}</span>
          </label>
          <ul class="pieces">
            {#each pieces as p (p.id)}
              <li class:off={!p.available}>
                <label>
                  <input type="checkbox" checked={selected.includes(p.id)} disabled={!p.available} onchange={() => (selected = toggle(selected, p.id))} />
                  <span class="pname">{p.name}</span>
                  <span class="dim">{short(p.template)}</span>
                </label>
                {#if p.description}<p class="desc">{p.description}</p>{/if}
                {#if p.reason}<p class="warn">{p.reason}</p>{/if}
                {#each p.notes ?? [] as n, i (i)}<p class="note">{n}</p>{/each}
              </li>
            {/each}
          </ul>
        </div>
      {/each}
      {#if mdOn}
        <label class="field">
          <span class="field-label">AGENTS.md</span>
          <textarea class="input mono" rows="14" bind:value={agentsMd}></textarea>
          <span class="hint">{$t("llm.central.run.wizard.agents_md_hint")}</span>
        </label>
      {/if}
    </section>

    <section class="surface-card block">
      <h2 class="section-header-title">4 · {$t("llm.central.run.wizard.step_targets")}</h2>
      <div class="chips">
        {#each session.targets.filter((x) => x.installed || x.in_project) as x (x.id)}
          <label class="chip" class:on={targets.includes(x.id)}>
            <input type="checkbox" checked={targets.includes(x.id)} onchange={() => (targets = toggle(targets, x.id))} />
            {x.name}{x.beta ? " · beta" : ""}{x.in_project ? ` · ${$t("llm.central.run.wizard.in_project")}` : ""}
          </label>
        {/each}
      </div>
      <details>
        <summary class="hint">{$t("llm.central.run.wizard.more_tools")}</summary>
        <div class="chips">
          {#each session.targets.filter((x) => !x.installed && !x.in_project) as x (x.id)}
            <label class="chip">
              <input type="checkbox" checked={targets.includes(x.id)} onchange={() => (targets = toggle(targets, x.id))} />
              {x.name}
            </label>
          {/each}
        </div>
      </details>
      {#if error}<p class="err">{error}</p>{/if}
      <div>
        <button type="button" class="button active" disabled={busy || selected.length === 0 || targets.length === 0} onclick={makePlan}>
          {$t("llm.central.run.wizard.plan", { pieces: String(selected.length), tools: String(targets.length) })}
        </button>
      </div>
    </section>

    {#if applied}
      <section class="surface-card block">
        <h2 class="section-header-title">5 · {$t("llm.central.run.wizard.step_review")}</h2>
        <p class="hint">{$t("llm.central.run.wizard.review_hint")}</p>
        <RunnerPicker bind:value={reviewer} />
        <div>
          <button type="button" class="button active" disabled={busy} onclick={review}>{$t("llm.central.run.wizard.review")}</button>
          {#if reviewJob}<a class="button" href="/llm/jobs">{$t("llm.central.run.wizard.open_jobs")}</a>{/if}
        </div>
      </section>
    {/if}
  {:else if error}
    <p class="err">{error}</p>
  {/if}
</div>

{#if plan}
  <PlanDialog
    {plan}
    onclose={() => (plan = null)}
    onapplied={() => (applied = true)}
    onreplan={() => void makePlan()}
  />
{/if}

<style>
  .wizard {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .page-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    margin-bottom: var(--space-4);
  }
  .facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px var(--space-3);
    margin: 0;
    font-size: var(--text-sm);
  }
  .facts dt {
    color: var(--text-muted);
  }
  .facts dd {
    margin: 0;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 10px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    font-size: var(--text-xs);
    cursor: pointer;
  }
  .chip.on {
    background: var(--accent-soft);
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .group-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .pieces {
    list-style: none;
    margin: 0;
    padding: 0 0 0 var(--space-5);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .pieces label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .pieces li.off {
    opacity: 0.6;
  }
  .pname {
    font-weight: 600;
  }
  .desc,
  .note,
  .warn {
    margin: 0 0 0 26px;
    font-size: var(--text-xs);
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }
  .warn {
    color: var(--warning);
  }
  .dim {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  textarea.input {
    resize: vertical;
    height: auto;
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .err {
    margin: 0;
    color: var(--red);
    font-size: var(--text-sm);
  }
</style>
