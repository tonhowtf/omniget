<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { acquireLimitsMonitor, getLimitsMonitor, saveLimitsPrefs, type Edge, type Prefs, type ProviderInfo, type StripSize } from "$lib/stores/limits-monitor.svelte";
  const EDGES: Edge[] = ["top", "right", "bottom", "left"];
  const SIZES: StripSize[] = ["s", "m", "l"];
  const LOOKS = ["show_percent", "show_pace", "contrast"] as const;
  let monitor = $derived(getLimitsMonitor());
  let prefs = $derived(monitor.prefs);
  let infos = $derived(monitor.providers);
  let busy = $derived(monitor.busy);
  let unavailable = $derived(!prefs && !!monitor.error);
  const save = saveLimitsPrefs;
  onMount(acquireLimitsMonitor);

  function setEdge(edge: Edge) {
    if (prefs && prefs.edge !== edge) void save({ ...prefs, edge });
  }

  function setSize(size: StripSize) {
    if (prefs && prefs.size !== size) void save({ ...prefs, size });
  }

  function flip(key: (typeof LOOKS)[number] | "notify_pace") {
    if (prefs) void save({ ...prefs, [key]: !prefs[key] } as Prefs);
  }

  function toggleProvider(id: string) {
    if (!prefs) return;
    void save({
      ...prefs,
      providers: prefs.providers.map((p) => (p.id === id ? { ...p, enabled: !p.enabled } : p)),
    });
  }

  function info(id: string): ProviderInfo | undefined {
    return infos.find((i) => i.id === id);
  }
</script>

<section class="limits" data-setting-key="agents.limits.edge">

  {#if monitor.error}<p role="alert" class="field-hint">{monitor.error}</p>{/if}
  {#if unavailable}
    <p class="field-hint">{$t("agents.limits.unavailable")}</p>
  {:else if prefs}
    <div class="limits-row">
      <span class="limits-name">{$t("agents.limits.edge")}</span>
      <div class="segmented" role="radiogroup" aria-label={$t("agents.limits.edge") as string}>
        {#each EDGES as edge (edge)}
          <button
            type="button"
            role="radio"
            class="segment"
            class:active={prefs.edge === edge}
            aria-checked={prefs.edge === edge}
            disabled={busy}
            onclick={() => setEdge(edge)}
          >
            {$t(`agents.limits.edge_${edge}`)}
          </button>
        {/each}
      </div>
    </div>
    <p class="field-hint"><bdi>{$t("agents.limits.drag_hint")}</bdi></p>

    <h3 class="limits-sub">{$t("agents.limits.appearance")}</h3>
    <div class="limits-row">
      <span class="limits-name">{$t("agents.limits.size")}</span>
      <div class="segmented" role="radiogroup" aria-label={$t("agents.limits.size") as string}>
        {#each SIZES as size (size)}
          <button
            type="button"
            role="radio"
            class="segment"
            class:active={prefs.size === size}
            aria-checked={prefs.size === size}
            aria-label={$t(`agents.limits.size_${size}`) as string}
            title={$t(`agents.limits.size_${size}`) as string}
            disabled={busy}
            onclick={() => setSize(size)}
          >
            {size.toUpperCase()}
          </button>
        {/each}
      </div>
    </div>
    {#each LOOKS as key (key)}
      <div class="limits-row">
        <span class="limits-text">
          <span class="limits-name">{$t(`agents.limits.${key}`)}</span>
          <span class="limits-reads">{$t(`agents.limits.${key}_hint`)}</span>
        </span>
        <button
          type="button"
          role="switch"
          class="toggle"
          class:on={prefs[key]}
          aria-checked={prefs[key]}
          aria-label={$t(`agents.limits.${key}`) as string}
          disabled={busy}
          onclick={() => flip(key)}
        >
          <span class="toggle-knob"></span>
        </button>
      </div>
    {/each}

    <h3 class="limits-sub">{$t("agents.limits.notifications")}</h3>
    <div class="limits-row">
      <span class="limits-text">
        <span class="limits-name">{$t("agents.limits.notify_pace")}</span>
        <span class="limits-reads">{$t("agents.limits.notify_pace_hint")}</span>
      </span>
      <button
        type="button"
        role="switch"
        class="toggle"
        class:on={prefs.notify_pace}
        aria-checked={prefs.notify_pace}
        aria-label={$t("agents.limits.notify_pace") as string}
        disabled={busy}
        onclick={() => flip("notify_pace")}
      >
        <span class="toggle-knob"></span>
      </button>
    </div>

    <h3 class="limits-sub">{$t("agents.limits.providers")}</h3>
    <p class="field-hint">{$t("agents.limits.privacy")}</p>
    <ul class="limits-list">
      {#each prefs.providers as p (p.id)}
        {@const meta = info(p.id)}
        <li class="limits-provider" class:missing={meta && !meta.detected}>
          <label class="limits-check">
            <input type="checkbox" checked={p.enabled} disabled={busy} onchange={() => toggleProvider(p.id)} />
            <span class="limits-text">
              <span class="limits-name">
                {meta?.label ?? p.id}
                {#if meta?.account?.label || meta?.account?.email}<span class="limits-dim">{[meta.account.label, meta.account.email].filter(Boolean).join(" · ")}</span>{/if}
                {#if meta?.local}<span class="tag">{$t("agents.limits.local")}</span>{/if}
                {#if meta?.beta}<span class="tag beta" title={$t("agents.limits.beta_hint") as string}>{$t("agents.limits.beta")}</span>{/if}
                {#if meta && !meta.detected}<span class="limits-dim">{$t("agents.limits.not_found")}</span>{/if}
              </span>
              <span class="limits-reads">{$t(`agents.limits.reads.${p.id.split(":")[0]}`)}</span>
            </span>
          </label>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .limits {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .limits-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .limits-name {
    font-weight: 600;
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
  }
  .limits-sub {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    font-weight: 600;
  }
  .limits-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .limits-provider.missing {
    opacity: 0.6;
  }
  .limits-check {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    cursor: pointer;
  }
  .limits-check input {
    margin-top: 3px;
    accent-color: var(--accent);
  }
  .limits-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .limits-reads,
  .limits-dim {
    font-size: var(--text-xs, 11px);
    font-weight: 400;
    color: var(--text-dim);
  }
  .tag {
    font-size: 10px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--surface-hi);
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .tag.beta {
    background: color-mix(in srgb, var(--warning) 22%, transparent);
    color: color-mix(in srgb, var(--warning) 80%, var(--text));
  }
  .segmented {
    display: inline-flex;
    padding: 2px;
    border-radius: var(--radius-md, 8px);
    background: var(--surface-hi);
  }
  .segment {
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    padding: 4px 10px;
    border-radius: 6px;
    cursor: pointer;
  }
  .segment.active {
    background: var(--accent);
    color: var(--on-accent);
  }
</style>
