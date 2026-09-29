<script lang="ts">
  // "Why is it like this?": how the file is being played and what the
  // webview could not play directly.
  import { t } from "$lib/i18n";
  import { player } from "$lib/video/player.svelte";
  import VIcon from "./VIcon.svelte";

  let { onclose }: { onclose: () => void } = $props();

  let info = $derived(player.info);
  let res = $derived(info?.info.width && info?.info.height ? `${info.info.width}×${info.info.height}` : "—");
  let rate = $derived(info?.info.bitrate ? `${(info.info.bitrate / 1_000_000).toFixed(1)} Mb/s` : "—");
</script>

{#if info}
  <div class="info" role="dialog" aria-label={$t("video.player.why_title") as string}>
    <div class="head">
      <strong>{$t("video.player.why_title")}</strong>
      <button onclick={onclose} aria-label={$t("video.close") as string}><VIcon name="close" size={14} /></button>
    </div>
    <div class="mode mode-{info.mode}">{$t(`video.player.mode_${info.mode}`)}</div>
    <p class="desc">{$t(`video.player.mode_${info.mode}_desc`)}</p>
    {#if info.reason}<p class="reason">{info.reason}</p>{/if}
    <dl>
      <dt>{$t("video.player.container")}</dt>
      <dd>{info.info.container || "—"}</dd>
      <dt>{$t("video.player.video_codec")}</dt>
      <dd>{info.info.vcodec || "—"}</dd>
      <dt>{$t("video.player.audio_codec")}</dt>
      <dd>{info.info.acodec || "—"}</dd>
      <dt>{$t("video.player.resolution")}</dt>
      <dd>{res}</dd>
      <dt>{$t("video.player.bitrate")}</dt>
      <dd>{rate}</dd>
      <dt>{$t("video.player.source")}</dt>
      <dd>{$t(`video.player.source_${info.source.kind}`)}</dd>
      {#if player.tried.length}
        <dt>{$t("video.player.tried")}</dt>
        <dd>{player.tried.map((m) => $t(`video.player.mode_${m}`)).join(" → ")}</dd>
      {/if}
    </dl>
  </div>
{/if}

<style>
  .info {
    position: absolute;
    top: 64px;
    left: 20px;
    width: 300px;
    padding: 14px;
    border-radius: var(--radius-xl);
    background: color-mix(in srgb, var(--pl-shade) 66%, transparent);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: 0 0 0 var(--hairline) color-mix(in srgb, var(--pl-ink) 14%, transparent);
    color: var(--pl-ink);
    font-size: var(--text-sm);
    z-index: 15;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 8px;
  }
  .head button {
    all: unset;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    cursor: pointer;
  }
  .head button:hover {
    background: color-mix(in srgb, var(--pl-ink) 15%, transparent);
  }
  .head button:focus-visible {
    outline: var(--focus-ring);
  }
  .mode {
    display: inline-block;
    padding: 2px 10px;
    border-radius: var(--radius-full);
    font-weight: 700;
    font-size: var(--text-xs);
    background: color-mix(in srgb, var(--success) 30%, transparent);
  }
  .mode-remux,
  .mode-transcode_audio {
    background: color-mix(in srgb, var(--blue) 35%, transparent);
  }
  .mode-transcode {
    background: color-mix(in srgb, var(--warning) 35%, transparent);
  }
  .desc {
    margin: 8px 0 4px;
    line-height: 1.45;
  }
  .reason {
    margin: 0 0 8px;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    opacity: 0.8;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 12px;
    margin: 0;
  }
  dt {
    opacity: 0.65;
  }
  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
  }
</style>
