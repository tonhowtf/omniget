<script lang="ts">
  /**
   * Instalados (`/llm/installed`): what the OmniGet installed, by tool or by
   * project (locks: global + every project it knows), drift (files edited
   * since), update (re-plan from the catalog: the dialog shows the diff
   * against what is on disk now), uninstall (only our files/pieces), restore
   * a transaction, copy an install to another tool, and import what a tool
   * already has that did not come from the OmniGet. The Plugins tab lists
   * the tools' own plugins/extensions with an on/off switch.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import KindIcon from "$components/central/catalog/KindIcon.svelte";
  import PlanDialog from "$components/central/catalog/PlanDialog.svelte";
  import ProjectPicker from "$components/central/catalog/ProjectPicker.svelte";
  import ToolDot from "$components/central/catalog/ToolDot.svelte";
  import PluginsPanel from "$components/central/catalog/PluginsPanel.svelte";
  import {
    agentkitDetect,
    agentkitDrift,
    agentkitImportInstalled,
    agentkitInstalled,
    agentkitPlan,
    agentkitRestore,
    agentkitTargets,
    agentkitTransactions,
    agentkitUninstall,
    baseName,
    errCode,
    errText,
    itemHref,
    type Component,
    type DetectedTarget,
    type DriftItem,
    type InstallPlan,
    type InstallRecord,
    type Scope,
    type TargetAdapter,
    type TxManifest,
  } from "$lib/central/catalog";
  import { stack } from "$lib/central/stack-store.svelte";

  let projectDir = $state<string | null>(stack.projectDir);
  let view = $state<"installs" | "plugins">("installs");
  let groupBy = $state<"tool" | "project">("tool");
  let records = $state<InstallRecord[]>([]);
  let drift = $state<DriftItem[]>([]);
  let txs = $state<TxManifest[]>([]);
  let targets = $state<TargetAdapter[]>([]);
  let detected = $state<DetectedTarget[]>([]);
  let loading = $state(false);
  let error = $state("");
  let busy = $state<string | null>(null);
  let confirm = $state<{ rec: InstallRecord; drift: string[] } | null>(null);
  let plan = $state<InstallPlan | null>(null);
  let replay = $state<(() => Promise<void>) | null>(null);
  let openDrift = $state<Set<string>>(new Set());
  let copyFor = $state<string | null>(null);

  // import (what a tool has that we did not install)
  let importTool = $state<string>("");
  let importScope = $state<Scope>("project");
  let imported = $state<Component[] | null>(null);
  let importing = $state(false);
  let importCopyFor = $state<string | null>(null);

  let driftBy = $derived.by(() => {
    const m = new Map<string, DriftItem[]>();
    for (const d of drift) m.set(d.install_id, [...(m.get(d.install_id) ?? []), d]);
    return m;
  });
  let nameOf = (id: string) => targets.find((x) => x.id === id)?.name ?? id;
  let groups = $derived.by(() => {
    const m = new Map<string, InstallRecord[]>();
    for (const r of records) {
      const key = groupBy === "tool" ? r.target : r.scope === "global" ? "" : (r.project_dir ?? "");
      m.set(key, [...(m.get(key) ?? []), r]);
    }
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  });
  let installedTools = $derived(detected.filter((d) => d.installed));
  let driftCount = $derived(new Set(drift.map((d) => d.install_id)).size);

  async function load() {
    loading = true;
    error = "";
    try {
      const [r, d, x] = await Promise.all([agentkitInstalled(projectDir), agentkitDrift(projectDir), agentkitTransactions()]);
      records = r;
      drift = d;
      txs = x;
    } catch (e) {
      error = errText(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    agentkitTargets().then((x) => (targets = x)).catch(() => {});
  });

  $effect(() => {
    void projectDir;
    void load();
    agentkitDetect(projectDir, true)
      .then((d) => {
        detected = d;
        if (!importTool) importTool = d.find((x) => x.installed)?.id ?? "";
      })
      .catch(() => {});
    imported = null;
  });

  function isCatalogId(id: string): boolean {
    return !id.startsWith("installed:") && !id.startsWith("path:");
  }

  async function doUninstall(rec: InstallRecord, force: boolean): Promise<boolean> {
    busy = rec.install_id;
    confirm = null;
    try {
      const r = await agentkitUninstall(rec.install_id, rec.project_dir ?? projectDir, force);
      if (r.kept.length) showToast("info", $t("llm.central.installed.kept", { count: String(r.kept.length) }));
      else showToast("success", $t("llm.central.installed.removed", { count: String(r.removed.length) }));
      await load();
      return true;
    } catch (e) {
      showToast("error", errText(e));
      return false;
    } finally {
      busy = null;
    }
  }

  /** Discard local edits: force-uninstall, then plan the catalog version again. */
  async function reinstall(rec: InstallRecord) {
    if (!(await doUninstall(rec, true))) return;
    await planFor(() =>
      agentkitPlan({
        catalogIds: [rec.component.id],
        targets: [rec.target],
        scope: rec.scope,
        projectDir: rec.project_dir ?? null,
      }),
    );
  }

  async function planFor(fn: () => Promise<InstallPlan>) {
    try {
      plan = await fn();
      replay = async () => {
        plan = await fn();
      };
    } catch (e) {
      const code = errCode(e);
      showToast("error", code === "CATALOG_NOT_FOUND" ? $t("llm.central.installed.not_in_catalog") : errText(e));
    }
  }

  function update(rec: InstallRecord) {
    busy = rec.install_id;
    void planFor(() =>
      agentkitPlan({
        catalogIds: [rec.component.id],
        targets: [rec.target],
        scope: rec.scope,
        projectDir: rec.project_dir ?? null,
        policy: "overwrite",
      }),
    ).finally(() => (busy = null));
  }

  async function componentOf(rec: InstallRecord): Promise<Component | null> {
    const list = await agentkitImportInstalled(rec.target, rec.scope, rec.project_dir ?? null);
    return (
      list.find((c) => c.kind === rec.component.kind && c.name === rec.installed_name) ??
      list.find((c) => c.kind === rec.component.kind && c.name === rec.component.name) ??
      null
    );
  }

  function copyTo(rec: InstallRecord, target: string) {
    copyFor = null;
    busy = rec.install_id;
    void planFor(async () => {
      if (isCatalogId(rec.component.id)) {
        return agentkitPlan({
          catalogIds: [rec.component.id],
          targets: [target],
          scope: rec.scope,
          projectDir: rec.project_dir ?? null,
        });
      }
      const c = await componentOf(rec);
      if (!c) throw new Error(`AGENTKIT_IMPORT: ${rec.installed_name}`);
      return agentkitPlan({ components: [c], targets: [target], scope: rec.scope, projectDir: rec.project_dir ?? null });
    }).finally(() => (busy = null));
  }

  async function restore(tx: string) {
    busy = tx;
    try {
      const r = await agentkitRestore(tx);
      showToast("success", $t("llm.central.installed.restored", { count: String(r.restored.length + r.removed.length) }));
      await load();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }

  async function runImport() {
    if (!importTool) return;
    importing = true;
    try {
      const scope = importScope === "global" || !projectDir ? "global" : importScope;
      const list = await agentkitImportInstalled(importTool, scope, scope === "global" ? null : projectDir);
      const ours = new Set(records.filter((r) => r.target === importTool).map((r) => `${r.component.kind}:${r.installed_name}`));
      imported = list.filter((c) => !ours.has(`${c.kind}:${c.name}`));
    } catch (e) {
      showToast("error", errText(e));
      imported = [];
    } finally {
      importing = false;
    }
  }

  function importCopy(c: Component, target: string) {
    importCopyFor = null;
    const scope: Scope = importScope === "global" || !projectDir ? "global" : importScope;
    void planFor(() => agentkitPlan({ components: [c], targets: [target], scope, projectDir: scope === "global" ? null : projectDir }));
  }

  function importToStack(c: Component) {
    stack.add({ id: c.id, kind: c.kind, name: c.name, category: c.category ?? undefined, description: c.description, component: c });
    showToast("success", $t("llm.central.installed.to_stack", { name: c.name }));
  }

  function toggleDrift(id: string) {
    const s = new Set(openDrift);
    if (s.has(id)) s.delete(id);
    else s.add(id);
    openDrift = s;
  }

  function when(iso: string): string {
    const d = new Date(iso);
    return isNaN(d.getTime()) ? iso : d.toLocaleString();
  }

  function txWhen(tx: TxManifest): string {
    if (tx.started_at) return when(tx.started_at);
    const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})/.exec(tx.tx);
    return m ? when(`${m[1]}-${m[2]}-${m[3]}T${m[4]}:${m[5]}:${m[6]}Z`) : tx.tx;
  }
</script>

<div class="installed">
  <div class="scroll">
    <header class="top">
      <div>
        <h1>{$t("llm.central.installed.title")}</h1>
        <p class="muted">{$t("llm.central.installed.subtitle")}</p>
      </div>
      <div class="controls">
        <div class="proj">
          <ProjectPicker value={projectDir} allowNone onchange={(v) => (projectDir = v)} />
        </div>
        {#if view === "installs"}
          <div class="seg" role="radiogroup" aria-label={$t("llm.central.installed.group_by")}>
            <button type="button" role="radio" aria-checked={groupBy === "tool"} onclick={() => (groupBy = "tool")}>{$t("llm.central.installed.by_tool")}</button>
            <button type="button" role="radio" aria-checked={groupBy === "project"} onclick={() => (groupBy = "project")}>{$t("llm.central.installed.by_project")}</button>
          </div>
          <button type="button" class="btn" onclick={load} disabled={loading}>{$t("llm.central.installed.refresh")}</button>
        {/if}
      </div>
    </header>

    <div class="tabs" role="tablist" aria-label={$t("llm.central.installed.title")}>
      <button type="button" role="tab" aria-selected={view === "installs"} onclick={() => (view = "installs")}>{$t("llm.central.plugins.tab_installs")}</button>
      <button type="button" role="tab" aria-selected={view === "plugins"} onclick={() => (view = "plugins")}>{$t("llm.central.plugins.tab")}</button>
    </div>

    {#if view === "plugins"}
      <PluginsPanel {projectDir} toolName={nameOf} onchanged={load} />
    {:else}
    {#if error}<p class="err" role="alert">{error}</p>{/if}

    <div class="stats">
      <span class="stat"><strong class="tabular">{records.length}</strong> {$t("llm.central.installed.installs")}</span>
      <span class="stat" class:warn={driftCount > 0}><strong class="tabular">{driftCount}</strong> {$t("llm.central.installed.drifted")}</span>
      <span class="stat"><strong class="tabular">{txs.length}</strong> {$t("llm.central.installed.transactions")}</span>
    </div>

    {#if !loading && !records.length}
      <div class="empty">
        <KindIcon kind="all" size={48} />
        <p>{$t("llm.central.installed.empty")}</p>
        <a class="btn primary" href="/llm/catalog">{$t("llm.central.installed.open_catalog")}</a>
      </div>
    {/if}

    {#each groups as [key, list] (key)}
      <section class="group">
        <h2>
          {#if groupBy === "tool"}
            <ToolDot id={key} name={nameOf(key)} compat={{ status: "native" }} size={22} />
            {nameOf(key)}
          {:else}
            {key ? baseName(key) : $t("llm.central.catalog.scope.global")}
            {#if key}<span class="muted mono small">{key}</span>{/if}
          {/if}
          <span class="n tabular">{list.length}</span>
        </h2>
        <ul class="recs">
          {#each list as r (r.install_id)}
            {@const d = driftBy.get(r.install_id) ?? []}
            <li class="rec">
              <div class="row">
                <KindIcon kind={r.component.kind} size={28} />
                <div class="info">
                  <p class="name">
                    {#if isCatalogId(r.component.id)}<a href={itemHref(r.component.id)}>{r.installed_name}</a>{:else}{r.installed_name}{/if}
                    {#if r.installed_name !== r.component.name}<span class="muted small">({r.component.name})</span>{/if}
                  </p>
                  <p class="sub">
                    {$t(`llm.central.catalog.kind.${r.component.kind}`)} ·
                    {groupBy === "tool" ? (r.project_dir ? baseName(r.project_dir) : $t("llm.central.catalog.scope.global")) : nameOf(r.target)} ·
                    {$t(`llm.central.catalog.scope.${r.scope}`)} ·
                    <span class="compat {r.compat.status}">{$t(`llm.central.agentkit.compat.${r.compat.status}`)}</span> ·
                    {when(r.installed_at)}
                  </p>
                </div>
                {#if d.length}
                  <button type="button" class="drift" aria-expanded={openDrift.has(r.install_id)} onclick={() => toggleDrift(r.install_id)}>
                    {$t("llm.central.installed.drift", { count: String(d.length) })}
                  </button>
                {/if}
                <div class="acts">
                  {#if isCatalogId(r.component.id)}
                    <button type="button" class="btn" onclick={() => update(r)} disabled={busy === r.install_id}>{$t("llm.central.installed.update")}</button>
                  {/if}
                  <div class="copy">
                    <button type="button" class="btn" aria-expanded={copyFor === r.install_id} onclick={() => (copyFor = copyFor === r.install_id ? null : r.install_id)} disabled={busy === r.install_id}>
                      {$t("llm.central.installed.copy_to")}
                    </button>
                    {#if copyFor === r.install_id}
                      <div class="menu" role="menu">
                        {#each installedTools.filter((x) => x.id !== r.target) as tg (tg.id)}
                          <button type="button" role="menuitem" onclick={() => copyTo(r, tg.id)}>{tg.name}</button>
                        {/each}
                        {#if !installedTools.filter((x) => x.id !== r.target).length}<p class="muted small">{$t("llm.central.installed.no_other_tool")}</p>{/if}
                      </div>
                    {/if}
                  </div>
                  <button type="button" class="btn danger" onclick={() => (confirm = { rec: r, drift: d.map((x) => x.path) })} disabled={busy === r.install_id}>
                    {$t("llm.central.installed.uninstall")}
                  </button>
                </div>
              </div>
              {#if openDrift.has(r.install_id)}
                <ul class="drift-list">
                  {#each d as x, i (i)}
                    <li>
                      <span class="state {x.state}">{$t(`llm.central.installed.state.${x.state}`)}</span>
                      <span class="mono small">{x.path}</span>
                      {#if x.detail.length}<pre>{x.detail.join("\n")}</pre>{/if}
                    </li>
                  {/each}
                  {#if isCatalogId(r.component.id)}
                    <li class="muted small">
                      {$t("llm.central.installed.drift_hint")}
                      <button type="button" class="btn" onclick={() => reinstall(r)} disabled={busy === r.install_id}>{$t("llm.central.installed.reinstall")}</button>
                    </li>
                  {/if}
                </ul>
              {/if}
              {#if confirm && confirm.rec.install_id === r.install_id}
                <div class="confirm" role="alertdialog" aria-label={$t("llm.central.installed.uninstall")}>
                  <p>{$t("llm.central.installed.confirm", { name: r.installed_name, tool: nameOf(r.target) })}</p>
                  {#if confirm.drift.length}
                    <p class="small">{$t("llm.central.installed.confirm_drift")}</p>
                    <ul class="mono small">{#each confirm.drift as p (p)}<li>{p}</li>{/each}</ul>
                  {/if}
                  <div class="confirm-acts">
                    <button type="button" class="btn" onclick={() => (confirm = null)}>{$t("llm.central.catalog.cancel")}</button>
                    {#if confirm.drift.length}
                      <button type="button" class="btn danger" onclick={() => doUninstall(r, true)}>{$t("llm.central.installed.force_uninstall")}</button>
                    {/if}
                    <button type="button" class="btn danger" onclick={() => doUninstall(r, false)}>
                      {confirm.drift.length ? $t("llm.central.installed.uninstall_keep") : $t("llm.central.installed.uninstall")}
                    </button>
                  </div>
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      </section>
    {/each}

    <section class="group import">
      <h2>{$t("llm.central.installed.import_title")}</h2>
      <p class="muted small">{$t("llm.central.installed.import_hint")}</p>
      <div class="import-row">
        <select bind:value={importTool} aria-label={$t("llm.central.installed.import_tool")}>
          {#each installedTools as tg (tg.id)}<option value={tg.id}>{tg.name}</option>{/each}
        </select>
        <select bind:value={importScope} aria-label={$t("llm.central.catalog.stack.scope")}>
          <option value="project" disabled={!projectDir}>{$t("llm.central.catalog.scope.project")}</option>
          <option value="global">{$t("llm.central.catalog.scope.global")}</option>
        </select>
        <button type="button" class="btn primary" onclick={runImport} disabled={!importTool || importing}>
          {importing ? $t("llm.central.catalog.loading") : $t("llm.central.installed.import_scan")}
        </button>
      </div>
      {#if imported}
        {#if !imported.length}
          <p class="muted small">{$t("llm.central.installed.import_none")}</p>
        {:else}
          <ul class="recs">
            {#each imported as c (c.id)}
              <li class="rec">
                <div class="row">
                  <KindIcon kind={c.kind} size={28} />
                  <div class="info">
                    <p class="name">{c.name}</p>
                    <p class="sub">{$t(`llm.central.catalog.kind.${c.kind}`)}{c.description ? ` · ${c.description}` : ""}</p>
                  </div>
                  <div class="acts">
                    <button type="button" class="btn" onclick={() => importToStack(c)} disabled={stack.has(c.id)}>
                      {stack.has(c.id) ? $t("llm.central.catalog.stack.in_stack") : $t("llm.central.catalog.stack.add")}
                    </button>
                    <div class="copy">
                      <button type="button" class="btn" aria-expanded={importCopyFor === c.id} onclick={() => (importCopyFor = importCopyFor === c.id ? null : c.id)}>
                        {$t("llm.central.installed.copy_to")}
                      </button>
                      {#if importCopyFor === c.id}
                        <div class="menu" role="menu">
                          {#each installedTools.filter((x) => x.id !== importTool) as tg (tg.id)}
                            <button type="button" role="menuitem" onclick={() => importCopy(c, tg.id)}>{tg.name}</button>
                          {/each}
                        </div>
                      {/if}
                    </div>
                  </div>
                </div>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
    </section>

    {#if txs.length}
      <section class="group">
        <h2>{$t("llm.central.installed.history")}</h2>
        <ul class="txs">
          {#each txs.slice(0, 30) as x (x.tx)}
            <li>
              <span class="pill">{x.kind}</span>
              <span class="small">{txWhen(x)}</span>
              <span class="muted small">{$t("llm.central.installed.tx_files", { count: String(x.entries.length) })}</span>
              <span class="mono small muted">{x.tx}</span>
              <button type="button" class="btn" onclick={() => restore(x.tx)} disabled={busy === x.tx}>{$t("llm.central.installed.restore")}</button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}
    {/if}
  </div>
</div>

{#if plan}
  <PlanDialog
    {plan}
    onclose={() => {
      plan = null;
      void load();
    }}
    onreplan={() => {
      const r = replay;
      plan = null;
      if (r) void r().catch((e) => showToast("error", errText(e)));
    }}
  />
{/if}

<style>
  .installed {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .scroll {
    flex: 1;
    overflow: auto;
    padding: var(--space-4) var(--space-5) var(--space-8);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .top {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  h1 {
    margin: 0;
    font-size: var(--text-xl);
    letter-spacing: var(--track-tight);
  }
  .controls {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .proj {
    width: 280px;
  }
  .seg {
    display: inline-flex;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .seg button {
    height: 24px;
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .seg button[aria-checked="true"] {
    background: var(--surface-hi);
    color: var(--text);
    box-shadow: var(--elev-1);
  }
  .tabs {
    display: flex;
    gap: var(--space-1);
    border-bottom: var(--hairline) solid var(--separator);
  }
  .tabs button {
    height: 30px;
    padding: 0 var(--space-3);
    border: none;
    border-bottom: 2px solid transparent;
    background: none;
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
  }
  .tabs button[aria-selected="true"] {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .stats {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .stat {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .stat strong {
    font-size: var(--text-lg);
    color: var(--text);
    margin-right: 4px;
  }
  .stat.warn strong {
    color: var(--warning);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-7) 0;
    color: var(--text-muted);
  }
  .group h2 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 var(--space-2);
    font-size: var(--text-md);
  }
  .n {
    font-size: var(--text-xs);
    color: var(--text-faint);
    font-weight: 400;
  }
  .recs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .rec {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .info {
    flex: 1;
    min-width: 200px;
  }
  .name {
    margin: 0;
    font-weight: 600;
  }
  .name a {
    color: var(--text);
    text-decoration: none;
  }
  .name a:hover {
    text-decoration: underline;
  }
  .sub {
    margin: 2px 0 0;
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
  .drift {
    height: 24px;
    padding: 0 10px;
    border: none;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--warning) 18%, transparent);
    color: var(--warning);
    font-size: var(--text-xs);
    font-weight: 600;
    cursor: pointer;
  }
  .acts {
    display: flex;
    gap: var(--space-1);
    align-items: center;
  }
  .copy {
    position: relative;
  }
  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    z-index: 10;
    min-width: 180px;
    max-height: 260px;
    overflow: auto;
    padding: 4px;
    border-radius: var(--radius-md);
    background: var(--material-thick);
    backdrop-filter: var(--material-blur);
    box-shadow: var(--elev-2);
    display: flex;
    flex-direction: column;
  }
  .menu button {
    padding: 6px var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text);
    font-size: var(--text-sm);
    text-align: left;
    cursor: pointer;
  }
  .menu button:hover,
  .menu button:focus-visible {
    background: var(--accent-soft);
  }
  .drift-list {
    list-style: none;
    margin: var(--space-2) 0 0 40px;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .drift-list pre {
    margin: 2px 0 0;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--surface-mut);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    white-space: pre-wrap;
  }
  .state {
    display: inline-block;
    min-width: 72px;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--warning);
  }
  .state.missing {
    color: var(--error);
  }
  .confirm {
    margin-top: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--error) 8%, transparent);
    font-size: var(--text-sm);
  }
  .confirm p {
    margin: 0 0 var(--space-1);
  }
  .confirm ul {
    margin: 0 0 var(--space-2);
    padding-left: var(--space-4);
  }
  .confirm-acts {
    display: flex;
    gap: var(--space-2);
    justify-content: flex-end;
  }
  .import-row {
    display: flex;
    gap: var(--space-2);
    margin: var(--space-2) 0;
    flex-wrap: wrap;
  }
  select {
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .txs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .txs li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px var(--space-2);
    border-radius: var(--radius-sm);
  }
  .txs li:hover {
    background: var(--fill-1);
  }
  .txs .mono {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pill {
    padding: 1px 8px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    font-size: var(--text-xs);
  }
  .btn {
    display: inline-flex;
    align-items: center;
    height: var(--control-h);
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-sm);
    text-decoration: none;
    cursor: pointer;
    white-space: nowrap;
  }
  .btn:hover:not(:disabled) {
    background: var(--fill-2);
  }
  .btn.primary {
    background: var(--cta);
    color: var(--on-cta);
  }
  .btn.danger {
    color: var(--error);
  }
  .confirm .btn.danger {
    background: var(--error);
    color: var(--on-error);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: var(--text-xs);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .err {
    color: var(--error);
  }
</style>
