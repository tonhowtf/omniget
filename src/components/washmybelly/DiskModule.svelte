<script lang="ts">
  import { moduleById } from "./registry";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { formatBytes } from "$lib/washmybelly/api";
  import { crumbs, type Health, type LargeFiles, type UsageNode, type UsageTree, type Volume } from "$lib/washmybelly/disk";
  import type { Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import SelectList, { type SelectItem } from "./ui/SelectList.svelte";
  import PlanReview from "./ui/PlanReview.svelte";
  import { onProgress } from "./ui/progress";
  import DiskVolumeCard from "./DiskVolumeCard.svelte";
  import DiskHealthCard from "./DiskHealthCard.svelte";
  import TreemapChart from "./TreemapChart.svelte";
  import SunburstChart from "./SunburstChart.svelte";

  let { caps }: { caps: Caps } = $props();

  let volumes = $state<Volume[]>([]);
  let health = $state<Health | null>(null);
  let selectedVolume = $state<string | null>(null);

  let tree = $state<UsageTree | null>(null);
  let scanning = $state(false);
  let scanCount = $state(0);
  let error = $state<string | null>(null);
  let view = $state<"treemap" | "sunburst">("treemap");
  let hovered = $state<UsageNode | null>(null);
  const DEPTH = 3;

  let large = $state<LargeFiles | null>(null);
  let largeBusy = $state(false);
  let largeCount = $state(0);
  let chosen = $state(new Set<string>());
  let reviewOpen = $state(false);

  let unlisten: (() => void)[] = [];

  onMount(async () => {
    unlisten.push(await onProgress("disk_usage", (p) => (scanCount = p.done)));
    unlisten.push(await onProgress("large_files", (p) => (largeCount = p.done)));
    api.call<{ volumes: Volume[] }>("wmb_disk_volumes").then((r) => (volumes = r.volumes)).catch((e) => (error = String(e)));
    api.call<Health>("wmb_disk_health").then((h) => (health = h)).catch((e) => (health = { disks: [], reason: String(e) }));
  });
  onDestroy(() => unlisten.forEach((u) => u()));

  async function analyze(path?: string, refresh = false) {
    if (scanning) return;
    scanning = true;
    scanCount = 0;
    error = null;
    hovered = null;
    try {
      tree = await api.call<UsageTree>("wmb_disk_usage", { ...(path ? { path } : {}), depth: DEPTH, min_fraction: 0.01, refresh });
    } catch (e) {
      error = String(e);
    } finally {
      scanning = false;
    }
  }

  function pickVolume(v: Volume) {
    selectedVolume = v.mount;
    analyze(v.mount);
  }

  function open(n: UsageNode) {
    if (n.path) analyze(n.path);
  }

  function otherLabel(n: UsageNode) {
    return $t("washmybelly.disk.other", { count: (n.count ?? 0).toLocaleString() }) as string;
  }

  function baseName(p: string) {
    const parts = p.split(/[\\/]/).filter(Boolean);
    return parts[parts.length - 1] ?? p;
  }

  let trail = $derived(tree ? crumbs(tree.root, tree.path ?? tree.root, baseName(tree.root)) : []);
  let focus = $derived(hovered ?? tree);

  let revealKey = $derived(caps.os === "macos" ? "washmybelly.disk.reveal_mac" : caps.os === "windows" ? "washmybelly.disk.reveal_win" : "washmybelly.disk.reveal_linux");
  async function reveal(path: string | undefined) {
    if (!path) return;
    try {
      await api.ui("disk.reveal", { path });
    } catch (e) {
      error = String(e);
    }
  }

  async function findLarge() {
    if (largeBusy) return;
    largeBusy = true;
    largeCount = 0;
    chosen = new Set();
    try {
      const roots = tree?.path ? [tree.path] : undefined;
      large = await api.call<LargeFiles>("wmb_find_large_files", { ...(roots ? { roots } : {}), min_mb: 100, limit: 50 });
    } catch (e) {
      error = String(e);
    } finally {
      largeBusy = false;
    }
  }

  let largeItems = $derived<SelectItem[]>(
    (large?.items ?? []).map((f) => ({ id: f.id, label: f.label, detail: f.detail, bytes: f.bytes, risk: f.risk, group: f.group })),
  );
  let chosenBytes = $derived((large?.items ?? []).filter((f) => chosen.has(f.id)).reduce((s, f) => s + f.bytes, 0));
  let singlePath = $derived(chosen.size === 1 ? large?.items.find((f) => chosen.has(f.id))?.path : undefined);

  function afterTrash() {
    const where = tree?.path;
    findLarge();
    if (where) analyze(where, true);
    api.call<{ volumes: Volume[] }>("wmb_disk_volumes").then((r) => (volumes = r.volumes)).catch(() => {});
  }
</script>

<div class="disk">
  <ModuleHeader tile={moduleById("disk")} title={$t("washmybelly.modules.disk") as string} subtitle={$t("washmybelly.disk.subtitle") as string}>
    {#snippet actions()}
      <button class="btn btn-primary btn-sm" type="button" disabled={scanning} onclick={() => { selectedVolume = null; analyze(); }}>{$t("washmybelly.disk.analyze_home")}</button>
    {/snippet}
  </ModuleHeader>

  <div class="content">
    {#if volumes.length}
      <div class="volumes">
        {#each volumes as v (v.mount)}
          <DiskVolumeCard volume={v} selected={selectedVolume === v.mount} onSelect={() => pickVolume(v)} />
        {/each}
      </div>
    {/if}

    {#if error}<p class="error" role="alert">{error}</p>{/if}

    <section class="card">
      {#if scanning}
        <div class="scanning">
          <div class="bar"><div class="fill"></div></div>
          <span>{$t("washmybelly.disk.scanning", { count: scanCount.toLocaleString() })}</span>
        </div>
      {/if}
      {#if tree}
        <div class="toolbar">
          <nav class="crumbs" aria-label={$t("washmybelly.disk.path") as string}>
            {#each trail as c, i (c.path)}
              {#if i > 0}<span class="sep" aria-hidden="true">›</span>{/if}
              {#if i === trail.length - 1}
                <span class="crumb current" aria-current="page">{c.name}</span>
              {:else}
                <button class="crumb" type="button" disabled={scanning} onclick={() => analyze(c.path)}>{c.name}</button>
              {/if}
            {/each}
          </nav>
          <div class="seg-ctl" role="tablist">
            <button type="button" role="tab" aria-selected={view === "treemap"} class:on={view === "treemap"} onclick={() => (view = "treemap")}>{$t("washmybelly.disk.treemap")}</button>
            <button type="button" role="tab" aria-selected={view === "sunburst"} class:on={view === "sunburst"} onclick={() => (view = "sunburst")}>{$t("washmybelly.disk.sunburst")}</button>
          </div>
          <button class="btn btn-ghost btn-sm" type="button" disabled={scanning} onclick={() => analyze(tree?.path, true)}>{$t("washmybelly.disk.rescan")}</button>
        </div>

        <div class="info">
          <strong>{focus?.kind === "other" ? otherLabel(focus) : focus?.name}</strong>
          <span>{formatBytes(focus?.bytes)} · {$t("washmybelly.disk.files", { count: (focus?.files ?? 0).toLocaleString() })}</span>
          {#if focus?.path}<button class="link" type="button" onclick={() => reveal(focus?.path)}>{$t(revealKey)}</button>{/if}
        </div>

        {#if (tree.children ?? []).length === 0}
          <EmptyState art="happy" title={$t("washmybelly.disk.empty_folder") as string} />
        {:else if view === "treemap"}
          <TreemapChart node={tree} {otherLabel} onOpen={open} onHover={(n) => (hovered = n)} />
        {:else}
          <SunburstChart node={tree} depth={DEPTH} {otherLabel} onOpen={open} onHover={(n) => (hovered = n)} />
        {/if}

        <p class="meta">
          {$t("washmybelly.disk.scanned", { count: tree.scanned_entries.toLocaleString() })}
          {#if tree.errors.permission_denied} · {$t("washmybelly.disk.denied", { count: tree.errors.permission_denied.toLocaleString() })}{/if}
          {#if tree.cached} · {$t("washmybelly.disk.from_memory")}{/if}
        </p>
      {:else if !scanning}
        <EmptyState art="searching" title={$t("washmybelly.disk.empty_title") as string} body={$t("washmybelly.disk.empty_body") as string} />
      {/if}
    </section>

    <section class="card">
      <div class="row-head">
        <div>
          <h3>{$t("washmybelly.disk.large_title")}</h3>
          <p class="dim">{tree?.path ? $t("washmybelly.disk.large_in", { folder: baseName(tree.path) }) : $t("washmybelly.disk.large_home")}</p>
        </div>
        <button class="btn btn-secondary btn-sm" type="button" disabled={largeBusy} onclick={findLarge}>{largeBusy ? $t("washmybelly.disk.searching", { count: largeCount.toLocaleString() }) : $t("washmybelly.disk.find_large")}</button>
      </div>
      {#if large}
        {#if large.items.length}
          <SelectList items={largeItems} bind:selected={chosen} showRisk={false} />
          <div class="actions">
            <span class="dim">{$t("washmybelly.disk.selected", { count: chosen.size, size: formatBytes(chosenBytes) })}</span>
            {#if singlePath}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(singlePath)}>{$t(revealKey)}</button>{/if}
            <button class="btn btn-primary btn-sm" type="button" disabled={!chosen.size} onclick={() => (reviewOpen = true)}>{$t("washmybelly.disk.to_trash")}</button>
          </div>
        {:else}
          <EmptyState art="happy" title={$t("washmybelly.disk.large_none") as string} />
        {/if}
      {/if}
    </section>

    <DiskHealthCard {health} />
  </div>
</div>

<PlanReview kind="large_files" itemIds={[...chosen]} bind:open={reviewOpen} onDone={afterTrash} />

<style>
  .disk { display: flex; flex-direction: column; min-height: 0; }
  .content { display: flex; flex-direction: column; gap: 16px; padding: 0 24px 24px; }
  .volumes { display: flex; flex-wrap: wrap; gap: 12px; }
  .card { border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator)); padding: 12px 16px; display: flex; flex-direction: column; gap: 10px; }
  .toolbar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .crumbs { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; flex: 1; min-width: 0; font-size: 13px; }
  .crumb { all: unset; cursor: pointer; color: var(--accent-text, var(--accent)); font-weight: 500; padding: 2px 4px; border-radius: 6px; }
  .crumb:hover:not(:disabled) { background: var(--surface-mut, var(--separator)); }
  .crumb:focus-visible { box-shadow: 0 0 0 2px var(--accent); }
  .crumb.current { color: var(--text); font-weight: 600; cursor: default; }
  .sep { color: var(--text-faint, var(--text-dim)); }
  .seg-ctl { display: inline-flex; padding: 2px; border-radius: 8px; background: var(--surface-mut, var(--separator)); }
  .seg-ctl button { all: unset; cursor: pointer; font-size: 12px; padding: 3px 10px; border-radius: 6px; color: var(--text-dim); }
  .seg-ctl button.on { background: var(--surface-hi, var(--surface)); color: var(--text); box-shadow: 0 1px 2px rgb(0 0 0 / 0.12); }
  .seg-ctl button:focus-visible { box-shadow: 0 0 0 2px var(--accent); }
  .info { display: flex; align-items: baseline; gap: 10px; font-size: 13px; min-height: 20px; flex-wrap: wrap; }
  .info span { color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .link { all: unset; cursor: pointer; font-size: 12px; color: var(--accent-text, var(--accent)); }
  .link:focus-visible { box-shadow: 0 0 0 2px var(--accent); border-radius: 4px; }
  .meta, .dim { margin: 0; font-size: 11.5px; color: var(--text-dim); }
  .scanning { display: flex; flex-direction: column; gap: 6px; font-size: 12px; color: var(--text-dim); }
  .bar { height: 4px; border-radius: 999px; background: var(--surface-mut, var(--separator)); overflow: hidden; }
  .fill { height: 100%; width: 35%; background: var(--accent); border-radius: 999px; animation: slide 1.1s ease-in-out infinite alternate; }
  @keyframes slide { from { margin-left: 0; } to { margin-left: 65%; } }
  .row-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  h3 { margin: 0; font-size: 13px; font-weight: 600; }
  .actions { display: flex; align-items: center; justify-content: flex-end; gap: 8px; }
  .actions .dim { margin-right: auto; }
  .error { margin: 0; font-size: 12px; color: var(--danger); }
  @media (prefers-reduced-motion: reduce) { .fill { animation: none; } }
</style>
