<script lang="ts">
  import { onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import * as api from "$lib/video/api";
  import { player } from "$lib/video/player.svelte";
  import type { CastDevice, CastStatus, SourceRef } from "$lib/video/types";
  import VIcon from "./parts/VIcon.svelte";
  import { errText, fmtClock } from "./parts/util";

  let {
    source = null,
    startMs = undefined,
    onclose,
  }: {
    source?: SourceRef | null;
    startMs?: number;
    onclose: () => void;
  } = $props();

  let devices = $state<CastDevice[] | null>(null);
  let searching = $state(false);
  let status = $state<CastStatus | null>(null);
  let connecting = $state<string | null>(null);
  let error = $state<string | null>(null);
  let seekDrag = $state<number | null>(null);
  let card: HTMLDivElement | undefined = $state();

  let what = $derived(source ?? player.info?.source ?? null);
  let active = $derived(!!status && status.state !== "idle" && !!status.device);

  async function discover() {
    searching = true;
    error = null;
    try {
      devices = await api.castDiscover();
    } catch (e) {
      error = errText(e);
      devices ??= [];
    } finally {
      searching = false;
    }
  }

  onMount(() => {
    let off: (() => void) | null = null;
    let dead = false;
    void listen<CastStatus>("video-cast-status", (e) => (status = e.payload)).then((u) => {
      if (dead) u();
      else off = u;
    });
    void api
      .castStatus()
      .then((s) => (status = s))
      .catch(() => {});
    void discover();
    const prev = document.activeElement as HTMLElement | null;
    void tick().then(() => card?.querySelector<HTMLElement>("button")?.focus());
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onclose();
      }
    };
    window.addEventListener("keydown", key, true);
    return () => {
      dead = true;
      off?.();
      window.removeEventListener("keydown", key, true);
      prev?.focus?.();
    };
  });

  async function connect(d: CastDevice) {
    if (!what) return;
    connecting = d.id;
    error = null;
    try {
      const start = startMs ?? (source ? 0 : Math.round(player.position * 1000));
      status = await api.castStart(d.id, what, start);
    } catch (e) {
      error = errText(e);
    } finally {
      connecting = null;
    }
  }

  async function control(action: "play" | "pause" | "seek" | "volume" | "mute", value: number | null = null) {
    try {
      await api.castControl(action, value);
    } catch (e) {
      error = errText(e);
    }
  }

  async function stop() {
    try {
      await api.castStop();
      status = null;
    } catch (e) {
      error = errText(e);
    }
  }

  let deviceName = $derived(status?.device ? (devices?.find((d) => d.id === status!.device)?.name ?? status.device) : "");
</script>

<div
  class="dialog-backdrop"
  role="presentation"
  onpointerdown={(e) => {
    if (e.target === e.currentTarget) onclose();
  }}
>
  <div class="dialog-card cp" bind:this={card} role="dialog" aria-modal="true" aria-labelledby="cast-title">
    <div class="cp-h">
      <img src="/icons/video/cast-64.webp" alt="" width="32" height="32" />
      <h2 class="dialog-title" id="cast-title">{$t("video.cast.title")}</h2>
      <button class="btn btn-icon btn-sm btn-ghost" onclick={onclose} aria-label={$t("video.common.close") as string}><VIcon name="close" size={14} /></button>
    </div>

    {#if error}
      <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
    {/if}

    {#if active && status}
      <section class="remote" aria-label={$t("video.cast.remote") as string}>
        <p class="on">
          <VIcon name="cast" size={14} />
          <span>{$t("video.cast.playing_on", { device: deviceName })}</span>
        </p>
        {#if status.title}<b class="rt">{status.title}</b>{/if}
        <p class="st">
          {#if status.state === "connecting" || status.state === "buffering"}<span class="spinner"></span>{/if}
          {$t(`video.cast.state_${status.state}`)}
        </p>
        {#if status.error}<p class="err">{status.error}</p>{/if}

        {#if status.duration_ms > 0}
          <div class="seek">
            <span>{fmtClock(seekDrag ?? status.position_ms)}</span>
            <input
              type="range"
              min="0"
              max={status.duration_ms}
              step="1000"
              value={seekDrag ?? status.position_ms}
              oninput={(e) => (seekDrag = Number((e.currentTarget as HTMLInputElement).value))}
              onchange={(e) => {
                const v = Number((e.currentTarget as HTMLInputElement).value);
                seekDrag = null;
                void control("seek", v);
              }}
              aria-label={$t("video.cast.seek") as string}
            />
            <span>{fmtClock(status.duration_ms)}</span>
          </div>
        {/if}

        <div class="ctl">
          <button class="btn btn-icon" onclick={() => void control("seek", Math.max(0, status!.position_ms - 10_000))} aria-label={$t("video.cast.back10") as string}><VIcon name="replay" size={16} /></button>
          {#if status.state === "playing" || status.state === "buffering"}
            <button class="btn btn-icon btn-primary big" onclick={() => void control("pause")} aria-label={$t("video.cast.pause") as string}><VIcon name="pause" size={18} /></button>
          {:else}
            <button class="btn btn-icon btn-primary big" onclick={() => void control("play")} aria-label={$t("video.cast.play") as string}><VIcon name="play" size={18} /></button>
          {/if}
          <button class="btn btn-icon" onclick={() => void stop()} aria-label={$t("video.cast.stop") as string}><VIcon name="stop" size={16} /></button>
        </div>

        <div class="vol">
          <button class="btn btn-icon btn-sm btn-ghost" onclick={() => void control("mute", status!.muted ? 0 : 1)} aria-label={(status.muted ? $t("video.cast.unmute") : $t("video.cast.mute")) as string}>
            <VIcon name={status.muted ? "mute" : "volume"} size={15} />
          </button>
          <input
            type="range"
            min="0"
            max="1"
            step="0.05"
            value={status.volume}
            onchange={(e) => void control("volume", Number((e.currentTarget as HTMLInputElement).value))}
            aria-label={$t("video.cast.volume") as string}
          />
        </div>
      </section>
      <div class="sep"></div>
    {/if}

    <div class="dh">
      <h3>{$t("video.cast.devices")}</h3>
      <button class="btn btn-sm btn-ghost" onclick={() => void discover()} disabled={searching}>
        <VIcon name="refresh" size={13} spin={searching} /> {$t("video.cast.search_again")}
      </button>
    </div>

    {#if !what && !active}
      <p class="hint">{$t("video.cast.nothing_open")}</p>
    {/if}

    {#if devices === null}
      <div class="searching"><span class="spinner"></span> {$t("video.cast.searching")}</div>
    {:else if !devices.length}
      <div class="none">
        <p><b>{$t("video.cast.none_title")}</b></p>
        <p class="hint">{$t("video.cast.none_body")}</p>
      </div>
    {:else}
      <ul class="devs" role="list">
        {#each devices as d (d.id)}
          <li>
            <button class="dev" class:cur={status?.device === d.id && active} onclick={() => void connect(d)} disabled={!what || connecting != null}>
              <span class="di"><VIcon name="tv" size={18} /></span>
              <span class="dt"><b>{d.name}</b><small>{d.model || d.host}</small></span>
              {#if connecting === d.id}<span class="spinner"></span>{:else if status?.device === d.id && active}<span class="tag tag-success">{$t("video.cast.connected")}</span>{/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    <p class="foot">{$t("video.cast.lan_note")}</p>
  </div>
</div>

<style>
  .cp {
    width: min(420px, 100%);
    max-height: 86vh;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .cp-h {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .cp-h h2 {
    flex: 1;
    margin: 0;
  }
  .remote {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--fill-1);
  }
  .on {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--accent-text);
    font-size: var(--text-sm);
    font-weight: 600;
  }
  .rt {
    font-size: var(--text-md);
  }
  .st {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: var(--text-sm);
  }
  .seek,
  .vol {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .seek input,
  .vol input {
    flex: 1;
    accent-color: var(--accent);
  }
  .ctl {
    display: flex;
    justify-content: center;
    align-items: center;
    gap: var(--space-3);
  }
  .big {
    width: 44px;
    height: 44px;
    border-radius: 50%;
  }
  .sep {
    height: var(--hairline);
    background: var(--separator);
  }
  .dh {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h3 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 650;
  }
  .hint,
  .foot {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .foot {
    font-size: var(--text-xs);
  }
  .searching {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-dim);
    padding: var(--space-3) 0;
  }
  .none p {
    margin: 0 0 4px;
  }
  .devs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .dev {
    all: unset;
    box-sizing: border-box;
    width: 100%;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 8px;
    border-radius: var(--radius-md);
    cursor: pointer;
  }
  .dev:hover:not(:disabled) {
    background: var(--fill-1);
  }
  .dev:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .dev.cur {
    background: var(--accent-soft);
  }
  .dev:focus-visible {
    outline: var(--focus-ring);
  }
  .di {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
  }
  .dt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .dt b {
    font-weight: 600;
  }
  .dt small {
    color: var(--text-dim);
    font-size: var(--text-xs);
  }
</style>
