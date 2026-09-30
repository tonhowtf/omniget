<script lang="ts">
  // Gear menu (spec §2.7): glass popup with 40px rows; submenus slide in
  // sideways while the box animates to the new height.
  import { cubicOut } from "svelte/easing";
  import { fly } from "svelte/transition";
  import { locale, t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { qualityBadge, qualityLabel } from "$lib/tube/dash";
  import { fmtDuration } from "$lib/tube/format";
  import { patchTubePrefs, tubePrefs } from "$lib/tube/prefs.svelte";
  import {
    RATES,
    captionKey,
    channelWhitelisted,
    downloadCurrent,
    seek,
    setAmbient,
    setCaptions,
    setQuality,
    setRate,
    setTrack,
    vplayer,
  } from "$lib/tube/video.svelte";
  import TIcon from "../TIcon.svelte";

  let { onclose }: { onclose: () => void } = $props();

  type Panel = "main" | "quality" | "speed" | "captions" | "translate" | "audio" | "sb";
  let panel = $state<Panel>("main");
  let dir = $state(1);
  let innerH = $state(0);

  const TRANSLATE = ["en", "pt", "es", "fr", "de", "it", "ja", "ko", "zh-Hans", "ru", "ar", "hi", "id", "tr"];

  function open(p: Panel) {
    dir = p === "main" ? -1 : 1;
    panel = p;
  }

  let langNames = $derived.by(() => {
    try {
      return new Intl.DisplayNames([$locale || "en"], { type: "language" });
    } catch {
      return null;
    }
  });
  function langName(code: string | null | undefined): string {
    if (!code) return "";
    try {
      return langNames?.of(code) ?? code;
    } catch {
      return code;
    }
  }

  let set = $derived(vplayer.streams);
  let captions = $derived(set?.captions ?? []);
  let current = $derived(vplayer.ladder.find((f) => f.itag === vplayer.videoItag) ?? null);
  let renderedH = $derived(vplayer.height);
  let qualityText = $derived(
    vplayer.autoQuality
      ? `${$t("tube.player.auto")}${renderedH ? ` (${renderedH}p)` : ""}`
      : current
        ? qualityLabel(current)
        : $t("tube.player.auto"),
  );
  let capText = $derived.by(() => {
    const c = captions.find((x) => captionKey(x) === vplayer.captions);
    if (!c) return $t("tube.player.off") as string;
    const base = c.name || langName(c.lang);
    return vplayer.translate ? `${base} >> ${langName(vplayer.translate)}` : base;
  });
  let rateText = $derived(vplayer.rate === 1 ? ($t("tube.player.normal") as string) : `${vplayer.rate}`);
  let tracks = $derived(vplayer.tracks);
  let trackText = $derived.by(() => {
    const tr = tracks.find((x) => x.key === vplayer.track) ?? tracks.find((x) => x.original) ?? tracks[0];
    if (!tr) return "";
    return `${tr.name || langName(tr.lang) || ($t("tube.player.default_track") as string)}${tr.original ? ` · ${$t("tube.player.original")}` : ""}`;
  });
  let prefs = $derived(tubePrefs());
  let segs = $derived(vplayer.segments.filter((s) => s.action !== "chapter").sort((a, b) => a.start_s - b.start_s));
  let whitelisted = $derived(channelWhitelisted());

  async function download() {
    onclose();
    try {
      await downloadCurrent(false);
      showToast("success", $t("tube.player.download_queued") as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function toggleWhitelist() {
    const ch = vplayer.info?.channel?.id;
    if (!ch) return;
    const list = prefs.sb_whitelist.filter((x) => x !== ch);
    if (!whitelisted) list.push(ch);
    void patchTubePrefs({ sb_whitelist: list });
  }
</script>

<div class="menu" role="menu" tabindex="-1" style:height={innerH ? `${innerH}px` : undefined} onkeydown={(e) => e.key === "Escape" && (e.stopPropagation(), panel === "main" ? onclose() : open("main"))}>
  {#key panel}
    <div class="inner" bind:offsetHeight={innerH} in:fly={{ x: 60 * dir, duration: 250, easing: cubicOut }}>
      {#if panel === "main"}
        {#if !vplayer.short}
          <button class="row" role="menuitemcheckbox" aria-checked={vplayer.ambient} onclick={() => setAmbient(!vplayer.ambient)}>
            <TIcon name="sparkles" size={22} />
            <span class="lbl">{$t("tube.player.ambient")}</span>
            <span class="switch" class:on={vplayer.ambient}><i></i></span>
          </button>
        {/if}
        {#if captions.length}
          <button class="row" role="menuitem" onclick={() => open("captions")}>
            <TIcon name="captions" size={22} />
            <span class="lbl">{$t("tube.player.captions")}</span>
            <span class="val">{capText}</span>
            <TIcon name="chevron-right" size={18} />
          </button>
        {/if}
        <button class="row" role="menuitem" onclick={() => open("speed")}>
          <TIcon name="speed" size={22} />
          <span class="lbl">{$t("tube.player.speed")}</span>
          <span class="val">{rateText}</span>
          <TIcon name="chevron-right" size={18} />
        </button>
        {#if tracks.length > 1}
          <button class="row" role="menuitem" onclick={() => open("audio")}>
            <TIcon name="music" size={22} />
            <span class="lbl">{$t("tube.player.audio_track")}</span>
            <span class="val">{trackText}</span>
            <TIcon name="chevron-right" size={18} />
          </button>
        {/if}
        {#if prefs.sponsorblock && (segs.length || vplayer.info?.channel?.id)}
          <button class="row" role="menuitem" onclick={() => open("sb")}>
            <TIcon name="shield" size={22} />
            <span class="lbl">SponsorBlock</span>
            <span class="val">{$t("tube.player.sb.segments", { count: segs.length })}</span>
            <TIcon name="chevron-right" size={18} />
          </button>
        {/if}
        {#if vplayer.ladder.length}
          <button class="row" role="menuitem" onclick={() => open("quality")}>
            <TIcon name="quality" size={22} />
            <span class="lbl">{$t("tube.player.quality")}</span>
            <span class="val">{qualityText}</span>
            <TIcon name="chevron-right" size={18} />
          </button>
        {/if}
      {:else}
        <button class="head" onclick={() => open(panel === "translate" ? "captions" : "main")}>
          <TIcon name="chevron-left" size={20} />
          <span>
            {#if panel === "quality"}{$t("tube.player.quality")}
            {:else if panel === "speed"}{$t("tube.player.speed")}
            {:else if panel === "captions"}{$t("tube.player.captions")}
            {:else if panel === "translate"}{$t("tube.player.translate")}
            {:else if panel === "audio"}{$t("tube.player.audio_track")}
            {:else}SponsorBlock{/if}
          </span>
        </button>
        <div class="list">
          {#if panel === "quality"}
            {#each vplayer.ladder as f (f.itag)}
              {@const on = !vplayer.autoQuality && vplayer.videoItag === f.itag}
              <button class="opt" role="menuitemradio" aria-checked={on} onclick={() => (setQuality(f.itag), onclose())}>
                <span class="chk">{#if on}<TIcon name="check" size={18} />{/if}</span>
                <span>{qualityLabel(f)}</span>
                {#if qualityBadge(f.height)}<sup class="badge">{qualityBadge(f.height)}</sup>{/if}
              </button>
            {/each}
            <button class="opt" role="menuitemradio" aria-checked={vplayer.autoQuality} onclick={() => (setQuality(null), onclose())}>
              <span class="chk">{#if vplayer.autoQuality}<TIcon name="check" size={18} />{/if}</span>
              <span>{$t("tube.player.auto")}{#if vplayer.autoQuality && renderedH}<span class="dim"> ({renderedH}p)</span>{/if}</span>
            </button>
            <div class="sep"></div>
            <button class="opt dl" onclick={download}>
              <span class="chk"><TIcon name="download" size={18} /></span>
              <span>{$t("tube.player.download_quality")}</span>
            </button>
          {:else if panel === "speed"}
            {#each RATES as r (r)}
              <button class="opt" role="menuitemradio" aria-checked={vplayer.rate === r} onclick={() => (setRate(r), open("main"))}>
                <span class="chk">{#if vplayer.rate === r}<TIcon name="check" size={18} />{/if}</span>
                <span>{r === 1 ? $t("tube.player.normal") : r}</span>
              </button>
            {/each}
          {:else if panel === "captions"}
            <button class="opt" role="menuitemradio" aria-checked={!vplayer.captions} onclick={() => (void setCaptions(null), open("main"))}>
              <span class="chk">{#if !vplayer.captions}<TIcon name="check" size={18} />{/if}</span>
              <span>{$t("tube.player.off")}</span>
            </button>
            {#each captions as c (captionKey(c))}
              {@const on = vplayer.captions === captionKey(c)}
              <button class="opt" role="menuitemradio" aria-checked={on} onclick={() => (void setCaptions(captionKey(c), true, null), open("main"))}>
                <span class="chk">{#if on}<TIcon name="check" size={18} />{/if}</span>
                <span>{c.name || langName(c.lang)}{#if c.auto}<span class="dim"> ({$t("tube.player.auto_generated")})</span>{/if}</span>
              </button>
            {/each}
            {#if vplayer.captions}
              <div class="sep"></div>
              <button class="opt" onclick={() => open("translate")}>
                <span class="chk"><TIcon name="sparkles" size={18} /></span>
                <span class="grow">{$t("tube.player.translate")}</span>
                <TIcon name="chevron-right" size={18} />
              </button>
            {/if}
          {:else if panel === "translate"}
            <button class="opt" role="menuitemradio" aria-checked={!vplayer.translate} onclick={() => (void setCaptions(vplayer.captions, false, null), onclose())}>
              <span class="chk">{#if !vplayer.translate}<TIcon name="check" size={18} />{/if}</span>
              <span>{$t("tube.player.off")}</span>
            </button>
            {#each TRANSLATE as l (l)}
              <button class="opt" role="menuitemradio" aria-checked={vplayer.translate === l} onclick={() => (void setCaptions(vplayer.captions, false, l), onclose())}>
                <span class="chk">{#if vplayer.translate === l}<TIcon name="check" size={18} />{/if}</span>
                <span>{langName(l)}</span>
              </button>
            {/each}
          {:else if panel === "audio"}
            {#each tracks as tr (tr.key)}
              {@const on = (vplayer.track ?? tracks.find((x) => x.original)?.key ?? tracks[0]?.key) === tr.key}
              <button class="opt" role="menuitemradio" aria-checked={on} onclick={() => (void setTrack(tr.original ? null : tr.key), onclose())}>
                <span class="chk">{#if on}<TIcon name="check" size={18} />{/if}</span>
                <span>{tr.name || langName(tr.lang) || $t("tube.player.default_track")}{#if tr.original}<span class="dim"> · {$t("tube.player.original")}</span>{/if}</span>
              </button>
            {/each}
          {:else}
            <button class="opt" role="menuitemcheckbox" aria-checked={!vplayer.sbOffHere} onclick={() => (vplayer.sbOffHere = !vplayer.sbOffHere)}>
              <span class="chk"><TIcon name="skip-forward" size={18} /></span>
              <span class="grow">{$t("tube.player.sb.skip_here")}</span>
              <span class="switch" class:on={!vplayer.sbOffHere}><i></i></span>
            </button>
            {#if vplayer.info?.channel?.id}
              <button class="opt" role="menuitemcheckbox" aria-checked={whitelisted} onclick={toggleWhitelist}>
                <span class="chk"><TIcon name="shield" size={18} /></span>
                <span class="grow">{$t("tube.player.sb.whitelist")}</span>
                <span class="switch" class:on={whitelisted}><i></i></span>
              </button>
            {/if}
            {#if segs.length}
              <div class="sep"></div>
              {#each segs as s (s.uuid)}
                <button class="opt seg" onclick={() => (seek(s.start_s), onclose())}>
                  <span class="chk"><i class="dot" style:background="var(--sb-{s.category}, #fff)"></i></span>
                  <span class="grow">{$t(`tube.player.sb.cat.${s.category}`)}</span>
                  <span class="dim">{fmtDuration(s.start_s)}{#if s.action !== "poi"}–{fmtDuration(s.end_s)}{/if}</span>
                </button>
              {/each}
            {/if}
          {/if}
        </div>
      {/if}
    </div>
  {/key}
</div>

<style>
  .menu {
    position: absolute;
    right: 12px;
    bottom: 84px;
    width: 300px;
    max-height: calc(100% - 100px);
    overflow: hidden;
    border-radius: var(--yt-radius-menu);
    background: rgba(28, 28, 28, 0.72);
    backdrop-filter: blur(16px) saturate(1.4);
    -webkit-backdrop-filter: blur(16px) saturate(1.4);
    color: var(--yt-ov-text);
    text-shadow: 0 0 2px rgba(0, 0, 0, 0.5);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
    transition: height 250ms var(--yt-ease-standard);
    animation: pop var(--yt-dur-fast) var(--yt-ease-enter);
    z-index: 5;
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
  .inner {
    padding: 8px 0;
  }
  .row,
  .opt,
  .head {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    min-height: 40px;
    padding: 0 16px;
    border: 0;
    background: transparent;
    color: inherit;
    font: 500 14px/20px var(--yt-font);
    text-align: left;
    cursor: pointer;
  }
  .row:hover,
  .opt:hover,
  .head:hover,
  .row:focus-visible,
  .opt:focus-visible,
  .head:focus-visible {
    background: rgba(255, 255, 255, 0.1);
    outline: none;
  }
  .lbl {
    flex: 1;
  }
  .val {
    max-width: 130px;
    color: var(--yt-ov-text-2);
    font-weight: 400;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .head {
    gap: 8px;
    padding-left: 10px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.15);
    margin-bottom: 6px;
    min-height: 48px;
  }
  .list {
    max-height: min(360px, 60vh);
    overflow-y: auto;
    scrollbar-width: thin;
  }
  .opt {
    font-weight: 400;
  }
  .chk {
    width: 22px;
    display: grid;
    place-items: center;
    flex: none;
  }
  .grow {
    flex: 1;
  }
  .dim {
    color: var(--yt-ov-text-2);
  }
  .badge {
    margin-left: -8px;
    align-self: flex-start;
    margin-top: 6px;
    padding: 0 2px;
    border-radius: 2px;
    font: 700 9px/12px var(--yt-font);
    background: var(--yt-accent);
    color: var(--yt-on-accent);
  }
  .sep {
    height: 1px;
    margin: 6px 0;
    background: rgba(255, 255, 255, 0.15);
  }
  .dl {
    color: #fff;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
  }
  .switch {
    position: relative;
    width: 36px;
    height: 14px;
    border-radius: 7px;
    background: rgba(255, 255, 255, 0.3);
    flex: none;
    transition: background var(--yt-dur-base);
  }
  .switch i {
    position: absolute;
    top: -3px;
    left: -2px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: #bdbdbd;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
    transition:
      transform var(--yt-dur-base) var(--yt-ease-standard),
      background var(--yt-dur-base);
  }
  .switch.on {
    background: color-mix(in srgb, var(--yt-accent) 55%, transparent);
  }
  .switch.on i {
    transform: translateX(20px);
    background: var(--yt-accent);
  }
  @media (prefers-reduced-motion: reduce) {
    .menu {
      transition: none;
      animation: none;
    }
  }
</style>
