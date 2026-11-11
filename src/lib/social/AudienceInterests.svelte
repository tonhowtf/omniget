<script lang="ts">
  import { untrack } from "svelte";
  import { t, locale } from "$lib/i18n";
  import { compact, social, tool, type Platform } from "./api";
  import { weighted, type Counted, type Interests } from "./second";

  let { platform, account }: { platform: Platform; account: string } = $props();

  let data = $state<Interests | null>(null);
  let loading = $state(true);
  let failed = $state(false);

  let words = $derived(weighted(data?.words.items ?? []));
  let tags = $derived(weighted(data?.hashtags.items ?? []));
  let names = $derived(weighted((data?.name_words.items ?? []).slice(0, 20)));

  async function load() {
    loading = true;
    failed = false;
    try {
      data = await social.call<Interests>(tool(platform, "audience"), { view: "interests", account, limit: 60 });
    } catch {
      failed = true;
    } finally {
      loading = false;
    }
  }

  function text(c: Counted): string {
    return c.word ?? (c.hashtag ? `#${c.hashtag}` : (c.location ?? ""));
  }

  function share(n: number | null | undefined): string {
    return n == null ? "–" : `${n}%`;
  }

  $effect(() => {
    void platform;
    void account;
    untrack(() => void load());
  });
</script>

<section class="in" aria-labelledby="audience-interests-title">
  <h3 class="in-h" id="audience-interests-title">{$t("social_audience.interests.title")}</h3>
  {#if loading}
    <div class="in-state"><span class="spinner"></span></div>
  {:else if failed}
    <div class="notice notice-warning" role="alert">
      <div class="notice-body"><div class="notice-text">{$t("social_audience.interests.failed")}</div></div>
      <button class="btn btn-ghost btn-sm" onclick={() => void load()}>{$t("social.common.retry")}</button>
    </div>
  {:else if data && data.empty_reason}
    <div class="in-state">{$t("social_audience.interests.empty")}</div>
  {:else if data}
    <p class="in-hint">{$t("social_audience.interests.hint")}</p>
    <div class="in-facts">
      <span class="tag">{$t("social_audience.interests.based_on", { bios: compact(data.with_bio, $locale ?? undefined), followers: compact(data.followers, $locale ?? undefined) })}</span>
      <span class="tag">{$t("social_audience.interests.with_link")} · {share(data.with_link.share_of_bios)}</span>
      <span class="tag">{$t("social_audience.interests.verified")} · {share(data.verified.share)}</span>
    </div>
    {#if data.thin}
      <div class="notice notice-info" role="status"><div class="notice-body"><div class="notice-text">{$t("social_audience.interests.thin")}</div></div></div>
    {/if}
    {#each [{ key: "words", list: words }, { key: "hashtags", list: tags }, { key: "names", list: names }] as group (group.key)}
      {#if group.list.length}
        <h4 class="in-sub">{$t(`social_audience.interests.${group.key}`)}</h4>
        <ul class="in-cloud">
          {#each group.list as w (text(w))}
            <li style="font-size: {w.size}em" title={$t("social_audience.interests.accounts", { word: text(w), count: w.accounts })}>{text(w)} <span class="in-n">{w.accounts}</span></li>
          {/each}
        </ul>
      {/if}
    {/each}
    {#if !words.length && !tags.length}
      <div class="in-state">{$t("social_audience.interests.none")}</div>
    {/if}
    <h4 class="in-sub">{$t("social_audience.interests.locations")}</h4>
    {#if data.locations.available}
      <ol class="in-places">
        {#each data.locations.items.slice(0, 12) as l (l.location)}
          <li><span>{l.location}</span><span class="in-n">{l.accounts}</span></li>
        {/each}
      </ol>
    {:else}
      <p class="in-hint">{$t("social_audience.interests.no_locations")}</p>
    {/if}
  {/if}
</section>

<style>
  .in {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .in-h,
  .in-sub {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-dim);
  }

  .in-sub {
    font-size: var(--text-xs);
  }

  .in-hint {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .in-facts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .in-state {
    display: flex;
    justify-content: center;
    padding: var(--space-4) 0;
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  .in-cloud {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px var(--space-3);
    margin: 0;
    padding: var(--space-3);
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-size: var(--text-sm);
    line-height: 1.3;
    overflow-wrap: anywhere;
  }

  .in-n {
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }

  .in-places {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 2px var(--space-4);
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-sm);
  }

  .in-places li {
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
    min-width: 0;
  }

  .in-places li span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
