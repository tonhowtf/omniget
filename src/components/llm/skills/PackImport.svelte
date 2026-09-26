<script lang="ts">
  /**
   * Selective import of a pack folder (skills, agents, rules, context,
   * commands). Nothing is selected by default; the plan shows what each item
   * would do before anything is written; a conflict is overwritten only
   * when its own box is ticked. Importing is text only: hooks and scripts
   * are listed but never installed, nothing runs, no dependency is installed.
   */
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { getAgents, loadRoster } from "$lib/stores/llm-store.svelte";
  import { loadSkills } from "$lib/stores/llm-skills-store.svelte";
  import { connectionAgents } from "$lib/stores/assist-bots-store.svelte";
  import {
    errorText, isConflict, overwriteList, packKey, packNeverInstalled, planActionTone, planHasWork, shortTime,
    isAbsolutePath, shortPath,
    type PlanAction,
  } from "$lib/stores/assist-missions-store.svelte";

  interface CatalogItem { kind: string; name: string; key: string; rel_path: string; description: string; bytes: number }
  interface Catalog { items: CatalogItem[]; license: string | null; sha: string | null; origin: string }
  interface PlanItem {
    key: string; kind: string; name: string; dest: string; hash: string; bytes: number; license: string | null; attribution: string | null;
    deps: string[]; capabilities: string[]; action: PlanAction; reason: string | null; diff: string | null; scripts: string[];
  }
  interface ImportPlan { source_dir: string; meta: { origin: string; sha: string | null; license: string | null; attribution: string | null }; items: PlanItem[]; created_ms: number }
  interface ApplyReport { installed: string[]; updated: string[]; skipped: [string, string][]; pending_scan: [string, string][] }
  interface PackItem { id: string; kind: string; name: string; origin: string; origin_sha: string | null; license: string | null; status: string; version: number; updated_ms: number }

  let dir = $state<string | null>(null);
  let typedDir = $state("");
  let catalog = $state<Catalog | null>(null);
  let selected = $state<Record<string, boolean>>({});
  let plan = $state<ImportPlan | null>(null);
  let overwrite = $state<Record<string, boolean>>({});
  let report = $state<ApplyReport | null>(null);
  let items = $state<PackItem[]>([]);
  let busy = $state(false);
  let error = $state("");
  let confirming = $state<{ label: string; run: () => Promise<unknown> } | null>(null);
  let viewing = $state<{ id: string; text: string } | null>(null);
  let history = $state<{ id: string; rows: unknown[] } | null>(null);
  let connection = $state("");

  let connections = $derived(connectionAgents(getAgents()));
  let chosen = $derived(Object.entries(selected).filter(([, v]) => v).map(([k]) => k));
  let kinds = $derived([...new Set((catalog?.items ?? []).map((i) => i.kind))]);

  onMount(() => {
    void loadRoster();
    void loadItems();
  });

  $effect(() => {
    if (!connection && connections.length) connection = connections[0].id;
  });

  async function loadItems() {
    try {
      items = await invoke<PackItem[]>("assist_packs_items");
    } catch {
      items = [];
    }
  }

  async function act(fn: () => Promise<unknown>) {
    if (busy) return;
    busy = true;
    error = "";
    confirming = null;
    try {
      await fn();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function pick() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const r = await open({ directory: true, multiple: false });
      if (typeof r !== "string") return;
      dir = r;
    } catch {
      error = $t("assist.packs.picker_failed");
      return;
    }
    await openCatalog();
  }

  /** The same pack folder typed by hand (no native dialog). */
  async function useTyped() {
    const d = typedDir.trim();
    if (!isAbsolutePath(d)) {
      error = $t("mission.path.err_absolute");
      return;
    }
    dir = d;
    await openCatalog();
  }

  async function openCatalog() {
    catalog = null; plan = null; report = null; selected = {}; overwrite = {};
    const d = dir;
    await act(async () => {
      catalog = await invoke<Catalog>("assist_packs_catalog", { dir: d });
    });
  }

  function makePlan() {
    if (!dir || !chosen.length) return;
    const input = { dir, selection: chosen };
    void act(async () => {
      plan = await invoke<ImportPlan>("assist_packs_plan", { input });
      overwrite = {};
      report = null;
    });
  }

  function apply() {
    if (!plan) return;
    const p = $state.snapshot(plan) as ImportPlan;
    const ow = overwriteList(p.items, overwrite);
    void act(async () => {
      report = await invoke<ApplyReport>("assist_packs_apply", { plan: p, overwrite: ow });
      plan = null;
      await Promise.all([loadItems(), loadSkills(true)]);
    });
  }

  function view(it: PackItem) {
    if (viewing?.id === it.id) { viewing = null; return; }
    void act(async () => {
      const text = await invoke<string | null>("assist_packs_text", { kind: it.kind, name: it.name });
      viewing = { id: it.id, text: text ?? "" };
    });
  }

  function showHistory(it: PackItem) {
    if (history?.id === it.id) { history = null; return; }
    void act(async () => {
      history = { id: it.id, rows: await invoke<unknown[]>("assist_packs_history", { itemId: it.id }) };
    });
  }

  function rollback(it: PackItem) {
    confirming = {
      label: $t("assist.packs.confirm_rollback", { name: it.name }),
      run: async () => { await invoke("assist_packs_rollback", { itemId: it.id }); await loadItems(); },
    };
  }

  function toBot(it: PackItem) {
    if (!connection) return;
    const c = connection;
    void act(async () => {
      await invoke("assist_packs_agent_to_bot", { itemId: it.id, connectionAgentId: c });
      await loadRoster();
    });
  }

  function fmtBytes(n: number): string {
    return n < 1024 ? `${n} B` : `${(n / 1024).toFixed(1)} KB`;
  }
</script>

<section class="pack" aria-labelledby="pack-title" aria-busy={busy}>
  <h2 class="block-title" id="pack-title">{$t("assist.packs.title")}</h2>
  <p class="dim">{$t("assist.packs.lede")}</p>
  <p class="notice safe">{$t("assist.packs.safety")}</p>

  {#if confirming}
    <div class="notice confirm" role="alertdialog" aria-label={confirming.label}>
      <span>{confirming.label}</span>
      <div class="actions">
        <button type="button" class="button active" disabled={busy} onclick={() => { const c = confirming; if (c) void act(c.run); }}>{$t("mission.confirm.yes")}</button>
        <button type="button" class="button" onclick={() => (confirming = null)}>{$t("mission.cancel")}</button>
      </div>
    </div>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <div class="actions">
    <button type="button" class="button" disabled={busy} onclick={pick}>{$t("assist.packs.pick")}</button>
    {#if dir}<code class="path" title={dir}>{shortPath(dir, 64)}</code>{/if}
  </div>
  <form class="actions" onsubmit={(e) => { e.preventDefault(); void useTyped(); }}>
    <input class="input typed" type="text" bind:value={typedDir} placeholder={$t("mission.path.folder_placeholder")} aria-label={$t("assist.packs.typed_label")} autocomplete="off" spellcheck={false} />
    <button type="submit" class="button" disabled={busy || !typedDir.trim()}>{$t("assist.packs.typed_submit")}</button>
  </form>

  {#if catalog}
    <p class="dim">
      {$t("assist.packs.origin", { origin: catalog.origin })}
      {#if catalog.sha}· <code>{catalog.sha.slice(0, 10)}</code>{/if}
      · {catalog.license ? $t("assist.packs.license", { license: catalog.license }) : $t("assist.packs.no_license")}
    </p>
    {#if catalog.items.length === 0}
      <p class="dim">{$t("assist.packs.empty")}</p>
    {:else}
      <p class="dim">{$t("assist.packs.select_hint")}</p>
      {#each kinds as kind (kind)}
        <div class="field-label">{$t(`assist.packs.kind.${kind}`)}</div>
        <ul class="plain">
          {#each catalog.items.filter((i) => i.kind === kind) as it (it.key)}
            {@const key = it.key || packKey(it)}
            <li>
              <label class="check" class:off={packNeverInstalled(it.kind)}>
                <input type="checkbox" disabled={packNeverInstalled(it.kind) || busy} checked={!!selected[key]}
                  onchange={(e) => (selected[key] = e.currentTarget.checked)} />
                <strong>{it.name}</strong>
                {#if packNeverInstalled(it.kind)}<span class="tag warn">{$t("assist.packs.never_installed")}</span>{/if}
                <span class="dim">{fmtBytes(it.bytes)}</span>
              </label>
              {#if it.description}<p class="dim desc">{it.description}</p>{/if}
            </li>
          {/each}
        </ul>
      {/each}
      <div class="actions">
        <button type="button" class="button" disabled={busy || !chosen.length} onclick={makePlan}>{$t("assist.packs.plan", { count: chosen.length })}</button>
        {#if chosen.length}<button type="button" class="button" onclick={() => (selected = {})}>{$t("assist.packs.clear")}</button>{/if}
      </div>
    {/if}
  {/if}

  {#if plan}
    <div class="plan">
      <div class="field-label">{$t("assist.packs.plan_title")}</div>
      {#if plan.meta.attribution}<p class="dim">{plan.meta.attribution}</p>{/if}
      <ul class="plain">
        {#each plan.items as it (it.key)}
          <li class="plan-item">
            <div class="line">
              <span class="pill {planActionTone(it.action)}">{$t(`assist.packs.action.${it.action}`)}</span>
              <strong>{it.name}</strong>
              <span class="tag">{$t(`assist.packs.kind.${it.kind}`)}</span>
              {#if it.license}<span class="dim">{it.license}</span>{/if}
            </div>
            {#if it.reason}<p class="dim">{it.reason}</p>{/if}
            {#if it.deps.length}<p class="dim">{$t("assist.packs.deps", { deps: it.deps.join(", ") })}</p>{/if}
            {#if it.capabilities.length}<p class="dim">{$t("assist.packs.capabilities", { caps: it.capabilities.join(", ") })}</p>{/if}
            {#if it.scripts.length}<p class="dim warn-text">{$t("assist.packs.scripts_skipped", { scripts: it.scripts.join(", ") })}</p>{/if}
            {#if isConflict(it.action)}
              <label class="check"><input type="checkbox" checked={!!overwrite[it.key]} onchange={(e) => (overwrite[it.key] = e.currentTarget.checked)} /> {$t("assist.packs.overwrite")}</label>
            {/if}
            <details class="adv">
              <summary>{$t("assist.packs.details")}</summary>
              <p class="dim"><code>{it.dest}</code> · {fmtBytes(it.bytes)} · <code>{it.hash.slice(0, 12)}</code></p>
              {#if it.diff}<pre class="pre">{it.diff}</pre>{/if}
            </details>
          </li>
        {/each}
      </ul>
      <div class="actions">
        <button type="button" class="button active" disabled={busy || !planHasWork(plan.items, overwrite)} onclick={apply}>{$t("assist.packs.apply")}</button>
        <button type="button" class="button" onclick={() => (plan = null)}>{$t("mission.cancel")}</button>
      </div>
    </div>
  {/if}

  {#if report}
    <div class="notice" role="status">
      <span>{$t("assist.packs.report", { installed: report.installed.length, updated: report.updated.length, skipped: report.skipped.length })}</span>
      {#if report.skipped.length}<ul class="plain">{#each report.skipped as [k, why] (k)}<li class="dim">{k}: {why}</li>{/each}</ul>{/if}
      {#if report.pending_scan.length}<span>{$t("assist.packs.pending_scan", { count: report.pending_scan.length })}</span>{/if}
    </div>
  {/if}

  {#if items.length}
    <details class="adv">
      <summary>{$t("assist.packs.installed", { count: items.length })}</summary>
      {#if items.some((i) => i.kind === "agent") && connections.length}
        <label class="field">
          <span class="field-label">{$t("assist.packs.connection")}</span>
          <select class="input narrow" bind:value={connection}>{#each connections as a (a.id)}<option value={a.id}>{a.name}</option>{/each}</select>
        </label>
      {/if}
      <ul class="plain">
        {#each items as it (it.id)}
          <li>
            <div class="line">
              <span class="tag">{$t(`assist.packs.kind.${it.kind}`)}</span>
              <strong>{it.name}</strong>
              <span class="dim">v{it.version} · {it.status} · {it.origin} · {shortTime(it.updated_ms)}</span>
            </div>
            <div class="actions">
              <button type="button" class="button small" disabled={busy} onclick={() => view(it)}>{$t("assist.packs.view")}</button>
              <button type="button" class="button small" disabled={busy} onclick={() => showHistory(it)}>{$t("assist.packs.history")}</button>
              {#if it.version > 1}<button type="button" class="button small" disabled={busy} onclick={() => rollback(it)}>{$t("assist.packs.rollback")}</button>{/if}
              {#if it.kind === "agent"}<button type="button" class="button small" disabled={busy || !connection} onclick={() => toBot(it)}>{$t("assist.packs.to_bot")}</button>{/if}
            </div>
            {#if viewing?.id === it.id}<pre class="pre">{viewing.text}</pre>{/if}
            {#if history?.id === it.id}<pre class="pre">{JSON.stringify(history.rows, null, 2)}</pre>{/if}
          </li>
        {/each}
      </ul>
    </details>
  {/if}
</section>

<style>
  .pack { display: flex; flex-direction: column; gap: var(--space-3); margin-bottom: var(--space-5); min-width: 0; }
  .block-title { margin: 0; font-size: var(--text-md); font-weight: 600; }
  .dim { margin: 0; font-size: var(--text-sm); color: var(--text-dim); overflow-wrap: anywhere; }
  .desc { padding-left: 24px; }
  .notice { margin: 0; padding: var(--space-2) var(--space-3); border-radius: var(--radius-sm); background: var(--fill-2); font-size: var(--text-sm); display: flex; flex-direction: column; gap: var(--space-2); }
  .notice.safe { background: color-mix(in srgb, var(--green) 10%, transparent); }
  .notice.confirm { background: var(--accent-soft); }
  .error { margin: 0; font-size: var(--text-sm); color: var(--danger, var(--red)); overflow-wrap: anywhere; }
  .actions { display: flex; flex-wrap: wrap; gap: var(--space-2); align-items: center; min-width: 0; }
  .path { font-family: var(--font-mono); font-size: var(--text-xs); color: var(--text-muted); overflow-wrap: anywhere; min-width: 0; max-width: 480px; }
  .typed { flex: 1 1 240px; min-width: 0; max-width: 560px; font-family: var(--font-mono); font-size: var(--text-xs); }
  .plain { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-2); }
  .check { display: inline-flex; gap: 8px; align-items: center; font-size: var(--text-sm); }
  .check.off { color: var(--text-dim); }
  .field { display: flex; flex-direction: column; gap: 4px; margin: var(--space-2) 0; }
  .narrow { max-width: 320px; }
  .plan { display: flex; flex-direction: column; gap: var(--space-2); padding: var(--space-3); border: var(--hairline) solid var(--separator); border-radius: var(--radius-md); }
  .plan-item { display: flex; flex-direction: column; gap: 4px; }
  .line { display: flex; flex-wrap: wrap; gap: 6px var(--space-2); align-items: center; font-size: var(--text-sm); }
  .warn-text { color: var(--orange); }
  .tag { padding: 0 6px; border-radius: 999px; font-size: var(--text-xs); color: var(--text-muted); background: var(--fill-2); }
  .tag.warn { color: var(--orange); background: color-mix(in srgb, var(--orange) 14%, transparent); }
  .pill { display: inline-flex; padding: 1px 8px; border-radius: 999px; font-size: var(--text-xs); font-weight: 600; color: var(--text-muted); background: var(--fill-2); }
  .pill.blue { color: var(--blue); background: color-mix(in srgb, var(--blue) 14%, transparent); }
  .pill.orange { color: var(--orange); background: color-mix(in srgb, var(--orange) 16%, transparent); }
  .pill.green { color: var(--green); background: color-mix(in srgb, var(--green) 14%, transparent); }
  .pill.red { color: var(--red); background: color-mix(in srgb, var(--red) 14%, transparent); }
  .pre { margin: 4px 0 0; padding: var(--space-2); max-height: 260px; overflow: auto; border-radius: var(--radius-sm); background: var(--fill-2); font-family: var(--font-mono); font-size: var(--text-xs); white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
  .adv summary { cursor: pointer; font-size: var(--text-sm); color: var(--accent-text); }
  summary:focus-visible, .button:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
</style>
