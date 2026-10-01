<script lang="ts">
  import { onMount, onDestroy, tick } from "svelte";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgIcon from "./TgIcon.svelte";
  import TranscriptPanel from "./TranscriptPanel.svelte";
  import {
    tg2,
    isAudio,
    messageLink,
    formatBytes,
    formatDate,
    formatDuration,
    type ExternalPlayer,
    type StreamInfo,
    type TelegramChat,
    type TelegramMediaItem,
  } from "./api";
  import { failText, notePlayback, resumeOf, thumbnail } from "./session.svelte";

  let {
    chat,
    queue,
    start = 0,
    startAt = 0,
    transcript = false,
    onclose,
    onDownload,
  }: {
    chat: TelegramChat;
    queue: TelegramMediaItem[];
    start?: number;
    startAt?: number;
    transcript?: boolean;
    onclose: () => void;
    onDownload?: (item: TelegramMediaItem) => void;
  } = $props();

  const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.5, 3];
  let players = $state<ExternalPlayer[]>(["default", "vlc", "iina", "mpv"]);
  const AUTOPLAY_KEY = "telegram.player.autoplay";
  const VOLUME_KEY = "telegram.player.volume";
  const RATE_KEY = "telegram.player.rate";
  const SAVE_EVERY = 5;

  let index = $state(0);
  let info = $state<StreamInfo | null>(null);
  let loading = $state(true);
  let failed = $state<string | null>(null);
  let video: HTMLVideoElement | undefined = $state();
  let stage: HTMLDivElement | undefined = $state();
  let playing = $state(false);
  let time = $state(0);
  let duration = $state(0);
  let buffered = $state(0);
  let volume = $state(readNum(VOLUME_KEY, 1));
  let muted = $state(false);
  let rate = $state(readNum(RATE_KEY, 1));
  let autoplay = $state(readBool(AUTOPLAY_KEY, true));
  type Side = "queue" | "transcript" | null;
  let side = $state<Side>(null);
  let src = $state<string | null>(null);
  let convertOffer = $state(false);
  let converting = $state(false);
  let firstLoad = true;
  let showExternal = $state(false);
  let showSpeed = $state(false);
  let idle = $state(false);
  let resumeAt = 0;
  let resumedFrom = $state(0);
  let lastSaved = 0;
  let loadSeq = 0;
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  let artUrl = $state<string | null>(null);

  let item = $derived<TelegramMediaItem | undefined>(queue[index]);
  let audioOnly = $derived(item ? isAudio(item) : false);
  let hasNext = $derived(index < queue.length - 1);
  let hasPrev = $derived(index > 0);

  function readNum(key: string, def: number): number {
    try {
      const v = Number(localStorage.getItem(key));
      return Number.isFinite(v) && v > 0 ? v : def;
    } catch {
      return def;
    }
  }

  function readBool(key: string, def: boolean): boolean {
    try {
      const v = localStorage.getItem(key);
      return v === null ? def : v === "1";
    } catch {
      return def;
    }
  }

  function store(key: string, value: string) {
    try {
      localStorage.setItem(key, value);
    } catch {
    }
  }

  async function load(i: number) {
    const seq = ++loadSeq;
    const it = queue[i];
    if (!it) return;
    info = null;
    failed = null;
    loading = true;
    time = 0;
    duration = 0;
    buffered = 0;
    resumeAt = 0;
    resumedFrom = 0;
    lastSaved = 0;
    artUrl = null;
    showExternal = false;
    src = null;
    convertOffer = false;
    converting = false;
    fullConvert = false;
    void thumbnail(chat, it.message_id).then((u) => {
      if (seq === loadSeq) artUrl = u;
    });
    try {
      const [s, pb] = await Promise.all([
        tg2.streamUrl(chat, it.message_id),
        tg2.playbackGet(chat, it.message_id).catch(() => null),
      ]);
      if (seq !== loadSeq) return;
      const d = pb?.duration_sec || s.duration_sec || 0;
      if (pb && !pb.finished && pb.position_sec > 5 && (!d || pb.position_sec < d - 8)) resumeAt = pb.position_sec;
      if (firstLoad && startAt > 0) resumeAt = startAt;
      firstLoad = false;
      info = s;
      if (s.playable === false) {
        loading = false;
        if (s.remux_url || s.transcode_url) convertOffer = true;
        else failed = $t("telegram.player.unsupported") as string;
      } else {
        src = s.url;
      }
    } catch (e) {
      if (seq !== loadSeq) return;
      failed = failText(e);
      loading = false;
    }
  }

  function go(i: number) {
    if (i < 0 || i >= queue.length || i === index) return;
    void save(true);
    index = i;
    void load(i);
  }

  async function save(force = false) {
    const it = item;
    if (!it || !info || !video) return;
    const pos = video.currentTime || 0;
    const dur = video.duration && Number.isFinite(video.duration) ? video.duration : duration;
    if (!force && Math.abs(pos - lastSaved) < SAVE_EVERY) return;
    if (pos < 1 && !force) return;
    lastSaved = pos;
    const finished = dur > 0 && pos >= dur - 3;
    notePlayback(chat, {
      message_id: it.message_id,
      position_sec: pos,
      duration_sec: dur,
      finished,
      file_name: it.file_name,
      media_type: it.media_type,
      updated_at: Math.floor(Date.now() / 1000),
    });
    try {
      await tg2.playbackSet(chat, it.message_id, pos, dur, {
        chatTitle: chat.title,
        fileName: it.file_name,
        mediaType: it.media_type,
      });
    } catch {
    }
  }

  function onMeta() {
    if (!video) return;
    duration = Number.isFinite(video.duration) ? video.duration : 0;
    video.playbackRate = rate;
    video.volume = volume;
    if (resumeAt > 0 && resumeAt < duration - 2) {
      video.currentTime = resumeAt;
      resumedFrom = resumeAt;
      setTimeout(() => (resumedFrom = 0), 4000);
    }
    loading = false;
    void video.play().catch(() => {
    });
  }

  function onTime() {
    if (!video) return;
    time = video.currentTime;
    if (video.buffered.length) buffered = video.buffered.end(video.buffered.length - 1);
    void save();
  }

  function onEnded() {
    void save(true);
    if (autoplay && hasNext) go(index + 1);
  }

  let fullConvert = $state(false);

  function playConverted() {
    const url = info?.remux_url || info?.transcode_url;
    if (!url) return;
    convertOffer = false;
    failed = null;
    converting = true;
    fullConvert = url === info?.transcode_url;
    loading = true;
    src = url;
  }

  function onError() {
    const code = video?.error?.code;
    if (code === 4 && (info?.remux_url || info?.transcode_url) && src === info?.url) {
      loading = false;
      src = null;
      convertOffer = true;
      return;
    }
    if (info?.transcode_url && src === info.remux_url) {
      converting = true;
      fullConvert = true;
      loading = true;
      src = info.transcode_url;
      return;
    }
    failed =
      code === 4
        ? ($t("telegram.player.unsupported") as string)
        : ($t("telegram.player.stream_failed") as string);
    loading = false;
  }

  function toggle() {
    if (!video || failed) return;
    if (video.paused) void video.play().catch(() => {});
    else {
      video.pause();
      void save(true);
    }
  }

  function seek(to: number) {
    if (!video || !duration) return;
    video.currentTime = Math.max(0, Math.min(duration - 0.1, to));
    time = video.currentTime;
  }

  function setVolume(v: number) {
    volume = Math.max(0, Math.min(1, v));
    if (video) video.volume = volume;
    if (volume > 0) muted = false;
    store(VOLUME_KEY, String(volume || 1));
  }

  function setRate(r: number) {
    rate = r;
    if (video) video.playbackRate = r;
    store(RATE_KEY, String(r));
    showSpeed = false;
  }

  function toggleAutoplay() {
    autoplay = !autoplay;
    store(AUTOPLAY_KEY, autoplay ? "1" : "0");
  }

  async function fullscreen() {
    const el = stage as (HTMLElement & { webkitRequestFullscreen?: () => void }) | undefined;
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else if (el?.requestFullscreen) await el.requestFullscreen();
      else if (el?.webkitRequestFullscreen) el.webkitRequestFullscreen();
      else (video as HTMLVideoElement & { webkitEnterFullscreen?: () => void })?.webkitEnterFullscreen?.();
    } catch {
      (video as HTMLVideoElement & { webkitEnterFullscreen?: () => void })?.webkitEnterFullscreen?.();
    }
  }

  async function pip() {
    const v = video as
      | (HTMLVideoElement & {
          webkitSupportsPresentationMode?: (m: string) => boolean;
          webkitSetPresentationMode?: (m: string) => void;
          webkitPresentationMode?: string;
        })
      | undefined;
    if (!v) return;
    try {
      if (document.pictureInPictureElement) await document.exitPictureInPicture();
      else if (document.pictureInPictureEnabled) await v.requestPictureInPicture();
      else if (v.webkitSupportsPresentationMode?.("picture-in-picture"))
        v.webkitSetPresentationMode?.(v.webkitPresentationMode === "picture-in-picture" ? "inline" : "picture-in-picture");
      else showToast("info", $t("telegram.player.pip_unavailable") as string);
    } catch {
      showToast("info", $t("telegram.player.pip_unavailable") as string);
    }
  }

  async function openExternal(p: ExternalPlayer) {
    showExternal = false;
    if (!info) return;
    video?.pause();
    try {
      await tg2.openExternal(info.url, p);
    } catch (e) {
      showToast("error", failText(e));
    }
  }

  async function copy(text: string | null) {
    if (!text) return;
    try {
      await writeText(text);
      showToast("success", $t("telegram.common.copied") as string);
    } catch {
    }
  }

  function close() {
    void save(true);
    onclose();
  }

  function poke() {
    idle = false;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
      if (playing && !showExternal && !showSpeed) idle = true;
    }, 2600);
  }

  function onKey(e: KeyboardEvent) {
    const tag = (e.target as HTMLElement | null)?.tagName;
    if (tag === "INPUT" && (e.target as HTMLInputElement).type !== "range") return;
    if (tag === "TEXTAREA" || tag === "SELECT") return;
    poke();
    const k = e.key;
    let handled = true;
    if (k === " " || k === "k") toggle();
    else if (k === "ArrowLeft") seek(time - (e.shiftKey ? 30 : 5));
    else if (k === "ArrowRight") seek(time + (e.shiftKey ? 30 : 5));
    else if (k === "ArrowUp") setVolume(volume + 0.1);
    else if (k === "ArrowDown") setVolume(volume - 0.1);
    else if (k === "f" || k === "F") void fullscreen();
    else if (k === "m" || k === "M") muted = !muted;
    else if (k === "t" || k === "T") side = side === "transcript" ? null : "transcript";
    else if (k === "n" || k === "N" || k === "PageDown") go(index + 1);
    else if (k === "p" || k === "P" || k === "PageUp") go(index - 1);
    else if (k === ">") setRate(SPEEDS[Math.min(SPEEDS.length - 1, SPEEDS.indexOf(rate) + 1)] ?? rate);
    else if (k === "<") setRate(SPEEDS[Math.max(0, SPEEDS.indexOf(rate) - 1)] ?? rate);
    else if (k === "Escape") {
      if (showExternal || showSpeed) {
        showExternal = false;
        showSpeed = false;
      } else if (!document.fullscreenElement) close();
      else handled = false;
    } else handled = false;
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  }

  function tileThumb(node: HTMLElement, it: TelegramMediaItem) {
    if (isAudio(it)) return;
    void thumbnail(chat, it.message_id).then((url) => {
      if (url) node.style.backgroundImage = `url("${url}")`;
    });
  }

  function seekTo(sec: number) {
    seek(sec);
    if (video?.paused) void video.play().catch(() => {});
  }

  onMount(() => {
    side = transcript ? "transcript" : queue.length > 1 ? "queue" : null;
    index = Math.max(0, Math.min(queue.length - 1, start));
    void load(index);
    tg2
      .externalPlayers()
      .then((list) => {
        if (Array.isArray(list) && list.length) players = list.includes("default") ? list : ["default", ...list];
      })
      .catch(() => {});
    window.addEventListener("keydown", onKey, true);
    poke();
    void tick().then(() => stage?.focus());
  });

  onDestroy(() => {
    window.removeEventListener("keydown", onKey, true);
    clearTimeout(idleTimer);
    void save(true);
  });
</script>

<div class="pl" role="dialog" aria-modal="true" aria-label={item?.file_name ?? "Player"}>
  <div class="pl-shell" class:with-queue={side === "transcript" || (side === "queue" && queue.length > 1)}>
    <header class="pl-top">
      <div class="pl-titles">
        <span class="pl-name" title={item?.file_name}>{item?.caption || item?.file_name}</span>
        <span class="pl-sub">
          {chat.title}
          {#if item} · {formatBytes(item.file_size)} · {formatDate(item.date)}{/if}
          {#if queue.length > 1} · {$t("telegram.player.position", { count: index + 1, total: queue.length })}{/if}
        </span>
      </div>
      <div class="pl-actions">
        {#if item && onDownload}
          <button class="pl-btn" onclick={() => item && onDownload?.(item)} title={$t("telegram.player.download_this")}>
            <TgIcon name="download" size={16} /><span>{$t("telegram.player.download_this")}</span>
          </button>
        {/if}
        <div class="pl-menu-wrap">
          <button class="pl-btn" onclick={() => (showExternal = !showExternal)} disabled={!info} aria-expanded={showExternal}>
            <TgIcon name="external" size={16} /><span>{$t("telegram.player.open_in")}</span>
          </button>
          {#if showExternal}
            <div class="pl-menu" role="menu">
              {#each players as p (p)}
                <button class="pl-menu-item" role="menuitem" onclick={() => openExternal(p)}>{$t(`telegram.player.players.${p}`)}</button>
              {/each}
              <div class="pl-menu-sep"></div>
              <button class="pl-menu-item" role="menuitem" onclick={() => { showExternal = false; void copy(info?.url ?? null); }}>{$t("telegram.player.copy_stream")}</button>
              {#if item && messageLink(chat, item.message_id)}
                <button class="pl-menu-item" role="menuitem" onclick={() => { showExternal = false; void copy(item ? messageLink(chat, item.message_id) : null); }}>{$t("telegram.player.copy_link")}</button>
              {/if}
            </div>
          {/if}
        </div>
        <button class="pl-btn" class:on={side === "transcript"} onclick={() => (side = side === "transcript" ? null : "transcript")} aria-pressed={side === "transcript"}>
          <TgIcon name="captions" size={16} /><span>{$t("telegram.transcript.button")}</span>
        </button>
        {#if queue.length > 1}
          <button class="pl-btn icon" class:on={side === "queue"} onclick={() => (side = side === "queue" ? null : "queue")} title={$t("telegram.player.queue")} aria-label={$t("telegram.player.queue")} aria-pressed={side === "queue"}>
            <TgIcon name="list" size={16} />
          </button>
        {/if}
        <button class="pl-btn icon" onclick={close} title={$t("telegram.common.close")} aria-label={$t("telegram.common.close")}>
          <TgIcon name="x" size={18} />
        </button>
      </div>
    </header>

    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div
      class="pl-stage"
      class:idle
      class:audio={audioOnly}
      bind:this={stage}
      tabindex="0"
      role="application"
      aria-label={$t("telegram.player.stage")}
      onpointermove={poke}
    >
      {#if info && src}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video
          bind:this={video}
          {src}
          preload="auto"
          playsinline
          bind:muted
          onloadedmetadata={onMeta}
          ontimeupdate={onTime}
          onplay={() => (playing = true)}
          onpause={() => { playing = false; idle = false; }}
          onended={onEnded}
          onerror={onError}
          onwaiting={() => (loading = true)}
          onplaying={() => { loading = false; converting = false; }}
          onclick={toggle}
          ondblclick={() => void fullscreen()}
        ></video>
      {/if}

      {#if audioOnly && !failed}
        <div class="pl-art" style:background-image={artUrl ? `url("${artUrl}")` : undefined}>
          {#if !artUrl}<TgIcon name="music" size={56} />{/if}
        </div>
      {/if}

      {#if loading && !failed}
        <div class="pl-center">
          <div class="pl-wait">
            <span class="spinner pl-spin"></span>
            {#if converting}<p class="pl-conv-note">{fullConvert ? $t("telegram.player.converting_full") : $t("telegram.player.converting")}</p>{/if}
          </div>
        </div>
      {/if}

      {#if convertOffer && info}
        <div class="pl-center">
          <div class="pl-fail">
            <TgIcon name="convert" size={28} />
            <p class="pl-fail-title">{$t("telegram.player.needs_convert")}</p>
            <p class="pl-fail-hint">{$t("telegram.player.needs_convert_hint", { name: info.file_name })}</p>
            <div class="pl-fail-actions">
              <button class="pl-btn solid primary" onclick={playConverted}><TgIcon name="play" size={14} /> {$t("telegram.player.convert_play")}</button>
            </div>
            <p class="pl-fail-hint">{$t("telegram.player.try_external")}</p>
            <div class="pl-fail-actions">
              {#each players as p (p)}
                <button class="pl-btn solid" onclick={() => openExternal(p)}>{$t(`telegram.player.players.${p}`)}</button>
              {/each}
            </div>
          </div>
        </div>
      {/if}

      {#if resumedFrom}
        <div class="pl-pill">
          {$t("telegram.player.resumed", { time: formatDuration(resumedFrom) })}
          <button class="pl-link" onclick={() => { seek(0); resumedFrom = 0; }}>{$t("telegram.player.from_start")}</button>
        </div>
      {/if}

      {#if failed}
        <div class="pl-center">
          <div class="pl-fail">
            <TgIcon name="warn" size={28} />
            <p class="pl-fail-title">{failed}</p>
            {#if info}
              <p class="pl-fail-hint">{$t("telegram.player.try_external")}</p>
              <div class="pl-fail-actions">
                {#each players as p (p)}
                  <button class="pl-btn solid" onclick={() => openExternal(p)}>{$t(`telegram.player.players.${p}`)}</button>
                {/each}
              </div>
            {:else}
              <button class="pl-btn solid" onclick={() => void load(index)}>{$t("telegram.common.retry")}</button>
            {/if}
          </div>
        </div>
      {/if}

      <div class="pl-controls" class:hidden={!!failed || convertOffer}>
        <div class="pl-seek">
          <div class="pl-seek-buf" style:width="{duration ? (buffered / duration) * 100 : 0}%"></div>
          <div class="pl-seek-fill" style:width="{duration ? (time / duration) * 100 : 0}%"></div>
          <input
            type="range"
            min="0"
            max={duration || 0}
            step="0.1"
            value={time}
            oninput={(e) => seek(Number((e.currentTarget as HTMLInputElement).value))}
            aria-label={$t("telegram.player.seek")}
          />
        </div>
        <div class="pl-row">
          <button class="pl-ctl" onclick={() => go(index - 1)} disabled={!hasPrev} title={$t("telegram.player.prev")} aria-label={$t("telegram.player.prev")}><TgIcon name="prev" size={18} /></button>
          <button class="pl-ctl big" onclick={toggle} title={$t("telegram.player.play_pause")} aria-label={$t("telegram.player.play_pause")}>
            <TgIcon name={playing ? "pause" : "play"} size={22} />
          </button>
          <button class="pl-ctl" onclick={() => go(index + 1)} disabled={!hasNext} title={$t("telegram.player.next")} aria-label={$t("telegram.player.next")}><TgIcon name="next" size={18} /></button>
          <span class="pl-time">{formatDuration(time)} / {formatDuration(duration)}</span>

          <span class="pl-grow"></span>

          <button class="pl-ctl" onclick={() => (muted = !muted)} title={$t("telegram.player.mute")} aria-label={$t("telegram.player.mute")}>
            <TgIcon name={muted || volume === 0 ? "mute" : "volume"} size={18} />
          </button>
          <input
            class="pl-vol"
            type="range"
            min="0"
            max="1"
            step="0.05"
            value={muted ? 0 : volume}
            oninput={(e) => setVolume(Number((e.currentTarget as HTMLInputElement).value))}
            aria-label={$t("telegram.player.volume")}
          />
          <div class="pl-menu-wrap">
            <button class="pl-ctl text" onclick={() => (showSpeed = !showSpeed)} title={$t("telegram.player.speed")} aria-expanded={showSpeed}>{rate}×</button>
            {#if showSpeed}
              <div class="pl-menu up" role="menu">
                {#each SPEEDS as s (s)}
                  <button class="pl-menu-item" class:cur={s === rate} role="menuitemradio" aria-checked={s === rate} onclick={() => setRate(s)}>{s}×</button>
                {/each}
              </div>
            {/if}
          </div>
          <button class="pl-ctl text" class:on={autoplay} onclick={toggleAutoplay} title={$t("telegram.player.autoplay_hint")} aria-pressed={autoplay}>
            {$t("telegram.player.autoplay")}
          </button>
          {#if !audioOnly}
            <button class="pl-ctl" onclick={pip} title={$t("telegram.player.pip")} aria-label={$t("telegram.player.pip")}><TgIcon name="pip" size={18} /></button>
          {/if}
          <button class="pl-ctl" onclick={() => void fullscreen()} title={$t("telegram.player.fullscreen")} aria-label={$t("telegram.player.fullscreen")}><TgIcon name="expand" size={18} /></button>
        </div>
      </div>
    </div>

    {#if side === "transcript" && item}
      <aside class="pl-queue pl-tr" aria-label={$t("telegram.transcript.title")}>
        <div class="pl-q-head">{$t("telegram.transcript.title")}</div>
        <div class="pl-tr-body">
          <TranscriptPanel {chat} messageId={item.message_id} current={time} dark onseek={seekTo} />
        </div>
      </aside>
    {:else if side === "queue" && queue.length > 1}
      <aside class="pl-queue" aria-label={$t("telegram.player.queue")}>
        <div class="pl-q-head">{$t("telegram.player.queue")} <span>{queue.length}</span></div>
        <div class="pl-q-list">
          {#each queue as q, i (q.message_id)}
            {@const r = resumeOf(chat, q.message_id)}
            <button class="pl-q-item" class:cur={i === index} onclick={() => go(i)}>
              <span class="pl-q-thumb" use:tileThumb={q}>
                <TgIcon name={isAudio(q) ? "music" : i === index && playing ? "pause" : "play"} size={14} />
                {#if r && r.duration_sec > 0}
                  <span class="pl-q-prog" class:done={r.finished} style:width="{r.finished ? 100 : (r.position_sec / r.duration_sec) * 100}%"></span>
                {/if}
              </span>
              <span class="pl-q-body">
                <span class="pl-q-name">{q.caption || q.file_name}</span>
                <span class="pl-q-meta">{formatBytes(q.file_size)} · {formatDate(q.date)}</span>
              </span>
            </button>
          {/each}
        </div>
      </aside>
    {/if}
  </div>
  <p class="pl-keys">{$t("telegram.player.keys")}</p>
</div>

<style>
  .pl {
    position: fixed;
    inset: 0;
    z-index: 450;
    display: flex;
    flex-direction: column;
    padding: var(--space-4);
    gap: var(--space-2);
    background: rgba(8, 9, 12, 0.9);
    backdrop-filter: blur(18px) saturate(1.2);
    -webkit-backdrop-filter: blur(18px) saturate(1.2);
    color: #f4f5f7;
    animation: pl-in 180ms var(--ease-out, ease-out);
  }

  @keyframes pl-in {
    from {
      opacity: 0;
      transform: scale(0.985);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .pl {
      animation: none;
    }
  }

  .pl-shell {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: auto minmax(0, 1fr);
    gap: var(--space-3);
  }

  .pl-shell.with-queue {
    grid-template-columns: minmax(0, 1fr) 300px;
  }

  .pl-top {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .pl-titles {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .pl-name {
    font-size: var(--text-lg);
    font-weight: 650;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .pl-sub {
    font-size: var(--text-sm);
    color: rgba(244, 245, 247, 0.6);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .pl-actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .pl-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 12px;
    border: none;
    border-radius: var(--radius-full);
    background: rgba(255, 255, 255, 0.08);
    color: inherit;
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 550;
    cursor: pointer;
    transition: background var(--duration-fast, 120ms) ease;
  }

  .pl-btn:hover:not(:disabled),
  .pl-btn.on {
    background: rgba(255, 255, 255, 0.16);
  }

  .pl-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .pl-btn.icon {
    width: 30px;
    padding: 0;
    justify-content: center;
  }

  .pl-btn.solid {
    background: rgba(255, 255, 255, 0.14);
  }

  .pl-btn:focus-visible,
  .pl-ctl:focus-visible,
  .pl-q-item:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .pl-menu-wrap {
    position: relative;
  }

  .pl-menu {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 3;
    min-width: 200px;
    padding: 4px;
    border-radius: var(--radius-lg);
    background: rgba(30, 32, 38, 0.97);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45), inset 0 0 0 1px rgba(255, 255, 255, 0.08);
    display: flex;
    flex-direction: column;
  }

  .pl-menu.up {
    top: auto;
    bottom: calc(100% + 8px);
    min-width: 90px;
  }

  .pl-menu-item {
    padding: 6px 10px;
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: inherit;
    font: inherit;
    font-size: var(--text-sm);
    text-align: left;
    cursor: pointer;
  }

  .pl-menu-item:hover,
  .pl-menu-item.cur {
    background: rgba(255, 255, 255, 0.1);
  }

  .pl-menu-sep {
    height: 1px;
    margin: 4px 6px;
    background: rgba(255, 255, 255, 0.1);
  }

  .pl-stage {
    position: relative;
    min-height: 0;
    border-radius: var(--radius-xl);
    overflow: hidden;
    background: #000;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.5);
    outline: none;
  }

  .pl-stage:fullscreen {
    border-radius: 0;
  }

  .pl-stage.idle {
    cursor: none;
  }

  video {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
    background: #000;
  }

  .pl-stage.audio video {
    position: absolute;
    inset: 0;
    opacity: 0;
  }

  .pl-art {
    position: absolute;
    inset: 0;
    margin: auto;
    width: min(46%, 340px);
    aspect-ratio: 1;
    border-radius: var(--radius-xl);
    background: linear-gradient(160deg, #3a3f4b, #16181d) center / cover no-repeat;
    display: grid;
    place-items: center;
    color: rgba(255, 255, 255, 0.5);
    box-shadow: 0 20px 50px rgba(0, 0, 0, 0.55);
    pointer-events: none;
  }

  .pl-center {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    pointer-events: none;
  }

  .pl-spin {
    width: 36px;
    height: 36px;
  }

  .pl-fail {
    pointer-events: auto;
    max-width: 440px;
    padding: var(--space-5);
    border-radius: var(--radius-xl);
    background: rgba(24, 26, 31, 0.92);
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
  }

  .pl-fail-title {
    margin: 0;
    font-weight: 600;
  }

  .pl-fail-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: rgba(244, 245, 247, 0.65);
  }

  .pl-fail-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .pl-pill {
    position: absolute;
    top: 14px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 12px;
    border-radius: var(--radius-full);
    background: rgba(20, 22, 27, 0.85);
    font-size: var(--text-sm);
  }

  .pl-link {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent-hi, #7cb8ff);
    font: inherit;
    cursor: pointer;
  }

  .pl-controls {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    padding: 36px 16px 12px;
    background: linear-gradient(to top, rgba(0, 0, 0, 0.78), rgba(0, 0, 0, 0));
    transition: opacity 220ms ease;
  }

  .pl-stage.idle .pl-controls,
  .pl-controls.hidden {
    opacity: 0;
    pointer-events: none;
  }

  .pl-seek {
    position: relative;
    height: 16px;
    display: flex;
    align-items: center;
  }

  .pl-seek::before,
  .pl-seek-buf,
  .pl-seek-fill {
    content: "";
    position: absolute;
    left: 0;
    height: 4px;
    border-radius: 2px;
  }

  .pl-seek::before {
    right: 0;
    background: rgba(255, 255, 255, 0.18);
  }

  .pl-seek-buf {
    background: rgba(255, 255, 255, 0.32);
  }

  .pl-seek-fill {
    background: var(--accent);
  }

  .pl-seek input {
    position: absolute;
    inset: 0;
    width: 100%;
    margin: 0;
    opacity: 0;
    cursor: pointer;
  }

  .pl-seek:hover::before,
  .pl-seek:hover .pl-seek-buf,
  .pl-seek:hover .pl-seek-fill {
    height: 6px;
  }

  .pl-row {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-top: 4px;
  }

  .pl-ctl {
    display: inline-grid;
    place-items: center;
    min-width: 32px;
    height: 32px;
    padding: 0 6px;
    border: none;
    border-radius: var(--radius-full);
    background: none;
    color: inherit;
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
  }

  .pl-ctl:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.12);
  }

  .pl-ctl:disabled {
    opacity: 0.35;
    cursor: default;
  }

  .pl-ctl.big {
    width: 40px;
    height: 40px;
  }

  .pl-ctl.text {
    padding: 0 10px;
    font-variant-numeric: tabular-nums;
  }

  .pl-ctl.text:not(.on):is([aria-pressed]) {
    color: rgba(244, 245, 247, 0.5);
  }

  .pl-time {
    margin-left: 6px;
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    color: rgba(244, 245, 247, 0.85);
  }

  .pl-grow {
    flex: 1;
  }

  .pl-vol {
    width: 84px;
    accent-color: #fff;
  }

  .pl-queue {
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-xl);
    background: rgba(255, 255, 255, 0.05);
    overflow: hidden;
  }

  .pl-tr-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 0 10px 10px;
  }

  .pl-tr-body > :global(.tr) {
    flex: 1;
  }

  .pl-wait {
    display: flex;
    flex-direction: column;
    align-items: center;
  }

  .pl-conv-note {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    color: rgba(244, 245, 247, 0.75);
    text-align: center;
  }

  .pl-btn.primary {
    background: var(--accent);
    color: var(--on-accent, #fff);
  }

  .pl-btn.primary:hover {
    background: var(--accent-hi, var(--accent));
  }

  .pl-q-head {
    padding: 12px 14px 8px;
    font-size: var(--text-sm);
    font-weight: 650;
  }

  .pl-q-head span {
    margin-left: 4px;
    color: rgba(244, 245, 247, 0.5);
    font-weight: 500;
  }

  .pl-q-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .pl-q-item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px;
    border: none;
    border-radius: var(--radius-md);
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .pl-q-item:hover {
    background: rgba(255, 255, 255, 0.07);
  }

  .pl-q-item.cur {
    background: rgba(255, 255, 255, 0.13);
  }

  .pl-q-thumb {
    position: relative;
    width: 72px;
    aspect-ratio: 16 / 10;
    flex-shrink: 0;
    border-radius: var(--radius-sm);
    background: #22252c center / cover no-repeat;
    display: grid;
    place-items: center;
    overflow: hidden;
    color: rgba(255, 255, 255, 0.85);
  }

  .pl-q-prog {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 3px;
    background: var(--accent);
  }

  .pl-q-prog.done {
    background: rgba(255, 255, 255, 0.55);
  }

  .pl-q-body {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .pl-q-name,
  .pl-q-meta {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .pl-q-name {
    font-size: var(--text-sm);
  }

  .pl-q-meta {
    font-size: var(--text-xs);
    color: rgba(244, 245, 247, 0.5);
  }

  .pl-keys {
    margin: 0;
    text-align: center;
    font-size: var(--text-xs);
    color: rgba(244, 245, 247, 0.45);
  }

  @media (max-width: 860px) {
    .pl-shell.with-queue {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: auto minmax(0, 1fr) 180px;
    }
    .pl-btn span {
      display: none;
    }
    .pl-vol {
      display: none;
    }
  }
</style>
