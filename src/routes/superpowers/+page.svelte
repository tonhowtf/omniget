<script lang="ts">
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { CATEGORIES, SUPERPOWERS, TAGS, type Superpower, type SuperpowerCategory } from "$lib/superpowers/catalog";
  import { MAX_RAIL_PINS, currentPins, togglePin } from "$lib/superpowers/pins";
  import { searchPowers } from "$lib/superpowers/search";
  import { powerPatch } from "$components/settings/integrations/superpowers";
  import NavIcon from "$components/shell/NavIcon.svelte";

  type Power = Superpower & {
    title: string;
    description: string;
    categoryLabel: string;
    tagLabels: string[];
    enabled: boolean;
  };

  let settings = $derived(getSettings());
  let pinned = $derived<string[]>(currentPins(settings?.superpowers?.pinned ?? []));

  let powers = $derived<Power[]>(
    SUPERPOWERS.map((entry) => ({
      ...entry,
      title: entry.title ?? ($t(entry.titleKey ?? "") as string),
      description: $t(entry.descKey) as string,
      categoryLabel: $t(`superpowers.cat_${entry.category}`) as string,
      tagLabels: entry.tags.map((tag) => TAGS[tag].label ?? ($t(TAGS[tag].labelKey ?? "") as string)),
      enabled: entry.settingsKey ? (settings?.[entry.settingsKey]?.enabled ?? true) : true,
    })),
  );

  let query = $state("");
  let filter = $state<SuperpowerCategory | "all">("all");
  let searching = $derived(query.trim().length > 0);

  let found = $derived(
    searchPowers(
      powers.map((power) => ({ ...power, category: power.categoryLabel, tags: [...power.tags, ...power.tagLabels] })),
      query,
    ).map((hit) => powers.find((power) => power.id === hit.id) as Power),
  );
  let visible = $derived(filter === "all" ? found : found.filter((power) => power.category === filter));
  let grouped = $derived(!searching && filter === "all");
  let pinnedPowers = $derived(
    pinned.map((id) => powers.find((power) => power.id === id)).filter((power): power is Power => !!power),
  );

  let chips = $derived([
    { id: "all" as const, label: $t("superpowers.cat_all") as string, count: found.length },
    ...CATEGORIES.map((category) => ({
      id: category.id,
      label: $t(category.labelKey) as string,
      count: found.filter((power) => power.category === category.id).length,
    })),
  ]);

  function target(power: Power): string {
    if (power.enabled) return power.href;
    return `/settings?tab=integrations&section=superpowers&setting=settings.integrations.superpowers.${power.id}`;
  }

  function turnOn(power: Power) {
    if (!power.settingsKey) return;
    updateSettings(powerPatch(settings, power.settingsKey, true)).catch(() => {});
  }

  function pin(power: Power) {
    updateSettings({ superpowers: { pinned: togglePin(pinned, power.id) } }).catch(() => {});
  }

  function reset() {
    query = "";
    filter = "all";
  }

  function onSearchKey(event: KeyboardEvent) {
    if (event.key === "Escape" && (query || filter !== "all")) {
      event.preventDefault();
      if (query) query = "";
      else filter = "all";
    } else if (event.key === "Enter" && !event.isComposing && searching && visible[0]) {
      event.preventDefault();
      void goto(target(visible[0]));
    }
  }
</script>

{#snippet card(power: Power)}
  {@const isPinned = pinned.includes(power.id)}
  <article class="power-card" class:off={!power.enabled} class:plain={power.enabled && power.tagLabels.length === 0}>
    <NavIcon icon={power.icon} size={48} />
    <div class="power-copy">
      <h3><a class="power-link" href={target(power)}>{power.title}</a></h3>
      <p class="power-desc">{power.description}</p>
      {#if !power.enabled}
        <p class="power-meta">
          <span class="power-tag power-tag-off">{$t("superpowers.disabled")}</span>
          <button type="button" class="btn btn-secondary btn-sm power-on" onclick={() => turnOn(power)}>{$t("superpowers.turn_on")}</button>
        </p>
      {:else if power.tagLabels.length > 0}
        <p class="power-meta">
          {#each power.tagLabels as label (label)}
            <span class="power-tag">{label}</span>
          {/each}
        </p>
      {/if}
    </div>
    <button
      type="button"
      class="power-pin"
      class:on={isPinned}
      aria-pressed={isPinned}
      aria-label={$t(isPinned ? "superpowers.unpin" : "superpowers.pin", { name: power.title }) as string}
      title={$t(isPinned ? "superpowers.unpin" : "superpowers.pin", { name: power.title }) as string}
      onclick={() => pin(power)}
    >
      <svg viewBox="0 0 20 20" width="16" height="16" fill={isPinned ? "currentColor" : "none"} stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" aria-hidden="true">
        <path d="M10 2.6l2.25 4.7 5.15.68-3.76 3.58.94 5.1L10 14.2l-4.58 2.46.94-5.1L2.6 7.98l5.15-.68z" />
      </svg>
    </button>
  </article>
{/snippet}

<section class="powers">
  <header class="powers-head">
    <NavIcon icon="superpowers" size={44} />
    <div class="powers-title">
      <h1>{$t("nav.superpowers")}</h1>
      <p class="powers-subtitle">{$t("superpowers.subtitle")}</p>
    </div>
    <input
      class="input input-search powers-search"
      type="search"
      placeholder={$t("superpowers.search") as string}
      aria-label={$t("superpowers.search") as string}
      bind:value={query}
      onkeydown={onSearchKey}
    />
  </header>

  <div class="powers-chips" role="group" aria-label={$t("superpowers.filter_label") as string}>
    {#each chips as chip (chip.id)}
      <button
        type="button"
        class="power-chip"
        class:empty={chip.count === 0}
        aria-pressed={filter === chip.id}
        onclick={() => (filter = chip.id)}
      >
        {chip.label}
        <span class="power-chip-count">{chip.count}</span>
      </button>
    {/each}
  </div>

  {#if grouped}
    {#if pinnedPowers.length > 0}
      <section class="powers-group" aria-labelledby="powers-pinned">
        <div class="powers-group-head">
          <h2 id="powers-pinned">{$t("superpowers.pinned")}</h2>
          {#if pinnedPowers.length > MAX_RAIL_PINS}
            <p>{$t("superpowers.pinned_limit", { count: MAX_RAIL_PINS })}</p>
          {/if}
        </div>
        <div class="powers-grid">
          {#each pinnedPowers as power (power.id)}
            {@render card(power)}
          {/each}
        </div>
      </section>
    {/if}
    {#each CATEGORIES as category (category.id)}
      <section class="powers-group" aria-labelledby="powers-{category.id}">
        <div class="powers-group-head">
          <h2 id="powers-{category.id}">{$t(category.labelKey)}</h2>
        </div>
        <div class="powers-grid">
          {#each powers.filter((power) => power.category === category.id) as power (power.id)}
            {@render card(power)}
          {/each}
        </div>
      </section>
    {/each}
  {:else if visible.length > 0}
    <div class="powers-grid">
      {#each visible as power (power.id)}
        {@render card(power)}
      {/each}
    </div>
  {:else}
    <div class="powers-empty" role="status">
      <p>{$t("superpowers.no_results", { query: query.trim() })}</p>
      <button type="button" class="btn btn-secondary" onclick={reset}>{$t("superpowers.clear")}</button>
    </div>
  {/if}
</section>

<style>
  .powers {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    width: 100%;
    max-width: 1120px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .powers-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3) var(--space-4);
  }

  .powers-title {
    flex: 1 1 240px;
    min-width: 0;
  }

  .powers-head h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 700;
    line-height: var(--leading-xl);
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .powers-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .powers-search {
    flex: 1 1 220px;
    max-width: 320px;
  }

  .powers-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .power-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: var(--control-h);
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .power-chip:hover:not([aria-pressed="true"]) {
      background: var(--fill-2);
    }
  }

  .power-chip:active:not([aria-pressed="true"]) {
    background: var(--fill-3);
  }

  .power-chip[aria-pressed="true"] {
    background: var(--text);
    color: var(--bg);
  }

  .power-chip.empty:not([aria-pressed="true"]) {
    color: var(--text-dim);
  }

  .power-chip:focus-visible,
  .power-pin:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .power-chip-count {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    opacity: 0.62;
  }

  .powers-group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .powers-group-head {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
  }

  .powers-group-head h2 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text-muted);
  }

  .powers-group-head p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .powers-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 272px), 1fr));
    gap: var(--space-3);
  }

  .power-card {
    position: relative;
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    min-width: 0;
    padding: var(--space-3) var(--space-4) var(--space-3) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    transition: box-shadow var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .power-card:hover {
      box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--text) 32%, transparent);
    }
  }

  .power-card:has(.power-link:active) {
    background: var(--surface-hi);
  }

  .power-card:has(.power-link:focus-visible) {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .power-card.off :global(.nav-icon) {
    filter: grayscale(1);
    opacity: 0.55;
  }

  .power-copy {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .power-copy h3 {
    margin: 0;
    padding-inline-end: var(--space-5);
    font-size: var(--text-md);
    font-weight: 600;
    line-height: var(--leading-md);
  }

  .power-link {
    color: var(--text);
    text-decoration: none;
    outline: none;
  }

  .power-link::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: inherit;
  }

  .power-desc {
    display: -webkit-box;
    margin: 0;
    overflow: hidden;
    color: var(--text-dim);
    font-size: var(--text-sm);
    line-height: var(--leading-sm);
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  .power-card.plain .power-desc {
    -webkit-line-clamp: 3;
    line-clamp: 3;
  }

  .power-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) 6px;
    margin: var(--space-1) 0 0;
    color: var(--text-dim);
    font-size: var(--text-xs);
  }

  .power-tag {
    padding: 1px 6px;
    border-radius: var(--radius-xs);
    background: var(--fill-1);
    color: var(--text-muted);
    font-weight: 500;
  }

  .power-tag-off {
    background: var(--fill-2);
    font-weight: 600;
  }

  .power-on {
    position: relative;
    z-index: 1;
  }

  .power-pin {
    position: absolute;
    inset-block-start: var(--space-2);
    inset-inline-end: var(--space-2);
    z-index: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--control-h);
    height: var(--control-h);
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    opacity: 0.55;
    transition: opacity var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
  }

  .power-card:hover .power-pin,
  .power-pin:focus-visible,
  .power-pin.on {
    opacity: 1;
  }

  .power-pin.on {
    color: var(--accent-text);
  }

  @media (hover: hover) {
    .power-pin:hover {
      background: var(--fill-2);
      color: var(--text);
    }

    .power-pin.on:hover {
      color: var(--accent-text);
    }
  }

  .power-pin:active {
    background: var(--fill-3);
  }

  .powers-empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .powers-empty p {
    margin: 0;
    overflow-wrap: anywhere;
  }

  @media (prefers-reduced-motion: reduce) {
    .power-chip,
    .power-card,
    .power-pin {
      transition: none;
    }
  }
</style>
