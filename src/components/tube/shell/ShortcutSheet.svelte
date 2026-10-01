<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { t } from "$lib/i18n";
  import { keys } from "$lib/tube/keys.svelte";
  import TIcon from "../TIcon.svelte";

  const ROWS: [string, string][] = [
    ["K / Space", "tube.keys.play"],
    ["J / L", "tube.keys.seek10"],
    ["← / →", "tube.keys.seek5"],
    ["↑ / ↓", "tube.keys.volume"],
    ["M", "tube.keys.mute"],
    ["F", "tube.keys.fullscreen"],
    ["T", "tube.keys.theater"],
    ["I", "tube.keys.mini"],
    ["C", "tube.keys.captions"],
    ["< / >", "tube.keys.speed"],
    ["0–9", "tube.keys.jump"],
    ["Shift+N / Shift+P", "tube.keys.next_prev"],
    ["Enter", "tube.keys.sponsor"],
    ["/", "tube.keys.search"],
    ["Alt+←", "tube.keys.back"],
  ];
</script>

<div class="scrim" role="presentation" onclick={() => (keys.help = false)} transition:fade={{ duration: 150 }}></div>
<div class="sheet yt-menu" role="dialog" aria-modal="true" aria-label={$t("tube.keys.title") as string} transition:scale={{ start: 0.96, duration: 150 }}>
  <div class="hd">
    <TIcon name="keyboard" size={22} />
    <h2>{$t("tube.keys.title")}</h2>
    <button class="yt-btn yt-btn--icon" onclick={() => (keys.help = false)} aria-label={$t("tube.player.close") as string}><TIcon name="close" size={20} /></button>
  </div>
  <dl>
    {#each ROWS as [k, label] (k)}
      <dt><kbd>{k}</kbd></dt>
      <dd>{$t(label)}</dd>
    {/each}
  </dl>
</div>

<style>
  .scrim {
    position: absolute;
    inset: 0;
    background: var(--yt-scrim);
    z-index: 90;
  }
  .sheet {
    position: absolute;
    left: 50%;
    top: 50%;
    translate: -50% -50%;
    width: min(520px, calc(100% - 32px));
    max-height: calc(100% - 64px);
    overflow-y: auto;
    padding: 8px 0 16px;
    z-index: 91;
  }
  .hd {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 4px 8px 8px 20px;
  }
  h2 {
    flex: 1;
    margin: 0;
    font: var(--yt-type-headline-s);
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 10px 20px;
    margin: 0;
    padding: 8px 20px 0;
  }
  dt {
    text-align: right;
  }
  dd {
    margin: 0;
    font: var(--yt-type-meta);
    color: var(--yt-text-2);
    align-self: center;
  }
  kbd {
    display: inline-block;
    padding: 2px 8px;
    border-radius: 6px;
    background: var(--yt-tonal);
    font: 500 13px/20px var(--yt-font);
    white-space: nowrap;
  }
</style>
