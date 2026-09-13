<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import SkipGapsButton from "./SkipGapsButton.svelte";
  import SponsorBlockOverlay from "./SponsorBlockOverlay.svelte";
  import ChaptersList from "./ChaptersList.svelte";
  import ThumbnailScrubber from "./ThumbnailScrubber.svelte";
  import type {
    SubtitleTrack,
    AudioTrack,
    SkipGaps,
    ThumbnailSlice,
    StudySettings,
    SponsorBlockSegment,
    YoutubeChapter,
  } from "$lib/study-bridge";

  type Props = {
    videoSrc: string;
    title: string;
    courseTitle: string;
    backHref: string;
    durationMs: number | null;
    initialSeconds: number;
    initialPlaybackSpeed: number;
    subtitles: SubtitleTrack[];
    audioTracks: AudioTrack[];
    skipGaps: SkipGaps | null;
    thumbnails: ThumbnailSlice[];
    sponsorBlockSegments?: SponsorBlockSegment[];
    sponsorBlockAutoSkip?: boolean;
    chapters?: YoutubeChapter[];
    settings: StudySettings["player"] | null;
    selectedSubtitleLang: string | null;
    selectedAudioLang: string | null;
    theaterMode: boolean;
    onTimeUpdate: (cur: number, dur: number) => void;
    onSeek: (fromMs: number, toMs: number) => void;
    onPlay: () => void;
    onPause: () => void;
    onEnded: () => void;
    onLoadedMetadata?: () => void;
    onSubtitleChange: (lang: string | null) => void;
    onAudioChange: (lang: string | null) => void;
    onSpeedChange: (speed: number) => void;
    onSkipGap: (toMs: number, kind: "intro" | "outro") => void;
    onTheaterToggle: () => void;
    onClose: () => void;
    onVideoEl?: (el: HTMLVideoElement | null) => void;
  };

  let {
    videoSrc,
    title,
    courseTitle,
    backHref,
    durationMs,
    initialSeconds,
    initialPlaybackSpeed,
    subtitles,
    audioTracks,
    skipGaps,
    thumbnails,
    sponsorBlockSegments = [],
    sponsorBlockAutoSkip = false,
    chapters = [],
    settings,
    selectedSubtitleLang,
    selectedAudioLang,
    theaterMode,
    onTimeUpdate,
    onSeek,
    onPlay,
    onPause,
    onEnded,
    onLoadedMetadata,
    onSubtitleChange,
    onAudioChange,
    onSpeedChange,
    onSkipGap,
    onTheaterToggle,
    onClose,
    onVideoEl,
  }: Props = $props();

  $effect(() => {
    onVideoEl?.(videoEl);
  });

  let videoEl = $state<HTMLVideoElement | null>(null);
  let progressEl = $state<HTMLDivElement | null>(null);
  let shellEl = $state<HTMLDivElement | null>(null);
  let currentTime = $state(0);
  let duration = $state(0);
  let buffered = $state(0);
  let paused = $state(true);
  let muted = $state(false);
  let volume = $state(1);
  let volumePopoverOpen = $state(false);
  let isFullscreen = $state(false);
  let isLoading = $state(true);
  let lastSeekFromMs = 0;
  let controlsVisible = $state(true);
  let idleTimer: ReturnType<typeof setTimeout> | null = null;
  let progressHovering = $state(false);

  const currentTimeMs = $derived(Math.round(currentTime * 1000));
  const progressPct = $derived(duration > 0 ? (currentTime / duration) * 100 : 0);
  const bufferedPct = $derived(duration > 0 ? (buffered / duration) * 100 : 0);

  const subtitleResolved = $derived.by(() => {
    if (selectedSubtitleLang == null) return null;
    const track = subtitles.find((s) => s.lang === selectedSubtitleLang);
    return track ? convertFileSrc(track.path) : null;
  });

  const introMarker = $derived.by(() => {
    if (duration <= 0 || !skipGaps?.intro_from_ms || !skipGaps?.intro_to_ms) return null;
    const startPct = (skipGaps.intro_from_ms / 1000 / duration) * 100;
    const endPct = (skipGaps.intro_to_ms / 1000 / duration) * 100;
    return { startPct, widthPct: Math.max(0.5, endPct - startPct) };
  });

  const outroMarker = $derived.by(() => {
    if (duration <= 0 || !skipGaps?.outro_from_ms) return null;
    const startPct = (skipGaps.outro_from_ms / 1000 / duration) * 100;
    return { startPct, widthPct: Math.max(0.5, 100 - startPct) };
  });

  function fmtTime(s: number): string {
    if (!isFinite(s) || s < 0) return "0:00";
    const total = Math.floor(s);
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const sec = total % 60;
    if (h > 0) {
      return `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
    }
    return `${m}:${String(sec).padStart(2, "0")}`;
  }

  function showControlsForAWhile() {
    controlsVisible = true;
    if (idleTimer) clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
      if (!paused && !progressHovering && !volumePopoverOpen) {
        controlsVisible = false;
      }
    }, 3000);
  }

  function onMouseMove() {
    showControlsForAWhile();
  }

  function onMouseLeaveShell() {
    scheduleCloseVolumePop(0);
    toolbarPickerOpen = null;
    if (!paused) {
      if (idleTimer) clearTimeout(idleTimer);
      idleTimer = setTimeout(() => {
        if (!progressHovering) controlsVisible = false;
      }, 600);
    }
  }

  $effect(() => {
    if (paused) controlsVisible = true;
  });

  function togglePlay() {
    if (!videoEl) return;
    if (videoEl.paused) void videoEl.play().catch(() => {});
    else videoEl.pause();
  }

  function skipBy(seconds: number) {
    if (!videoEl) return;
    const next = Math.max(0, Math.min(videoEl.duration || 0, videoEl.currentTime + seconds));
    videoEl.currentTime = next;
  }

  function toggleMute() {
    if (!videoEl) return;
    videoEl.muted = !videoEl.muted;
    muted = videoEl.muted;
  }

  function setVolume(v: number) {
    if (!videoEl) return;
    videoEl.volume = Math.max(0, Math.min(1, v));
    if (videoEl.volume > 0 && videoEl.muted) videoEl.muted = false;
    volume = videoEl.volume;
    muted = videoEl.muted;
  }

  function toggleFullscreen() {
    if (!shellEl) return;
    if (document.fullscreenElement) {
      document.exitFullscreen().catch(() => {});
    } else {
      shellEl.requestFullscreen().catch(() => {});
    }
  }

  function onProgressClick(e: MouseEvent) {
    if (!progressEl || !videoEl || duration <= 0) return;
    const rect = progressEl.getBoundingClientRect();
    const ratio = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    videoEl.currentTime = ratio * duration;
  }

  function onTimeUpdateInternal() {
    if (!videoEl) return;
    currentTime = videoEl.currentTime;
    duration = videoEl.duration || 0;
    if (videoEl.buffered && videoEl.buffered.length > 0) {
      buffered = videoEl.buffered.end(videoEl.buffered.length - 1);
    }
    onTimeUpdate(currentTime, duration);
    const curMs = Math.round(currentTime * 1000);
    if (Math.abs(curMs - lastSeekFromMs) < 1500) {
      lastSeekFromMs = curMs;
    }
  }

  function onSeekedInternal() {
    if (!videoEl) return;
    const toMs = Math.round(videoEl.currentTime * 1000);
    const fromMs = lastSeekFromMs;
    if (Math.abs(toMs - fromMs) >= 1500) {
      onSeek(fromMs, toMs);
    }
    lastSeekFromMs = toMs;
  }

  function onPlayInternal() {
    paused = false;
    onPlay();
    showControlsForAWhile();
  }

  function onPauseInternal() {
    paused = true;
    onPause();
    controlsVisible = true;
  }

  function onLoadedMetadataInternal() {
    if (!videoEl) return;
    duration = videoEl.duration || 0;
    if (initialSeconds > 0 && initialSeconds < 86400) {
      videoEl.currentTime = initialSeconds;
      lastSeekFromMs = Math.round(initialSeconds * 1000);
    }
    videoEl.playbackRate = initialPlaybackSpeed;
    isLoading = false;
    muted = videoEl.muted;
    volume = videoEl.volume;
    onLoadedMetadata?.();
  }

  function onWaiting() {
    isLoading = true;
  }

  function onPlaying() {
    isLoading = false;
  }

  $effect(() => {
    if (!videoEl) return;
    videoEl.playbackRate = initialPlaybackSpeed;
  });

  $effect(() => {
    function onFsChange() {
      isFullscreen = !!document.fullscreenElement;
    }
    document.addEventListener("fullscreenchange", onFsChange);
    return () => document.removeEventListener("fullscreenchange", onFsChange);
  });

  $effect(() => {
    if (!settings?.pause_on_minimize) return;
    function onVisibility() {
      if (document.hidden && videoEl && !videoEl.paused) {
        videoEl.pause();
      }
    }
    document.addEventListener("visibilitychange", onVisibility);
    return () => document.removeEventListener("visibilitychange", onVisibility);
  });

  let toolbarPickerOpen = $state<"subs" | "audio" | "speed" | null>(null);
  let chaptersOpen = $state(false);
  const SPEED_OPTIONS = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
  let volumeCloseTimer: ReturnType<typeof setTimeout> | null = null;

  function openVolumePop() {
    if (volumeCloseTimer) {
      clearTimeout(volumeCloseTimer);
      volumeCloseTimer = null;
    }
    volumePopoverOpen = true;
  }

  function scheduleCloseVolumePop(delay: number = 240) {
    if (volumeCloseTimer) clearTimeout(volumeCloseTimer);
    volumeCloseTimer = setTimeout(() => {
      volumePopoverOpen = false;
      volumeCloseTimer = null;
    }, delay);
  }

  function pickerToggle(which: "subs" | "audio" | "speed") {
    toolbarPickerOpen = toolbarPickerOpen === which ? null : which;
  }

  function fmtSpeed(v: number): string {
    return Number.isInteger(v) ? `${v}×` : `${v}×`;
  }

  function pickSpeed(v: number) {
    onSpeedChange(v);
    toolbarPickerOpen = null;
  }

  function pickSubtitle(lang: string | null) {
    onSubtitleChange(lang);
    toolbarPickerOpen = null;
  }

  function pickAudio(lang: string | null) {
    onAudioChange(lang);
    toolbarPickerOpen = null;
  }

  function onPopoverDocClick(e: MouseEvent) {
    if (toolbarPickerOpen == null) return;
    const target = e.target as HTMLElement;
    if (!target.closest(".picker-pop") && !target.closest(".icon-btn")) {
      toolbarPickerOpen = null;
    }
  }

  $effect(() => {
    if (toolbarPickerOpen == null) return;
    document.addEventListener("click", onPopoverDocClick);
    return () => document.removeEventListener("click", onPopoverDocClick);
  });
</script>

<div
  class="shell"
  class:controls-visible={controlsVisible}
  class:paused
  class:loading={isLoading}
  class:fullscreen={isFullscreen}
  class:theater={theaterMode}
  bind:this={shellEl}
  onmousemove={onMouseMove}
  onmouseleave={onMouseLeaveShell}
  role="region"
  aria-label={$t("study.player.shell.aria")}
>
  {#key videoSrc}
    <!-- svelte-ignore a11y_media_has_caption -->
    <video
      bind:this={videoEl}
      src={videoSrc}
      crossorigin="anonymous"
      playsinline
      preload="metadata"
      disablepictureinpicture
      disableremoteplayback
      controlslist="nodownload nofullscreen noremoteplayback noplaybackrate"
      ontimeupdate={onTimeUpdateInternal}
      onseeked={onSeekedInternal}
      onplay={onPlayInternal}
      onpause={onPauseInternal}
      onended={onEnded}
      onloadedmetadata={onLoadedMetadataInternal}
      onwaiting={onWaiting}
      onplaying={onPlaying}
      onclick={togglePlay}
      ondblclick={toggleFullscreen}
    >
      {#if subtitleResolved}
        <track
          kind="captions"
          src={subtitleResolved}
          srclang={selectedSubtitleLang ?? undefined}
          label={subtitles.find((s) => s.lang === selectedSubtitleLang)?.label ?? "Legenda"}
          default
        />
      {/if}
    </video>
  {/key}

  {#if isLoading}
    <div class="spinner" aria-hidden="true">
      <div class="ring"></div>
    </div>
  {/if}

  {#if paused && !isLoading}
    <button type="button" class="big-play" onclick={togglePlay} aria-label="Reproduzir">
      <svg viewBox="0 0 24 24" width="32" height="32" fill="currentColor" aria-hidden="true">
        <polygon points="6,4 20,12 6,20" />
      </svg>
    </button>
  {/if}

  <SkipGapsButton
    {skipGaps}
    {currentTimeMs}
    bingeAutoSkip={settings?.binge_watching ?? false}
    onSkip={onSkipGap}
  />

  <SponsorBlockOverlay
    segments={sponsorBlockSegments}
    {currentTimeMs}
    autoSkip={sponsorBlockAutoSkip}
    onSkip={(toMs) => {
      if (videoEl) videoEl.currentTime = toMs / 1000;
    }}
  />

  {#if chapters.length > 0 && chaptersOpen}
    <aside class="chapters-drawer" aria-label={$t("study.player.shell.chapters")}>
      <header class="chapters-drawer__header">
        <span>Capítulos</span>
        <button type="button" class="icon-btn" onclick={() => (chaptersOpen = false)} aria-label={$t("study.player.shell.close_chapters")}>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </header>
      <div class="chapters-drawer__body">
        <ChaptersList
          {chapters}
          {currentTimeMs}
          onJump={(toMs) => {
            if (videoEl) videoEl.currentTime = toMs / 1000;
          }}
        />
      </div>
    </aside>
  {/if}

  <header class="top-bar">
    <a class="back" href={backHref} aria-label="Voltar">
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <line x1="19" y1="12" x2="5" y2="12" />
        <polyline points="12 19 5 12 12 5" />
      </svg>
    </a>
    <div class="title-block">
      <span class="course-title">{courseTitle}</span>
      <span class="lesson-title">{title}</span>
    </div>
    <button type="button" class="icon-btn" onclick={onClose} aria-label="Fechar">
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <line x1="18" y1="6" x2="6" y2="18" />
        <line x1="6" y1="6" x2="18" y2="18" />
      </svg>
    </button>
  </header>

  <div class="scrim-bottom" aria-hidden="true"></div>

  <footer class="bottom-bar">
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="progress-wrap"
      bind:this={progressEl}
      onclick={onProgressClick}
      onmouseenter={() => (progressHovering = true)}
      onmouseleave={() => (progressHovering = false)}
      role="slider"
      tabindex="-1"
      aria-label="Progresso"
      aria-valuemin="0"
      aria-valuemax={duration}
      aria-valuenow={currentTime}
    >
      <div class="progress-track">
        <div class="progress-buffered" style:width="{bufferedPct}%"></div>
        {#if introMarker}
          <div
            class="gap-marker intro"
            style:left="{introMarker.startPct}%"
            style:width="{introMarker.widthPct}%"
            title="Intro"
          ></div>
        {/if}
        {#if outroMarker}
          <div
            class="gap-marker outro"
            style:left="{outroMarker.startPct}%"
            style:width="{outroMarker.widthPct}%"
            title={$t("study.player.shell.credits")}
          ></div>
        {/if}
        <div class="progress-fill" style:width="{progressPct}%"></div>
        <div class="progress-thumb" style:left="{progressPct}%"></div>
      </div>
      {#if thumbnails.length > 0 && progressEl}
        <ThumbnailScrubber
          {thumbnails}
          {duration}
          target={progressEl}
        />
      {/if}
    </div>

    <div class="controls">
      <div class="left-group">
        <button type="button" class="icon-btn primary" onclick={togglePlay} aria-label={paused ? "Reproduzir" : "Pausar"}>
          {#if paused}
            <svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor" aria-hidden="true">
              <polygon points="6,4 20,12 6,20" />
            </svg>
          {:else}
            <svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor" aria-hidden="true">
              <rect x="6" y="4" width="4" height="16" />
              <rect x="14" y="4" width="4" height="16" />
            </svg>
          {/if}
        </button>
        <button type="button" class="icon-btn" onclick={() => skipBy(-10)} aria-label="Voltar 10s">
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <polyline points="11 17 6 12 11 7" />
            <path d="M6 12h12a3 3 0 0 1 0 6h-3" />
          </svg>
        </button>
        <button type="button" class="icon-btn" onclick={() => skipBy(10)} aria-label={$t("study.player.keys.fwd_10")}>
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <polyline points="13 17 18 12 13 7" />
            <path d="M18 12H6a3 3 0 0 0 0 6h3" />
          </svg>
        </button>
        <div
          class="volume-wrap"
          role="group"
          aria-label="Volume"
          onmouseenter={openVolumePop}
          onmouseleave={() => scheduleCloseVolumePop()}
        >
          <button
            type="button"
            class="icon-btn"
            onclick={toggleMute}
            aria-label={muted ? "Ativar som" : "Mutar"}
          >
            {#if muted || volume === 0}
              <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
                <line x1="23" y1="9" x2="17" y2="15" />
                <line x1="17" y1="9" x2="23" y2="15" />
              </svg>
            {:else}
              <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
                <path d="M19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.07" />
              </svg>
            {/if}
          </button>
          {#if volumePopoverOpen}
            <div class="volume-pop" role="presentation">
              <input
                type="range"
                min="0"
                max="1"
                step="0.05"
                value={muted ? 0 : volume}
                oninput={(e) => setVolume(Number((e.target as HTMLInputElement).value))}
                aria-label="Volume"
              />
            </div>
          {/if}
        </div>
        <span class="time">
          {fmtTime(currentTime)} <span class="sep">/</span> {fmtTime(duration || (durationMs ?? 0) / 1000)}
        </span>
      </div>

      <div class="right-group">
        {#if subtitles.length > 0}
          <button
            type="button"
            class="icon-btn"
            class:active={toolbarPickerOpen === "subs"}
            onclick={(e) => { e.stopPropagation(); pickerToggle("subs"); }}
            aria-label="Legendas"
          >
            <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <rect x="3" y="5" width="18" height="14" rx="2" />
              <path d="M7 12h3M14 12h3M7 16h6" />
            </svg>
          </button>
        {/if}
        {#if audioTracks.length > 1}
          <button
            type="button"
            class="icon-btn"
            class:active={toolbarPickerOpen === "audio"}
            onclick={(e) => { e.stopPropagation(); pickerToggle("audio"); }}
            aria-label="Áudio"
          >
            <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M11 5L6 9H2v6h4l5 4z" />
            </svg>
          </button>
        {/if}
        {#if chapters.length > 0}
          <button
            type="button"
            class="icon-btn"
            class:active={chaptersOpen}
            onclick={(e) => { e.stopPropagation(); chaptersOpen = !chaptersOpen; }}
            aria-label={$t("study.player.shell.chapters")}
          >
            <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <line x1="8" y1="6" x2="21" y2="6" />
              <line x1="8" y1="12" x2="21" y2="12" />
              <line x1="8" y1="18" x2="21" y2="18" />
              <line x1="3" y1="6" x2="3.01" y2="6" />
              <line x1="3" y1="12" x2="3.01" y2="12" />
              <line x1="3" y1="18" x2="3.01" y2="18" />
            </svg>
          </button>
        {/if}
        <button
          type="button"
          class="icon-btn speed-trigger"
          class:active={toolbarPickerOpen === "speed"}
          onclick={(e) => { e.stopPropagation(); pickerToggle("speed"); }}
          aria-label="Velocidade ({initialPlaybackSpeed}×)"
        >
          <span class="speed-label">{fmtSpeed(initialPlaybackSpeed)}</span>
        </button>
        <button
          type="button"
          class="icon-btn"
          class:active={theaterMode}
          onclick={onTheaterToggle}
          aria-label="Modo cinema"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true">
            <rect x="3" y="6" width="18" height="12" rx="1" />
          </svg>
        </button>
        <button type="button" class="icon-btn" onclick={toggleFullscreen} aria-label="Tela cheia">
          {#if isFullscreen}
            <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M8 3v3a2 2 0 0 1-2 2H3M21 8h-3a2 2 0 0 1-2-2V3M3 16h3a2 2 0 0 1 2 2v3M16 21v-3a2 2 0 0 1 2-2h3" />
            </svg>
          {:else}
            <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M8 3H5a2 2 0 0 0-2 2v3m18 0V5a2 2 0 0 0-2-2h-3m0 18h3a2 2 0 0 0 2-2v-3M3 16v3a2 2 0 0 0 2 2h3" />
            </svg>
          {/if}
        </button>
      </div>
    </div>
  </footer>

  {#if toolbarPickerOpen === "subs"}
    <div class="picker-pop subs-pop" role="listbox" aria-label="Legendas">
      <button
        type="button"
        role="option"
        class="pop-opt"
        class:selected={selectedSubtitleLang == null}
        aria-selected={selectedSubtitleLang == null}
        onclick={() => pickSubtitle(null)}
      >
        <span class="pop-check">{selectedSubtitleLang == null ? "✓" : ""}</span>
        <span>Off</span>
      </button>
      <div class="pop-divider"></div>
      {#each subtitles as t (t.lang + t.path)}
        {@const sel = t.lang === selectedSubtitleLang}
        <button
          type="button"
          role="option"
          class="pop-opt"
          class:selected={sel}
          aria-selected={sel}
          onclick={() => pickSubtitle(t.lang)}
        >
          <span class="pop-check">{sel ? "✓" : ""}</span>
          <span class="pop-label">{t.label}</span>
          <span class="pop-fmt">{t.format.toUpperCase()}</span>
        </button>
      {/each}
    </div>
  {/if}
  {#if toolbarPickerOpen === "audio"}
    <div class="picker-pop audio-pop" role="listbox" aria-label="Áudio">
      {#each audioTracks as t (t.lang + t.path)}
        {@const sel = t.lang === selectedAudioLang}
        <button
          type="button"
          role="option"
          class="pop-opt"
          class:selected={sel}
          aria-selected={sel}
          onclick={() => pickAudio(t.lang)}
        >
          <span class="pop-check">{sel ? "✓" : ""}</span>
          <span class="pop-label">{t.label}</span>
          <span class="pop-fmt">{t.format.toUpperCase()}</span>
        </button>
      {/each}
    </div>
  {/if}
  {#if toolbarPickerOpen === "speed"}
    <div class="picker-pop speed-pop" role="listbox" aria-label="Velocidade">
      {#each SPEED_OPTIONS as o (o)}
        {@const sel = Math.abs(o - initialPlaybackSpeed) < 0.001}
        <button
          type="button"
          role="option"
          class="pop-opt"
          class:selected={sel}
          aria-selected={sel}
          onclick={() => pickSpeed(o)}
        >
          <span class="pop-check">{sel ? "✓" : ""}</span>
          <span class="pop-label">{fmtSpeed(o)}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .shell {
    position: relative;
    width: 100%;
    aspect-ratio: 16 / 9;
    background: #000;
    border-radius: 12px;
    overflow: hidden;
    color: white;
    user-select: none;
  }

  .shell.fullscreen {
    border-radius: 0;
    aspect-ratio: auto;
    height: 100vh;
  }

  video {
    position: absolute;
    inset: 0;
    width: 100% !important;
    height: 100% !important;
    min-width: 100%;
    min-height: 100%;
    max-width: none;
    max-height: none;
    object-fit: contain;
    background: black;
    cursor: pointer;
    display: block;
  }

  video::cue {
    background: color-mix(in oklab, black 70%, transparent);
    color: white;
    font-size: 1.1em;
  }

  .shell.loading > video {
    visibility: hidden;
  }

  .spinner {
    position: absolute;
    top: 50%;
    left: 50%;
    width: 48px;
    height: 48px;
    margin: -24px 0 0 -24px;
    z-index: 5;
    pointer-events: none;
  }

  .spinner .ring {
    width: 100%;
    height: 100%;
    border-radius: 50%;
    border: 4px solid rgba(255, 255, 255, 0.18);
    border-top-color: #fff;
    animation: shell-spin 1s linear infinite;
  }

  @keyframes shell-spin {
    to { transform: rotate(360deg); }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner .ring { animation-duration: 3s; }
  }


  .big-play {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 80px;
    height: 80px;
    border-radius: 50%;
    background: color-mix(in oklab, black 50%, transparent);
    backdrop-filter: blur(12px);
    border: 1px solid color-mix(in oklab, white 20%, transparent);
    color: white;
    cursor: pointer;
    display: grid;
    place-items: center;
    z-index: 6;
    transition: transform 200ms ease, background 200ms ease;
  }

  .big-play:hover {
    transform: translate(-50%, -50%) scale(1.06);
    background: var(--accent);
  }

  .big-play svg {
    margin-left: 4px;
  }

  .top-bar,
  .bottom-bar {
    position: absolute;
    left: 0;
    right: 0;
    padding: 16px 24px;
    z-index: 4;
    transition: opacity 280ms ease, transform 280ms ease;
  }

  .top-bar {
    top: 0;
    background: linear-gradient(to bottom, color-mix(in oklab, black 70%, transparent), transparent);
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }

  .bottom-bar {
    bottom: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 12px 24px 16px;
  }

  .shell:not(.controls-visible) .top-bar,
  .shell:not(.controls-visible) .bottom-bar {
    opacity: 0;
    transform: translateY(8px);
    pointer-events: none;
  }

  .scrim-bottom {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 38%;
    min-height: 96px;
    z-index: 3;
    pointer-events: none;
    background: linear-gradient(
      to top,
      color-mix(in oklab, black 60%, transparent) 0%,
      color-mix(in oklab, black 28%, transparent) 45%,
      transparent 100%
    );
    opacity: 0.55;
    transition: opacity 280ms ease;
  }

  .shell.controls-visible .scrim-bottom {
    opacity: 1;
  }

  .shell.paused .scrim-bottom {
    opacity: 1;
  }

  .shell:not(.controls-visible):not(.paused) {
    cursor: none;
  }

  .shell.paused .top-bar,
  .shell.paused .bottom-bar {
    opacity: 1;
    transform: translateY(0);
    pointer-events: auto;
  }

  .back,
  .icon-btn {
    position: relative;
    width: 36px;
    height: 36px;
    border-radius: 8px;
    border: none;
    background: transparent;
    color: white;
    display: grid;
    place-items: center;
    cursor: pointer;
    transition: background 150ms ease, color 150ms ease;
    text-decoration: none;
    flex: 0 0 auto;
  }

  .icon-btn[aria-label]:hover::after,
  .back[aria-label]:hover::after {
    content: attr(aria-label);
    position: absolute;
    bottom: calc(100% + 8px);
    left: 50%;
    transform: translateX(-50%);
    padding: 4px 8px;
    background: color-mix(in oklab, black 92%, transparent);
    color: white;
    font-size: 11px;
    font-weight: 500;
    white-space: nowrap;
    border-radius: 4px;
    pointer-events: none;
    z-index: 10;
    border: 1px solid color-mix(in oklab, white 14%, transparent);
    animation: tip-in 120ms ease-out 400ms both;
  }

  @keyframes tip-in {
    from {
      opacity: 0;
      transform: translate(-50%, 4px);
    }
    to {
      opacity: 1;
      transform: translate(-50%, 0);
    }
  }

  .back:hover,
  .icon-btn:hover,
  .icon-btn.active {
    background: color-mix(in oklab, white 14%, transparent);
  }

  .icon-btn.primary:hover {
    background: var(--accent);
  }

  .title-block {
    flex: 1 1 auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding-top: 4px;
  }

  .course-title {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: color-mix(in oklab, white 60%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .lesson-title {
    font-size: 15px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .progress-wrap {
    position: relative;
    padding: 12px 0;
    cursor: pointer;
  }

  .progress-track {
    position: relative;
    height: 4px;
    background: color-mix(in oklab, white 24%, transparent);
    border-radius: 2px;
    transition: height 150ms ease;
  }

  .progress-wrap:hover .progress-track {
    height: 6px;
  }

  .progress-buffered,
  .progress-fill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 2px;
  }

  .progress-buffered {
    background: color-mix(in oklab, white 24%, transparent);
    pointer-events: none;
  }

  .progress-fill {
    background: var(--accent);
    pointer-events: none;
  }

  .progress-thumb {
    position: absolute;
    top: 50%;
    width: 14px;
    height: 14px;
    background: white;
    border-radius: 50%;
    transform: translate(-50%, -50%) scale(0);
    transition: transform 150ms ease;
    pointer-events: none;
    box-shadow: 0 1px 4px color-mix(in oklab, black 40%, transparent);
  }

  .progress-wrap:hover .progress-thumb {
    transform: translate(-50%, -50%) scale(1);
  }

  .gap-marker {
    position: absolute;
    top: 0;
    bottom: 0;
    border-radius: 2px;
    pointer-events: none;
  }

  .gap-marker.intro {
    background: color-mix(in oklab, gold 60%, transparent);
    opacity: 0.4;
  }

  .gap-marker.outro {
    background: color-mix(in oklab, orchid 60%, transparent);
    opacity: 0.4;
  }

  .controls {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .left-group,
  .right-group {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .volume-wrap {
    position: relative;
  }

  .volume-pop {
    position: absolute;
    bottom: calc(100% + 4px);
    left: 50%;
    transform: translateX(-50%);
    background: color-mix(in oklab, black 80%, transparent);
    backdrop-filter: blur(8px);
    border: 1px solid color-mix(in oklab, white 16%, transparent);
    border-radius: 8px;
    padding: 12px 8px;
  }

  .volume-pop input[type="range"] {
    writing-mode: vertical-lr;
    direction: rtl;
    width: 4px;
    height: 80px;
    accent-color: var(--accent);
  }

  .time {
    margin-left: 6px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: color-mix(in oklab, white 80%, transparent);
  }

  .time .sep {
    margin: 0 4px;
    color: color-mix(in oklab, white 40%, transparent);
  }

  .speed-trigger {
    width: auto;
    padding: 0 12px;
  }

  .speed-label {
    font-size: 13px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .picker-pop {
    position: absolute;
    bottom: 80px;
    right: 24px;
    z-index: 8;
    min-width: 180px;
    background: color-mix(in oklab, black 80%, transparent);
    backdrop-filter: blur(12px);
    border: 1px solid color-mix(in oklab, white 16%, transparent);
    border-radius: 10px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    box-shadow: 0 8px 24px color-mix(in oklab, black 50%, transparent);
    animation: pop-in 160ms ease-out;
  }

  .speed-pop {
    right: 80px;
    min-width: 100px;
  }

  .audio-pop {
    right: 130px;
  }

  .subs-pop {
    right: 170px;
  }

  .pop-opt {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: white;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
    width: 100%;
    transition: background 120ms ease;
  }

  .pop-opt:hover {
    background: color-mix(in oklab, white 12%, transparent);
  }

  .pop-opt.selected {
    color: var(--accent);
    font-weight: 600;
  }

  .pop-check {
    width: 14px;
    flex: 0 0 auto;
    color: var(--accent);
    text-align: center;
  }

  .pop-label {
    flex: 1 1 auto;
    font-variant-numeric: tabular-nums;
  }

  .pop-fmt {
    margin-left: auto;
    font-size: 10px;
    color: color-mix(in oklab, white 50%, transparent);
    font-weight: 500;
    letter-spacing: 0.04em;
  }

  .pop-divider {
    height: 1px;
    background: color-mix(in oklab, white 14%, transparent);
    margin: 4px 0;
  }

  @keyframes pop-in {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .top-bar,
    .bottom-bar,
    .progress-track,
    .progress-thumb,
    .big-play {
      transition: none;
    }
  }

  .chapters-drawer {
    position: absolute;
    top: 64px;
    right: 16px;
    bottom: 96px;
    width: 320px;
    max-width: 38vw;
    background: color-mix(in oklab, black 80%, transparent);
    border: 1px solid color-mix(in oklab, white 12%, transparent);
    border-radius: 12px;
    backdrop-filter: blur(10px);
    color: white;
    z-index: 6;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: 0 12px 32px color-mix(in oklab, black 50%, transparent);
    animation: drawer-in 200ms ease-out;
  }

  .chapters-drawer__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    border-bottom: 1px solid color-mix(in oklab, white 10%, transparent);
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.02em;
    text-transform: uppercase;
  }

  .chapters-drawer__body {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: 6px;
  }

  @keyframes drawer-in {
    from {
      opacity: 0;
      transform: translateX(8px);
    }
    to {
      opacity: 1;
      transform: translateX(0);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .chapters-drawer {
      animation: none;
    }
  }
</style>
