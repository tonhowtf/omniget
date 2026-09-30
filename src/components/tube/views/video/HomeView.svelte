<script lang="ts">
  // Home: sticky chips, rich grid with Shorts shelves in between, infinite
  // scroll. With `hide_home_feed` it is a calm page with the search in the
  // middle and a "Show suggestions" button that loads the feed on demand.
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import { go, type Route } from "$lib/tube/ui.svelte";
  import TIcon from "../../TIcon.svelte";
  import ChipBar from "../../cards/ChipBar.svelte";
  import Sentinel from "../../cards/Sentinel.svelte";
  import Shelf from "../../cards/Shelf.svelte";
  import SkeletonGrid from "../../cards/SkeletonGrid.svelte";
  import State from "../../cards/State.svelte";
  import { signIn } from "../../shell/session.svelte";
  import UndoPill from "../../cards/UndoPill.svelte";
  import { Pager } from "../../cards/pager.svelte";
  import { lsGet, lsSet } from "../../cards/util";

  let { route: _route }: { route: Extract<Route, { name: "home" }> } = $props();

  const CHIP_KEY = "omnitube.home.chip";

  let prefs = $derived(tubePrefs());
  let reveal = $state(false);
  let q = $state("");
  let chip = $state<{ label: string; token: string } | null>(lsGet(CHIP_KEY, null));
  // Chips of the unfiltered feed stay put while a chip's page is shown.
  let chips = $state<{ label: string; token: string | null }[]>([]);

  const pager = new Pager("home", () => api.home(chip?.token ?? null));

  let hidden = $derived(prefs.hide_home_feed && !reveal);

  $effect(() => {
    if (hidden) return;
    void pager.load(() => api.home(chip?.token ?? null)).then(() => {
      if (!chip && pager.page) chips = pager.page.chips.map((c) => ({ label: c.label, token: c.token ?? null }));
      if (chip && pager.error) {
        // A remembered chip token can expire; fall back to the plain feed.
        chip = null;
        lsSet(CHIP_KEY, null);
      }
    });
  });

  function pick(key: string) {
    const c = chips.find((x) => x.label === key);
    const next = c && c.token && chip?.token !== c.token ? { label: c.label, token: c.token } : null;
    chip = next;
    lsSet(CHIP_KEY, next);
  }

  let chipDefs = $derived(
    chips.map((c, i) => ({
      key: c.label,
      label: c.label,
      selected: chip ? chip.label === c.label : i === 0,
    })),
  );

  function search(e: SubmitEvent) {
    e.preventDefault();
    const s = q.trim();
    if (s) go({ name: "search", q: s });
  }

  const TOPICS = ["music", "gaming", "news", "sports", "learning", "podcasts", "cooking", "tech"] as const;

  let readiness = $derived(hidden ? "ready" : pager.loading ? "loading" : pager.error ? "error" : "ready");
</script>

<div class="home" data-agent-readiness={readiness} data-agent-surface="omnitube-home">
  {#if hidden}
    <div class="calm">
      <img src="/icons/tube/tube-128.webp" width="96" height="96" alt="" />
      <h1>{$t("tube.home.calm_title")}</h1>
      <form class="search" onsubmit={search} role="search">
        <input bind:value={q} type="search" placeholder={$t("tube.home.search_placeholder") as string} aria-label={$t("tube.home.search_placeholder") as string} />
        <button type="submit" aria-label={$t("tube.home.search") as string}><TIcon name="search" /></button>
      </form>
      <div class="calm-actions">
        <button class="yt-btn" onclick={() => go({ name: "subscriptions" })}><TIcon name="subscriptions" size={20} />{$t("tube.home.go_subscriptions")}</button>
        <button class="yt-btn yt-btn--ghost" onclick={() => (reveal = true)}>{$t("tube.home.show_suggestions")}</button>
      </div>
    </div>
  {:else}
    {#if chipDefs.length > 1 && !prefs.hide_chips}
      <ChipBar chips={chipDefs} onselect={pick} sticky label={$t("tube.home.chips") as string} />
    {/if}

    <div class="feed">
      {#if pager.loading && !pager.page}
        <SkeletonGrid count={16} />
      {:else if pager.error && !pager.page}
        <State icon="home" error={pager.error} onretry={() => pager.load()} />
      {:else if pager.page && pager.shelves.every((s) => s.items.length === 0)}
        <!-- New visitor without account or history: YouTube itself only says "search to start". -->
        <div class="calm fresh">
          <img src="/icons/tube/tube-128.webp" width="96" height="96" alt="" />
          <h1>{$t("tube.home.empty_title")}</h1>
          <p class="calm-text">{$t("tube.home.empty_text")}</p>
          <form class="search" onsubmit={search} role="search">
            <input bind:value={q} type="search" placeholder={$t("tube.home.search_placeholder") as string} aria-label={$t("tube.home.search_placeholder") as string} />
            <button type="submit" aria-label={$t("tube.home.search") as string}><TIcon name="search" /></button>
          </form>
          <div class="topics" role="group" aria-label={$t("tube.home.topics_label") as string}>
            {#each TOPICS as topic (topic)}
              {@const label = $t(`tube.home.topic_${topic}`) as string}
              <button class="yt-chip" onclick={() => go({ name: "search", q: label })}>{label}</button>
            {/each}
          </div>
          <div class="calm-actions">
            <button class="yt-btn yt-btn--outline" onclick={() => void signIn()}><TIcon name="user" size={20} />{$t("tube.common.sign_in")}</button>
            <button class="yt-btn" onclick={() => go({ name: "import" })}><TIcon name="import" size={20} />{$t("tube.shell.import")}</button>
          </div>
        </div>
      {:else}
        <div class="shelves" class:dim={pager.loading}>
          {#each pager.shelves as s, i (s.id)}
            <Shelf shelf={s} first={i === 0} />
          {/each}
        </div>
        {#if pager.hasMore}
          <Sentinel onmore={() => pager.more()} loading={pager.loadingMore} error={pager.moreError} />
        {/if}
      {/if}
    </div>
  {/if}
  <UndoPill />
</div>

<style>
  .home {
    padding: 0 var(--yt-grid-gutter) 24px;
    min-height: 100%;
  }
  .feed {
    padding-top: 24px;
  }
  .shelves {
    display: flex;
    flex-direction: column;
    gap: 40px;
    transition: opacity var(--yt-dur-base);
  }
  .shelves.dim {
    opacity: 0.5;
    pointer-events: none;
  }
  .calm.fresh {
    padding-top: 8vh;
    gap: 16px;
  }
  .calm-text {
    margin: -8px 0 0;
    max-width: 440px;
    font: var(--yt-type-meta);
    color: var(--yt-text-2);
  }
  .topics {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
    max-width: 560px;
  }
  .calm {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 20px;
    max-width: 640px;
    margin: 0 auto;
    padding: 14vh 0 48px;
    text-align: center;
  }
  .calm img {
    filter: drop-shadow(0 12px 32px var(--yt-shadow));
  }
  .calm h1 {
    margin: 0;
    font: var(--yt-type-headline-m);
  }
  .search {
    display: flex;
    width: 100%;
    height: var(--yt-search-h);
  }
  .search input {
    flex: 1;
    min-width: 0;
    padding: 0 16px;
    border: 1px solid var(--yt-search-border);
    border-right: 0;
    border-radius: 40px 0 0 40px;
    font: 400 16px/22px var(--yt-font);
    color: var(--yt-text);
    background: var(--yt-search-bg);
    outline: none;
  }
  .search input:focus {
    border-color: var(--yt-link);
  }
  .search button {
    display: grid;
    place-items: center;
    width: 64px;
    border: 1px solid var(--yt-search-border);
    border-radius: 0 40px 40px 0;
    color: var(--yt-text);
    background: var(--yt-search-btn);
    cursor: pointer;
  }
  .search button:hover {
    background: var(--yt-tonal-hover);
  }
  .calm-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }
  @container omnitube (max-width: 640px) {
    .home {
      padding: 0 16px 24px;
    }
  }
</style>
