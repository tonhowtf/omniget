<script lang="ts">
  // like | dislike segmented pill. The like "pops" with a few particles
  // (21st 26660); the count slides. Return YouTube Dislike fills in the
  // dislike count (and the likes when YouTube hides them).
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { fmtCount } from "$lib/tube/format";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import type { Rating, Ryd, WatchInfo } from "$lib/tube/types";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TIcon from "../TIcon.svelte";
  import { errText } from "../cards/util";

  let { info }: { info: WatchInfo } = $props();

  let rating = $state<Rating | null>(null);
  let ryd = $state<Ryd | null>(null);
  let pop = $state(0);

  $effect(() => {
    rating = info.rating ?? null;
    ryd = null;
    const id = info.id;
    if (!tubePrefs().ryd) return;
    api.ryd(id).then(
      (r) => {
        if (info.id === id) ryd = r;
      },
      () => {},
    );
  });

  let prefs = $derived(tubePrefs());
  let baseLikes = $derived(info.likes ?? ryd?.likes ?? null);
  let likes = $derived(
    baseLikes == null ? null : baseLikes + (rating === "like" && info.rating !== "like" ? 1 : 0) - (rating !== "like" && info.rating === "like" ? 1 : 0),
  );
  let dislikes = $derived(ryd ? ryd.dislikes + (rating === "dislike" && info.rating !== "dislike" ? 1 : 0) : null);

  async function rate(r: Rating) {
    const prev = rating;
    const next = prev === r ? null : r;
    rating = next;
    if (next === "like") pop++;
    try {
      await api.action(next === "like" ? "like" : next === "dislike" ? "dislike" : "remove_like", { video_id: info.id });
    } catch (e) {
      rating = prev;
      showToast("error", errText(e));
    }
  }
</script>

<div class="yt-seg likes">
  <button
    class="yt-btn like"
    aria-pressed={rating === "like"}
    aria-label={$t("tube.watch.like") as string}
    title={$t("tube.watch.like") as string}
    onclick={() => rate("like")}
  >
    <span class="ic">
      {#key pop}
        <span class="icon" class:bounce={pop > 0}><TIcon name={rating === "like" ? "thumbs-up-fill" : "thumbs-up"} size={22} /></span>
        {#if pop > 0}
          {#each Array(6) as _, i (i)}<i class="p" style:--a="{-150 + i * 24}deg" style:--d="{26 + (i % 2) * 10}px"></i>{/each}
        {/if}
      {/key}
    </span>
    {#if likes != null && !prefs.hide_metrics}
      <span class="n">{#key likes}<span class="roll">{fmtCount(likes)}</span>{/key}</span>
    {/if}
  </button>
  <button
    class="yt-btn dislike"
    aria-pressed={rating === "dislike"}
    aria-label={$t("tube.watch.dislike") as string}
    title={$t("tube.watch.dislike") as string}
    onclick={() => rate("dislike")}
  >
    <TIcon name={rating === "dislike" ? "thumbs-down-fill" : "thumbs-down"} size={22} />
    {#if dislikes != null && !prefs.hide_metrics}<span class="n">{fmtCount(dislikes)}</span>{/if}
  </button>
</div>

<style>
  .likes {
    flex: none;
  }
  .likes > .yt-btn {
    gap: 6px;
  }
  .like {
    padding-right: 12px;
  }
  .ic {
    position: relative;
    display: inline-flex;
  }
  .icon {
    display: inline-flex;
  }
  .bounce {
    animation: bounce 420ms var(--yt-ease-standard);
  }
  .p {
    position: absolute;
    left: 50%;
    top: 50%;
    width: 5px;
    height: 5px;
    margin: -2.5px;
    border-radius: 50%;
    background: var(--yt-accent);
    pointer-events: none;
    animation: fly 560ms cubic-bezier(0.22, 1, 0.36, 1) forwards;
  }
  .p:nth-child(odd) {
    background: color-mix(in srgb, var(--yt-accent) 60%, #ffd000);
  }
  .n {
    display: inline-flex;
    overflow: hidden;
    font-variant-numeric: tabular-nums;
  }
  .roll {
    animation: roll 180ms var(--yt-ease-standard);
  }
  @keyframes bounce {
    0% { transform: scale(1) rotate(0); }
    35% { transform: scale(1.25) rotate(-12deg); }
    70% { transform: scale(0.95) rotate(4deg); }
    100% { transform: scale(1) rotate(0); }
  }
  @keyframes fly {
    from {
      opacity: 1;
      transform: rotate(var(--a)) translateX(0) scale(1);
    }
    to {
      opacity: 0;
      transform: rotate(var(--a)) translateX(var(--d)) scale(0.5);
    }
  }
  @keyframes roll {
    from {
      transform: translateY(60%);
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .bounce,
    .p,
    .roll {
      animation: none;
    }
    .p {
      display: none;
    }
  }
</style>
