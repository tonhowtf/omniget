<script lang="ts">
  /**
   * "See plan": what an install will do, before anything is written. Units by
   * tool (status, compatibility, what is lost, collision renames), the
   * commands that tools will execute (hooks, MCP launchers, statuslines)
   * always on top and highlighted, conflicts, and the diff of every file.
   * "Install" applies the plan (`agentkit_apply`); the result offers undo
   * (`agentkit_restore` of the transaction).
   */
  import { t } from "$lib/i18n";
  import DiffViewer from "$components/central/vcs/DiffViewer.svelte";
  import {
    agentkitApply,
    agentkitRestore,
    errCode,
    errText,
    filePlanToDiff,
    type ApplyReport,
    type InstallPlan,
    type PlanUnit,
  } from "$lib/central/catalog";
  import { showToast } from "$lib/stores/toast-store.svelte";

  let {
    plan,
    onclose,
    onapplied,
    onreplan,
    autoApply = false,
  }: {
    plan: InstallPlan;
    /** Apply right away (the "Install" button): the dialog then shows the result. */
    autoApply?: boolean;
    onclose?: () => void;
    onapplied?: (r: ApplyReport) => void;
    /** Called on AGENTKIT_PLAN_STALE: the parent plans again. */
    onreplan?: () => void;
  } = $props();

  let busy = $state(false);
  let report = $state<ApplyReport | null>(null);
  let restored = $state(false);
  let error = $state("");
  let tab = $state<"units" | "files">("units");
  let dialog = $state<HTMLElement | null>(null);

  let byTarget = $derived.by(() => {
    const m = new Map<string, { name: string; units: PlanUnit[] }>();
    for (const u of plan.units) {
      const g = m.get(u.target) ?? { name: u.target_name, units: [] };
      g.units.push(u);
      m.set(u.target, g);
    }
    return [...m.entries()];
  });
  let counts = $derived.by(() => {
    const c: Record<string, number> = {};
    for (const u of plan.units) c[u.status] = (c[u.status] ?? 0) + 1;
    return c;
  });
  let commands = $derived.by(() => {
    const s = new Set<string>();
    for (const f of plan.files) for (const c of f.commands) s.add(c);
    for (const u of plan.units) for (const c of u.commands) s.add(c);
    return [...s];
  });
  let conflicts = $derived(plan.files.filter((f) => f.conflicts.length));
  let changing = $derived(plan.files.filter((f) => f.action !== "unchanged"));
  let diffs = $derived(changing.map(filePlanToDiff));
  let writable = $derived(changing.length > 0 || plan.units.some((u) => u.status === "new" || u.status === "update"));

  async function apply() {
    busy = true;
    error = "";
    try {
      report = await agentkitApply(plan.id);
      showToast("success", $t("llm.central.catalog.plan.applied", { count: String(report.installed.length) }));
      onapplied?.(report);
    } catch (e) {
      if (errCode(e) === "AGENTKIT_PLAN_STALE" || errCode(e) === "AGENTKIT_PLAN") {
        error = $t("llm.central.catalog.plan.stale");
      } else error = errText(e);
    } finally {
      busy = false;
    }
  }

  async function undo() {
    if (!report) return;
    busy = true;
    try {
      await agentkitRestore(report.tx);
      restored = true;
      showToast("success", $t("llm.central.catalog.plan.restored"));
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape" && !busy) onclose?.();
  }

  $effect(() => {
    dialog?.focus();
  });

  let autoDone = false;
  $effect(() => {
    if (autoApply && !autoDone) {
      autoDone = true;
      if (writable) void apply();
    }
  });
</script>

<svelte:window onkeydown={onkey} />

<div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && !busy && onclose?.()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="plan-title" tabindex="-1" bind:this={dialog}>
    <header class="head">
      <div>
        <h2 id="plan-title">{report ? $t("llm.central.catalog.plan.done_title") : $t("llm.central.catalog.plan.title")}</h2>
        <p class="sub">
          {$t(`llm.central.catalog.scope.${plan.scope}`)}{plan.project_dir ? ` · ${plan.project_dir}` : ""} ·
          {$t(`llm.central.agentkit.policy.${plan.policy}`)}
        </p>
      </div>
      <button type="button" class="x" onclick={() => onclose?.()} aria-label={$t("llm.central.catalog.close")} disabled={busy}>✕</button>
    </header>

    <div class="body">
      {#if report}
        <section class="result">
          <p class="big">{$t("llm.central.catalog.plan.applied", { count: String(report.installed.length) })}</p>
          <p class="muted">{$t("llm.central.catalog.plan.written", { count: String(report.files_written.length) })} · {$t("llm.central.catalog.plan.unchanged", { count: String(report.unchanged.length) })}</p>
          {#if report.skipped_units.length}
            <p class="muted">{$t("llm.central.catalog.plan.skipped", { count: String(report.skipped_units.length) })}</p>
          {/if}
          {#if report.notes.length}
            <ul class="notes">{#each report.notes as n, i (i)}<li>{n}</li>{/each}</ul>
          {/if}
          <details>
            <summary>{$t("llm.central.catalog.plan.files_written")}</summary>
            <ul class="paths">{#each report.files_written as p (p)}<li class="mono">{p}</li>{/each}</ul>
          </details>
          <p class="tx mono">tx {report.tx}</p>
        </section>
      {:else}
        <div class="summary">
          {#each Object.entries(counts) as [s, n] (s)}
            <span class="pill {s}">{$t(`llm.central.agentkit.status.${s}`)} · {n}</span>
          {/each}
          <span class="pill">{$t("llm.central.catalog.plan.files", { count: String(changing.length) })}</span>
        </div>

        {#if plan.warnings.length}
          <ul class="warn-list">{#each plan.warnings as w, i (i)}<li>{w}</li>{/each}</ul>
        {/if}

        {#if commands.length}
          <section class="commands" aria-label={$t("llm.central.catalog.plan.commands")}>
            <h3>{$t("llm.central.catalog.plan.commands")}</h3>
            <p class="muted">{$t("llm.central.catalog.plan.commands_hint")}</p>
            <ul>{#each commands as c (c)}<li><code>{c}</code></li>{/each}</ul>
          </section>
        {/if}

        {#if conflicts.length}
          <section class="conflicts">
            <h3>{$t("llm.central.catalog.plan.conflicts")}</h3>
            <ul>
              {#each conflicts as f (f.path)}
                <li><span class="mono">{f.path}</span> — {f.conflicts.join("; ")}</li>
              {/each}
            </ul>
          </section>
        {/if}

        <div class="tabs" role="tablist">
          <button type="button" role="tab" aria-selected={tab === "units"} onclick={() => (tab = "units")}>{$t("llm.central.catalog.plan.by_tool")}</button>
          <button type="button" role="tab" aria-selected={tab === "files"} onclick={() => (tab = "files")}>{$t("llm.central.catalog.plan.diff")} ({changing.length})</button>
        </div>

        {#if tab === "units"}
          {#each byTarget as [tid, g] (tid)}
            <section class="target">
              <h3>{g.name}</h3>
              <ul class="units">
                {#each g.units as u (u.unit_id)}
                  <li class="unit">
                    <div class="row">
                      <span class="pill {u.status}">{$t(`llm.central.agentkit.status.${u.status}`)}</span>
                      <span class="cname">{u.component.name}</span>
                      {#if u.install_name && u.install_name !== u.component.name}
                        <span class="muted">→ {u.install_name}</span>
                      {/if}
                      <span class="compat {u.compat.status}">{$t(`llm.central.agentkit.compat.${u.compat.status}`)}</span>
                    </div>
                    {#if u.error}<p class="err">{u.error}</p>{/if}
                    {#if u.compat.status === "unsupported"}<p class="muted">{u.compat.reason}</p>{/if}
                    {#if u.losses.length}
                      <p class="loss">{$t("llm.central.catalog.plan.lost")}: {u.losses.join("; ")}</p>
                    {/if}
                    {#if u.notes.length}<p class="muted">{u.notes.join(" · ")}</p>{/if}
                    {#if u.files.length}
                      <ul class="paths">
                        {#each u.files as f, i (i)}<li><span class="lbl">{f.label || f.action}</span> <span class="mono">{f.path}</span></li>{/each}
                      </ul>
                    {/if}
                  </li>
                {/each}
              </ul>
            </section>
          {/each}
        {:else}
          <div class="diff">
            <DiffViewer files={diffs} height="52vh" emptyText={$t("llm.central.catalog.plan.nothing")} />
          </div>
        {/if}
      {/if}
      {#if error}<p class="err" role="alert">{error}</p>{/if}
    </div>

    <footer class="foot">
      {#if report}
        {#if !restored}
          <button type="button" class="btn" onclick={undo} disabled={busy}>{$t("llm.central.catalog.plan.undo")}</button>
        {/if}
        <button type="button" class="btn primary" onclick={() => onclose?.()}>{$t("llm.central.catalog.done")}</button>
      {:else}
        {#if error && onreplan}
          <button type="button" class="btn" onclick={() => onreplan?.()} disabled={busy}>{$t("llm.central.catalog.plan.replan")}</button>
        {/if}
        <button type="button" class="btn" onclick={() => onclose?.()} disabled={busy}>{$t("llm.central.catalog.cancel")}</button>
        <button type="button" class="btn primary" onclick={apply} disabled={busy || !writable}>
          {busy ? $t("llm.central.catalog.plan.installing") : $t("llm.central.catalog.plan.install")}
        </button>
      {/if}
    </footer>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--dialog-backdrop);
  }
  .dialog {
    width: min(980px, 95vw);
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    background: var(--popup-bg);
    border-radius: var(--radius-xl);
    box-shadow: var(--elev-3);
    outline: none;
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5) var(--space-2);
  }
  h2 {
    margin: 0;
    font-size: var(--text-lg);
  }
  h3 {
    margin: 0 0 var(--space-1);
    font-size: var(--text-md);
  }
  .sub,
  .muted {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .x {
    border: none;
    background: var(--fill-1);
    width: 28px;
    height: 28px;
    border-radius: var(--radius-full);
    color: var(--text);
    cursor: pointer;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--space-2) var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .summary {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .pill {
    padding: 2px 8px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    font-size: var(--text-xs);
    white-space: nowrap;
  }
  .pill.new {
    background: color-mix(in srgb, var(--success) 18%, transparent);
    color: var(--success);
  }
  .pill.update {
    background: color-mix(in srgb, var(--blue) 18%, transparent);
    color: var(--blue);
  }
  .pill.conflict,
  .pill.unsupported {
    background: color-mix(in srgb, var(--warning) 18%, transparent);
    color: var(--warning);
  }
  .pill.error {
    background: color-mix(in srgb, var(--error) 18%, transparent);
    color: var(--error);
  }
  .warn-list,
  .notes {
    margin: 0;
    padding-left: var(--space-4);
    font-size: var(--text-sm);
    color: var(--warning);
  }
  .commands {
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--warning) 10%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--warning) 45%, transparent);
  }
  .commands ul {
    margin: var(--space-2) 0 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .commands code {
    display: block;
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    background: var(--surface-mut);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    white-space: pre-wrap;
    word-break: break-all;
  }
  .conflicts {
    font-size: var(--text-sm);
  }
  .conflicts ul {
    margin: 0;
    padding-left: var(--space-4);
  }
  .tabs {
    display: flex;
    gap: var(--space-1);
    border-bottom: 1px solid var(--separator);
  }
  .tabs button {
    padding: 6px 10px;
    border: none;
    background: none;
    color: var(--text-muted);
    font-size: var(--text-sm);
    border-bottom: 2px solid transparent;
    cursor: pointer;
  }
  .tabs button[aria-selected="true"] {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .units {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .unit {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .cname {
    font-weight: 600;
  }
  .compat {
    margin-left: auto;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .compat.native {
    color: var(--success);
  }
  .compat.converted {
    color: var(--blue);
  }
  .compat.degraded {
    color: var(--warning);
  }
  .loss {
    margin: 4px 0 0;
    font-size: var(--text-sm);
    color: var(--warning);
  }
  .paths {
    margin: 4px 0 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .paths .lbl {
    display: inline-block;
    min-width: 80px;
    color: var(--text-faint);
  }
  .mono {
    font-family: var(--font-mono);
    word-break: break-all;
  }
  .err {
    margin: 4px 0 0;
    color: var(--error);
    font-size: var(--text-sm);
  }
  .diff {
    min-height: 200px;
  }
  .result .big {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 600;
  }
  .tx {
    font-size: var(--text-xs);
    color: var(--text-faint);
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5) var(--space-4);
    border-top: 1px solid var(--separator);
  }
  .btn {
    height: var(--control-h-lg);
    padding: 0 var(--space-4);
    border: none;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-base);
    font-weight: 500;
    cursor: pointer;
  }
  .btn:hover:not(:disabled) {
    background: var(--fill-2);
  }
  .btn.primary {
    background: var(--cta);
    color: var(--on-cta);
  }
  .btn.primary:hover:not(:disabled) {
    background: var(--cta-hover);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
