<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import type { ReceiptSummary } from "$lib/washmybelly/api";
  import { errorText, formatBytes, shortenPath } from "$lib/washmybelly/format";
  import {
    byConfidence,
    classify,
    defaultLeftovers,
    isInstalled,
    modesFor,
    planBytes,
    planIds,
    type App,
    type Classified,
    type LeftoverItem,
    type LeftoversResponse,
    type UninstallMode,
  } from "$lib/washmybelly/apps";
  import AppTile from "./AppTile.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import Icon from "./ui/Icon.svelte";
  import PlanReview from "./ui/PlanReview.svelte";
  import SelectList, { type SelectItem } from "./ui/SelectList.svelte";

  /// Right pane of Uninstall: the chosen app, what it left behind grouped by
  /// how sure we are, and the one button that builds the plan. Works for an
  /// app that is already gone too (only its leftovers).
  let {
    app,
    sourceLabel,
    reasonText,
    onDone,
    onBack,
  }: {
    app: App;
    sourceLabel: (source: string) => string;
    reasonText: (reason: string | null | undefined) => string;
    onDone?: (r: ReceiptSummary, app: App) => void;
    /// Narrow window: back to the list.
    onBack?: () => void;
  } = $props();

  let resp = $state<LeftoversResponse | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let mode = $state<UninstallMode>("uninstall");
  let chosen = $state(new Set<string>());
  let reviewOpen = $state(false);
  let ticket = 0;

  let classified = $derived<Classified>(resp ? classify(resp) : { app: [], zap: [], leftovers: [] });
  let shown = $derived(resp?.app ?? app);
  let installed = $derived(isInstalled(shown));
  let modes = $derived(modesFor(classified, installed));
  let sections = $derived(byConfidence(classified.leftovers));
  let ids = $derived(planIds(classified, mode, chosen));
  let bytes = $derived(planBytes(classified, mode, chosen));
  let running = $derived(!!resp?.extra?.running);
  // The "is open" warning already has its own card.
  let notes = $derived(
    (resp?.extra?.notes ?? []).filter(
      (n): n is string => typeof n === "string" && n.trim() !== "" && !(running && / is open\b/i.test(n)),
    ),
  );
  let cask = $derived(typeof resp?.extra?.cask === "string" ? resp.extra.cask : null);

  $effect(() => {
    void load(app.id);
  });

  async function load(id: string) {
    const my = ++ticket;
    loading = true;
    error = null;
    resp = null;
    reviewOpen = false;
    try {
      const r = await api.call<LeftoversResponse>("wmb_app_leftovers", { app_id: id });
      if (my !== ticket) return;
      resp = r;
      const c = classify(r);
      chosen = defaultLeftovers(c.leftovers);
      const m = modesFor(c, isInstalled(r.app ?? app));
      mode = m[0] ?? "reset";
    } catch (e) {
      if (my !== ticket) return;
      error = errorText(e);
    } finally {
      if (my === ticket) loading = false;
    }
  }

  function toSelect(items: LeftoverItem[]): (SelectItem & { evidence: string })[] {
    return items.map((l) => ({
      id: l.id,
      label: l.label,
      detail: l.path ?? l.detail ?? null,
      detailIsPath: !!l.path,
      bytes: l.bytes,
      risk: l.risk,
      group: l.group,
      evidence: l.evidence,
    }));
  }

  function groupLabel(key: string): string {
    const k = `washmybelly.uninstall.group.${key}`;
    const s = $t(k) as string;
    if (s !== k) return s;
    const plain = key.replace(/_/g, " ");
    return plain.charAt(0).toUpperCase() + plain.slice(1);
  }

  function note(n: string): string {
    if (!n.startsWith("washmybelly.")) return n;
    const s = $t(n) as string;
    return s;
  }

  function evidenceOf(it: SelectItem): string {
    return (it as SelectItem & { evidence?: string }).evidence ?? "";
  }

  let confirmLabel = $derived(
    !installed
      ? ($t("washmybelly.uninstall.remove_leftovers_button") as string)
      : mode === "reset"
        ? ($t("washmybelly.uninstall.reset_button") as string)
        : ($t("washmybelly.uninstall.uninstall_button") as string),
  );

  function done(r: ReceiptSummary) {
    onDone?.(r, shown);
  }
</script>

<div class="wmb-ud">
  {#if onBack}
    <button type="button" class="back" onclick={onBack}>
      <span class="chev" aria-hidden="true"><Icon name="chevron" size={11} /></span>{$t("washmybelly.uninstall.back")}
    </button>
  {/if}

  <header class="head">
    <AppTile name={shown.name} source={shown.source} size={56} dim={!installed} />
    <div class="head-text">
      <h3 title={shown.name}>{shown.name}</h3>
      <p class="meta">
        {#if !installed}
          {$t("washmybelly.uninstall.not_installed")}
        {:else}
          {[shown.version, shown.publisher, sourceLabel(shown.source)].filter(Boolean).join(" · ")}
        {/if}
      </p>
      {#if shown.path}<p class="path" title={shown.path}>{shortenPath(shown.path, 64)}</p>{/if}
    </div>
    {#if shown.bytes != null && installed}<span class="big-size">{formatBytes(shown.bytes)}</span>{/if}
  </header>

  {#if loading}
    <div class="skel" aria-busy="true">
      <span class="skeleton" style:height="18px" style:width="40%"></span>
      <span class="skeleton" style:height="120px"></span>
      <span class="skeleton" style:height="80px"></span>
    </div>
  {:else if error}
    <EmptyState art="worried" compact title={$t("washmybelly.uninstall.leftovers_error") as string} body={error}>
      {#snippet action()}
        <button class="btn btn-secondary" type="button" onclick={() => load(app.id)}>{$t("washmybelly.retry")}</button>
      {/snippet}
    </EmptyState>
  {:else if resp}
    {#if installed && !shown.removable}
      <div class="notice notice-info">
        <Icon name="lock" size={16} />
        <div class="notice-body">
          <span class="notice-title">{$t("washmybelly.uninstall.locked_title")}</span>
          <span class="notice-text">{reasonText(shown.reason)}</span>
        </div>
      </div>
    {/if}

    {#if running}
      <div class="notice notice-warning">
        <Icon name="warning" size={16} />
        <div class="notice-body">
          <span class="notice-title">{$t("washmybelly.uninstall.running_title", { name: shown.name })}</span>
          <span class="notice-text">{$t("washmybelly.uninstall.running_body")}</span>
        </div>
      </div>
    {/if}

    {#each notes as n, i (i)}
      <div class="notice notice-info">
        <Icon name="info" size={16} />
        <div class="notice-body"><span class="notice-text">{note(n)}</span></div>
      </div>
    {/each}

    {#if modes.length > 1}
      <div class="modes">
        <div class="segmented" role="radiogroup" aria-label={$t("washmybelly.uninstall.mode_label") as string}>
          {#each modes as m (m)}
            <button type="button" class="segmented-btn" role="radio" aria-checked={mode === m} onclick={() => (mode = m)}>
              {$t(`washmybelly.uninstall.mode.${m}`)}
            </button>
          {/each}
        </div>
        <p class="mode-hint">{$t(`washmybelly.uninstall.mode_hint.${mode}`, { name: shown.name })}</p>
      </div>
    {/if}

    {#if mode === "uninstall" && classified.app.length}
      <h4 class="group-label">{$t("washmybelly.uninstall.app_section")}</h4>
      <ul class="group fixed">
        {#each classified.app as it (it.id)}
          <li class="group-row">
            <Icon name="check" size={14} />
            <span class="group-row-content">
              <span class="group-row-title">{it.label}</span>
              {#if it.detail}<span class="group-row-sub" title={it.detail}>{shortenPath(it.detail, 64)}</span>{/if}
            </span>
            {#if it.admin}<span class="tag tag-warning"><Icon name="lock" size={10} />{$t("washmybelly.uninstall.admin")}</span>{/if}
            <span class="size">{formatBytes(it.bytes)}</span>
          </li>
        {/each}
      </ul>
    {:else if mode === "zap" && classified.zap.length}
      <h4 class="group-label">{$t("washmybelly.uninstall.zap_section")}</h4>
      <ul class="group fixed">
        {#each classified.zap as it (it.id)}
          <li class="group-row">
            <Icon name="check" size={14} />
            <span class="group-row-content">
              <code class="cmd">{cask ? `brew uninstall --zap ${cask}` : it.label}</code>
              <span class="group-row-sub">{$t("washmybelly.uninstall.zap_body")}</span>
            </span>
          </li>
        {/each}
      </ul>
    {/if}

    {#if sections.length}
      {#each sections as s (s.confidence)}
        <div class="conf-head">
          <h4 class="group-label">
            <span class="conf-dot {s.confidence}" aria-hidden="true"></span>{$t(`washmybelly.uninstall.confidence.${s.confidence}`)}
          </h4>
          <span class="conf-sub">{$t(`washmybelly.uninstall.confidence_hint.${s.confidence}`)}</span>
        </div>
        <SelectList items={toSelect(s.items)} bind:selected={chosen} showToolbar={false} showRisk={false} {groupLabel} maxHeight={360}>
          {#snippet trailing(it)}
            {@const ev = evidenceOf(it)}
            {#if ev}<span class="evidence" title={ev}>{ev}</span>{/if}
          {/snippet}
        </SelectList>
      {/each}
    {:else if installed}
      <p class="none">{$t("washmybelly.uninstall.no_leftovers")}</p>
    {:else}
      <EmptyState art="happy" compact title={$t("washmybelly.uninstall.clean_title", { name: shown.name }) as string} body={$t("washmybelly.uninstall.clean_body") as string} />
    {/if}

    {#if ids.length > 0}
      <footer class="foot">
        <span class="sum">{$t("washmybelly.uninstall.summary", { count: ids.length, size: formatBytes(bytes) })}</span>
        <button class="btn btn-primary" type="button" onclick={() => (reviewOpen = true)}>{confirmLabel}</button>
      </footer>
    {/if}
  {/if}
</div>

<PlanReview kind="uninstall" itemIds={ids} bind:open={reviewOpen} {groupLabel} {confirmLabel} onDone={done} />

<style>
  .wmb-ud {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }
  .back {
    all: unset;
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-sm);
    color: var(--accent-text, var(--accent));
    cursor: pointer;
    border-radius: 4px;
  }
  .back .chev {
    display: inline-flex;
    transform: rotate(180deg);
  }
  .back:focus-visible {
    outline: var(--focus-ring);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .head-text {
    flex: 1;
    min-width: 0;
  }
  h3 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta,
  .path {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .path {
    font-size: var(--text-xs);
  }
  .big-size {
    font-size: var(--text-md);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .skel {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .skel .skeleton {
    display: block;
    border-radius: var(--radius-md);
  }
  .modes {
    display: flex;
    flex-direction: column;
    gap: 6px;
    align-items: flex-start;
  }
  .mode-hint,
  .conf-sub,
  .none {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  h4.group-label {
    margin: var(--space-2) 0 0;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .conf-head {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: var(--space-2);
  }
  .conf-head h4.group-label {
    margin: 0;
  }
  .conf-sub {
    padding: 0 var(--space-2);
    font-size: var(--text-xs);
  }
  .conf-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-dim);
  }
  .conf-dot.high {
    background: var(--success);
  }
  .conf-dot.medium {
    background: var(--warning);
  }
  .conf-dot.low {
    background: var(--text-dim);
  }
  ul.fixed {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  ul.fixed :global(svg) {
    color: var(--success);
    flex-shrink: 0;
  }
  ul.fixed .tag :global(svg) {
    color: inherit;
  }
  .size {
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .cmd {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-sm);
  }
  .evidence {
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-xs);
    color: var(--text-dim);
    padding: 1px 6px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
  }
  .foot {
    position: sticky;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-3);
    padding: var(--space-3) 0 var(--space-1);
    background: linear-gradient(to bottom, transparent, var(--pane-bg, var(--pane)) 30%);
  }
  .sum {
    margin-right: auto;
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
</style>
