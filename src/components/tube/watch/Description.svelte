<script lang="ts">
  import { t } from "$lib/i18n";
  import { fmtCount } from "$lib/tube/format";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import type { WatchInfo } from "$lib/tube/types";
  import Runs from "./Runs.svelte";

  let { info, onseek }: { info: WatchInfo; onseek: (t: number) => void } = $props();

  let open = $state(false);
  let body = $state<HTMLDivElement>();
  let overflows = $state(true);

  $effect(() => {
    void info.id;
    open = false;
  });

  $effect(() => {
    void info.description;
    if (!body || open) return;
    overflows = body.scrollHeight > body.clientHeight + 2;
  });

  let views = $derived(
    tubePrefs().hide_metrics
      ? ""
      : open && info.views != null
        ? ($t("tube.card.views", { count: info.views.toLocaleString() }) as string)
        : info.views_text || (info.views != null ? ($t("tube.card.views", { count: fmtCount(info.views) }) as string) : ""),
  );
  let when = $derived(open ? info.date_text || info.published_text : info.published_text || info.date_text);
  let tags = $derived(info.description.filter((r) => r.text.startsWith("#") && r.url).slice(0, 3));
  let hasText = $derived(info.description.some((r) => r.text.trim()));
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="desc" class:open onclick={() => !open && (open = true)}>
  <div class="top">
    {#if views}<span>{views}</span>{/if}
    {#if when}<span>{when}</span>{/if}
    {#each tags as tg, i (i)}<span class="tag">{tg.text}</span>{/each}
  </div>
  {#if hasText}
    <div class="body" class:clamp={!open} bind:this={body}>
      <Runs runs={info.description} videoId={info.id} {onseek} />
    </div>
  {/if}
  {#if !open && (overflows || !hasText)}
    <button class="toggle" onclick={(e) => { e.stopPropagation(); open = true; }}>{$t("tube.watch.more")}</button>
  {:else if open}
    <button class="toggle less" onclick={(e) => { e.stopPropagation(); open = false; }}>{$t("tube.watch.less")}</button>
  {/if}
</div>

<style>
  .desc {
    position: relative;
    margin-top: 12px;
    padding: 12px;
    border-radius: 12px;
    font: var(--yt-type-meta);
    color: var(--yt-text);
    background: var(--yt-tonal);
    cursor: pointer;
    transition: background var(--yt-dur-base);
  }
  .desc:not(.open):hover {
    background: var(--yt-tonal-hover);
  }
  .desc.open {
    cursor: default;
  }
  .top {
    display: flex;
    flex-wrap: wrap;
    gap: 0 8px;
    font: var(--yt-type-body-strong);
  }
  .tag {
    color: var(--yt-text-2);
  }
  .body {
    white-space: pre-wrap;
    word-break: break-word;
    margin-top: 2px;
  }
  .body.clamp {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .toggle {
    padding: 0;
    border: 0;
    font: var(--yt-type-body-strong);
    color: var(--yt-text);
    background: none;
    cursor: pointer;
  }
  .toggle.less {
    display: block;
    margin-top: 16px;
  }
  .toggle:focus-visible {
    outline: 2px solid var(--yt-link);
    border-radius: 4px;
  }
</style>
