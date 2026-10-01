<script lang="ts">
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { t } from "$lib/i18n";
  import { closeSbToast, dismissAsk, redoSkip, skipAsked, undoSkip, vplayer } from "$lib/tube/video.svelte";
  import TIcon from "../TIcon.svelte";

  let { lifted = false }: { lifted?: boolean } = $props();

  let toast = $derived(vplayer.sbToast);
  let ask = $derived(vplayer.sbAsk);
  let full = $derived(vplayer.sbFull);
  let showFull = $derived(!!full && vplayer.position < 8);

  function hold(on: boolean) {
    if (vplayer.sbToast) vplayer.sbToast.hold = on;
  }
</script>

<div class="sbl" class:lifted>
  {#if toast}
    {#key toast.seg.uuid + String(toast.undone)}
      <div
        class="pill toast"
        role="status"
        aria-live="polite"
        onpointerenter={() => hold(true)}
        onpointerleave={() => hold(false)}
        onfocusin={() => hold(true)}
        onfocusout={() => hold(false)}
        in:fly={{ y: 12, duration: 220, easing: cubicOut }}
        out:fly={{ y: 12, duration: 160 }}
      >
        <span class="shield" style:--c="var(--sb-{toast.seg.category}, #fff)"><TIcon name="shield" size={16} /></span>
        <span class="txt">
          {toast.undone
            ? $t("tube.player.sb.undone", { name: $t(`tube.player.sb.cat.${toast.seg.category}`) })
            : $t("tube.player.sb.skipped", { name: $t(`tube.player.sb.cat.${toast.seg.category}`) })}
        </span>
        {#if toast.undone}
          <button class="act" onclick={redoSkip}><TIcon name="skip-forward" size={14} />{$t("tube.player.sb.skip_again")}</button>
        {:else}
          <button class="act" onclick={undoSkip}><TIcon name="undo" size={16} />{$t("tube.player.sb.undo")}<kbd>⏎</kbd></button>
        {/if}
        <button class="x" onclick={closeSbToast} aria-label={$t("tube.player.close") as string}><TIcon name="close" size={16} /></button>
        {#if !toast.undone}<span class="bar" style:transform="scaleX({toast.left / 5000})"></span>{/if}
      </div>
    {/key}
  {:else if showFull && full}
    <div class="pill full" in:fly={{ y: 12, duration: 220 }} out:fly={{ y: 12, duration: 160 }}>
      <span class="shield" style:--c="var(--sb-{full.category}, #fff)"><TIcon name="shield" size={16} /></span>
      <span class="txt">{$t("tube.player.sb.full", { name: $t(`tube.player.sb.cat.${full.category}`) })}</span>
    </div>
  {/if}
</div>

{#if ask}
  <div class="sbr" class:lifted in:fly={{ x: 24, duration: 220, easing: cubicOut }} out:fly={{ x: 24, duration: 160 }}>
    <button class="skip" onclick={skipAsked}>
      <i class="dot" style:background="var(--sb-{ask.category}, #fff)"></i>
      {$t("tube.player.sb.skip_prompt", { name: $t(`tube.player.sb.cat.${ask.category}`) })}
      <TIcon name="skip-forward" size={16} />
      <kbd>⏎</kbd>
    </button>
    <button class="x" onclick={dismissAsk} aria-label={$t("tube.player.close") as string}><TIcon name="close" size={16} /></button>
  </div>
{/if}

<style>
  .sbl,
  .sbr {
    position: absolute;
    bottom: 24px;
    z-index: 6;
    transition: bottom 250ms var(--yt-ease-standard);
  }
  .sbl {
    left: 16px;
  }
  .sbr {
    right: 16px;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .sbl.lifted,
  .sbr.lifted {
    bottom: 100px;
  }
  .pill {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    height: 44px;
    padding: 0 6px 0 8px;
    border-radius: 22px;
    overflow: hidden;
    color: #fff;
    background: rgba(15, 15, 15, 0.72);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.35);
    font: 500 14px/20px var(--yt-font);
    white-space: nowrap;
  }
  .full {
    padding-right: 16px;
  }
  .shield {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    color: #000;
    background: var(--c);
    flex: none;
  }
  .txt {
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .act {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 12px;
    border: 0;
    border-radius: 16px;
    color: #fff;
    background: rgba(255, 255, 255, 0.14);
    font: 500 13px/1 var(--yt-font);
    cursor: pointer;
    transition: background var(--yt-dur-base) var(--yt-ease-standard);
  }
  .act:hover {
    background: rgba(255, 255, 255, 0.24);
  }
  kbd {
    font: 500 11px/1 var(--yt-font);
    opacity: 0.6;
  }
  .x {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: 0;
    border-radius: 50%;
    color: rgba(255, 255, 255, 0.8);
    background: transparent;
    cursor: pointer;
  }
  .x:hover {
    background: rgba(255, 255, 255, 0.12);
    color: #fff;
  }
  .act:focus-visible,
  .x:focus-visible,
  .skip:focus-visible {
    outline: 2px solid #fff;
    outline-offset: 1px;
  }
  .bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 2px;
    background: rgba(255, 255, 255, 0.7);
    transform-origin: left;
    transition: transform 100ms linear;
  }
  .skip {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    height: 44px;
    padding: 0 18px;
    border: 0;
    border-radius: 22px;
    color: #fff;
    background: rgba(15, 15, 15, 0.72);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.35);
    font: 500 15px/1 var(--yt-font);
    cursor: pointer;
    transition: background var(--yt-dur-base) var(--yt-ease-standard);
  }
  .skip:hover {
    background: rgba(40, 40, 40, 0.85);
  }
  .sbr .x {
    background: rgba(15, 15, 15, 0.6);
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
  }
  @media (prefers-reduced-motion: reduce) {
    .sbl,
    .sbr,
    .bar {
      transition: none;
    }
  }
</style>
