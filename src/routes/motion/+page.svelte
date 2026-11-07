<script lang="ts">
  import { t } from "$lib/i18n";
  import type { Bezier } from "$lib/motion/bezier";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import EasingPanel from "$components/motion/EasingPanel.svelte";
  import SpringPanel from "$components/motion/SpringPanel.svelte";
  import TimelinePanel from "$components/motion/TimelinePanel.svelte";
  import LottiePanel from "$components/motion/LottiePanel.svelte";

  type Tab = "curves" | "springs" | "timeline" | "lottie";
  const TABS: Tab[] = ["curves", "springs", "timeline", "lottie"];
  const KEY = "omniget.motion.tab";

  function initial(): Tab {
    try {
      const saved = localStorage.getItem(KEY) as Tab | null;
      if (saved && TABS.includes(saved)) return saved;
    } catch {
      return "curves";
    }
    return "curves";
  }

  let tab = $state<Tab>(initial());
  let incoming = $state<{ ease: Bezier; seq: number } | null>(null);
  let tabEls: HTMLButtonElement[] = [];

  function select(next: Tab, focus = false) {
    tab = next;
    try {
      localStorage.setItem(KEY, next);
    } catch {
      tab = next;
    }
    if (focus) tabEls[TABS.indexOf(next)]?.focus();
  }

  function onTabKey(event: KeyboardEvent) {
    const i = TABS.indexOf(tab);
    let next = -1;
    if (event.key === "ArrowRight") next = (i + 1) % TABS.length;
    else if (event.key === "ArrowLeft") next = (i - 1 + TABS.length) % TABS.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = TABS.length - 1;
    if (next < 0) return;
    event.preventDefault();
    select(TABS[next], true);
  }

  function useEase(ease: Bezier) {
    incoming = { ease: [...ease], seq: (incoming?.seq ?? 0) + 1 };
    select("timeline");
  }
</script>

<section class="mo">
  <header class="mo-head">
    <NavIcon icon="motion" size={56} />
    <div class="mo-head-copy">
      <h1>{$t("motion.title")}</h1>
      <p class="mo-subtitle">{$t("motion.subtitle")}</p>
    </div>
  </header>

  <div class="segmented segmented-lg mo-tabs" role="tablist" aria-label={$t("motion.tabs_label")} tabindex="-1" onkeydown={onTabKey}>
    {#each TABS as id, i (id)}
      <button
        bind:this={tabEls[i]}
        type="button"
        class="segmented-btn"
        role="tab"
        id="mo-tab-{id}"
        aria-selected={tab === id}
        aria-controls="mo-panel-{id}"
        tabindex={tab === id ? 0 : -1}
        onclick={() => select(id)}
      >
        {$t(`motion.tab.${id}`)}
      </button>
    {/each}
  </div>
  <p class="mo-tab-desc">{$t(`motion.tab.${tab}_desc`)}</p>

  <div id="mo-panel-curves" role="tabpanel" aria-labelledby="mo-tab-curves" hidden={tab !== "curves"}>
    <EasingPanel active={tab === "curves"} onuse={useEase} />
  </div>
  <div id="mo-panel-springs" role="tabpanel" aria-labelledby="mo-tab-springs" hidden={tab !== "springs"}>
    <SpringPanel active={tab === "springs"} />
  </div>
  <div id="mo-panel-timeline" role="tabpanel" aria-labelledby="mo-tab-timeline" hidden={tab !== "timeline"}>
    <TimelinePanel active={tab === "timeline"} {incoming} />
  </div>
  <div id="mo-panel-lottie" role="tabpanel" aria-labelledby="mo-tab-lottie" hidden={tab !== "lottie"}>
    <LottiePanel active={tab === "lottie"} />
  </div>

  <p class="mo-credit">{$t("motion.credit")}</p>
</section>

<style>
  .mo {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .mo-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .mo-head-copy {
    min-width: 0;
  }

  .mo-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .mo-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .mo-tabs {
    align-self: flex-start;
  }

  .mo-tab-desc {
    margin: calc(var(--space-2) * -1) 0 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .mo-credit {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-faint);
  }

  .mo :global(.mo-card) {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .mo :global(.mo-card-head) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
  }

  .mo :global(.mo-card-title) {
    margin: 0 auto 0 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .mo :global(.mo-sub) {
    margin: 0;
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
    color: var(--text-dim);
  }

  .mo :global(.mo-meta) {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .mo :global(.mo-note) {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .mo :global(.mo-check) {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-sm);
    color: var(--text-muted);
    cursor: pointer;
  }

  .mo :global(.mo-check input) {
    accent-color: var(--accent);
  }

  .mo :global(.mo-transport) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .mo :global(.mo-codes) {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: var(--space-3);
  }

  .mo :global(.mo-status) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-2);
    min-height: var(--control-h);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .mo :global(.mo-ok) {
    color: var(--success);
  }

  .mo :global(.mo-bad) {
    color: var(--error);
  }

  .mo :global(.mo-spin) {
    width: 12px;
    height: 12px;
  }
</style>
