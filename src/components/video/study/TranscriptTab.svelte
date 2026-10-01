<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { cueAt, fmtTime } from "$lib/video/format";
  import { loadTranscript, openPlayer, player, seek, transcribe } from "$lib/video/player.svelte";
  import type { TranscriptHit } from "$lib/video/types";
  import VIcon from "$components/video/player/VIcon.svelte";

  let query = $state("");
  let scope = $state<"video" | "course">("video");
  let hits = $state<TranscriptHit[]>([]);
  let searching = $state(false);
  let follow = $state(true);
  let hovering = $state(false);
  let listEl: HTMLElement | undefined = $state();
  let timer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    if (player.info && player.transcriptState === "idle") void loadTranscript();
  });

  let cues = $derived(player.transcript);
  let delay = $derived(cues === player.cues ? player.subs.delayMs : 0);
  let active = $derived(cueAt(cues, player.position * 1000 - delay));
  let q = $derived(query.trim().toLowerCase());
  let shown = $derived(
    q && scope === "video" ? cues.map((c, i) => ({ c, i })).filter((x) => x.c.text.toLowerCase().includes(q)) : cues.map((c, i) => ({ c, i })),
  );

  $effect(() => {
    const i = active;
    if (!follow || hovering || q || i < 0 || !listEl) return;
    listEl.querySelector<HTMLElement>(`[data-i="${i}"]`)?.scrollIntoView({ block: "center", behavior: "smooth" });
  });

  function onQuery(e: Event) {
    query = (e.currentTarget as HTMLInputElement).value;
    if (scope === "course") scheduleCourseSearch();
  }

  function scheduleCourseSearch() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(async () => {
      const text = query.trim();
      if (!text) {
        hits = [];
        return;
      }
      searching = true;
      try {
        hits = await api.transcriptSearch(text, player.info?.course_id ?? null);
      } catch (e) {
        showToast("error", String(e));
      } finally {
        searching = false;
      }
    }, 250);
  }

  function openHit(h: TranscriptHit) {
    if (h.key === player.info?.key) seek(h.start_ms / 1000);
    else if (h.media_id != null) void openPlayer({ kind: "local", id: h.media_id }, { title: h.title, startMs: h.start_ms });
  }

  async function generate() {
    try {
      await transcribe();
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function card(text: string, start: number) {
    player.cardDraft = { front: text, back: "", t_ms: start };
  }

  function mark(text: string): { pre: string; hit: string; post: string } | null {
    if (!q) return null;
    const i = text.toLowerCase().indexOf(q);
    if (i < 0) return null;
    return { pre: text.slice(0, i), hit: text.slice(i, i + q.length), post: text.slice(i + q.length) };
  }
</script>

<div class="tr">
  <div class="head">
    <label class="search">
      <VIcon name="search" size={13} />
      <input
        type="search"
        value={query}
        oninput={onQuery}
        onkeydown={(e) => e.stopPropagation()}
        placeholder={$t(scope === "course" ? "video.transcript.search_course" : "video.transcript.search") as string}
        aria-label={$t("video.transcript.search") as string}
      />
    </label>
    {#if player.info?.course_id != null}
      <div class="seg" role="radiogroup" aria-label={$t("video.transcript.scope") as string}>
        <button role="radio" aria-checked={scope === "video"} class:on={scope === "video"} onclick={() => (scope = "video")}>{$t("video.transcript.scope_video")}</button>
        <button
          role="radio"
          aria-checked={scope === "course"}
          class:on={scope === "course"}
          onclick={() => {
            scope = "course";
            scheduleCourseSearch();
          }}>{$t("video.transcript.scope_course")}</button
        >
      </div>
    {/if}
    <button class="icb" class:on={follow} onclick={() => (follow = !follow)} aria-pressed={follow} title={$t("video.transcript.follow") as string} aria-label={$t("video.transcript.follow") as string}>
      <VIcon name="list" size={14} />
    </button>
  </div>

  {#if scope === "course" && q}
    <ul class="list">
      {#if searching}<li class="muted"><VIcon name="loader" size={13} spin /> {$t("video.transcript.searching")}</li>{/if}
      {#each hits as h, i (`${h.key}:${h.start_ms}:${i}`)}
        <li>
          <button class="line" onclick={() => openHit(h)}>
            <span class="ts">{fmtTime(h.start_ms)}</span>
            <span class="txt"><small class="src">{h.title}</small>{h.text}</span>
          </button>
        </li>
      {:else}
        {#if !searching}<li class="muted">{$t("video.transcript.no_hits")}</li>{/if}
      {/each}
    </ul>
  {:else if player.transcriptState === "loading"}
    <div class="empty"><VIcon name="loader" size={18} spin /></div>
  {:else if !cues.length}
    <div class="empty">
      <img src="/icons/video/transcript-64.webp" alt="" width="40" height="40" />
      <p>{$t("video.transcript.empty")}</p>
      <button class="btn btn-sm btn-primary" onclick={generate} disabled={player.transcriptState === "working"}>
        {#if player.transcriptState === "working"}<VIcon name="loader" size={13} spin /> {$t("video.transcript.generating")}{:else}<VIcon name="sparkles" size={13} /> {$t("video.transcript.generate")}{/if}
      </button>
      <p class="hint">{$t("video.transcript.generate_hint")}</p>
    </div>
  {:else}
    <ul class="list" bind:this={listEl} onmouseenter={() => (hovering = true)} onmouseleave={() => (hovering = false)}>
      {#each shown as { c, i } (i)}
        {@const m = mark(c.text)}
        <li data-i={i}>
          <button class="line" class:active={i === active} class:past={i < active} onclick={() => seek((c.start_ms + delay) / 1000)}>
            <span class="ts">{fmtTime(c.start_ms)}</span>
            <span class="txt">{#if m}{m.pre}<mark>{m.hit}</mark>{m.post}{:else}{c.text}{/if}</span>
          </button>
          <button class="mini" onclick={() => card(c.text, c.start_ms)} title={$t("video.study.make_card") as string} aria-label={$t("video.study.make_card") as string}>
            <VIcon name="cards" size={12} />
          </button>
        </li>
      {:else}
        <li class="muted">{$t("video.transcript.no_hits")}</li>
      {/each}
    </ul>
    <div class="foot">
      <span>{$t("video.transcript.lines", { count: cues.length })}</span>
      <button class="link" onclick={generate} disabled={player.transcriptState === "working"}>
        {#if player.transcriptState === "working"}<VIcon name="loader" size={12} spin />{/if}
        {$t("video.transcript.regenerate")}
      </button>
    </div>
  {/if}
</div>

<style>
  .tr {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
  }
  .search {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 8px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text-muted);
    min-width: 0;
  }
  .search:focus-within {
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }
  .search input {
    all: unset;
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
    color: var(--text);
  }
  .seg {
    display: flex;
    padding: 2px;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
  }
  .seg button {
    all: unset;
    padding: 3px 8px;
    border-radius: var(--radius-xs);
    font-size: var(--text-xs);
    cursor: pointer;
    color: var(--text-dim);
  }
  .seg button.on {
    background: var(--surface-hi);
    color: var(--text);
    font-weight: 600;
    box-shadow: var(--elev-1);
  }
  .icb {
    all: unset;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    color: var(--text-muted);
    cursor: pointer;
  }
  .icb.on {
    background: var(--accent-soft);
    color: var(--accent-hi);
  }
  .icb:focus-visible,
  .seg button:focus-visible,
  .line:focus-visible,
  .mini:focus-visible {
    outline: var(--focus-ring);
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 0 6px 16px;
  }
  .list li {
    position: relative;
    display: flex;
    align-items: flex-start;
  }
  .line {
    all: unset;
    flex: 1;
    display: flex;
    gap: 10px;
    padding: 6px 8px;
    border-radius: var(--radius-md);
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--text-sm);
    line-height: var(--leading-sm);
    transition:
      background var(--duration-fast),
      color var(--duration-fast);
  }
  .line:hover {
    background: var(--fill-1);
    color: var(--text);
  }
  .line.past {
    color: var(--text-muted);
  }
  .line.active {
    background: var(--accent-soft);
    color: var(--text);
    font-weight: 550;
  }
  .ts {
    flex-shrink: 0;
    width: 44px;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    padding-top: 1px;
  }
  .active .ts {
    color: var(--accent-hi);
  }
  .txt {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .src {
    display: block;
    color: var(--text-muted);
    font-size: var(--text-xs);
  }
  mark {
    background: color-mix(in srgb, var(--warning) 35%, transparent);
    color: inherit;
    border-radius: 2px;
  }
  .mini {
    all: unset;
    position: absolute;
    right: 6px;
    top: 5px;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--surface-hi);
    color: var(--text-muted);
    box-shadow: var(--elev-1);
    opacity: 0;
    cursor: pointer;
  }
  .list li:hover .mini,
  .mini:focus-visible {
    opacity: 1;
  }
  .muted {
    padding: 12px 8px;
    color: var(--text-dim);
    font-size: var(--text-sm);
    gap: 6px;
    align-items: center;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 28px 16px;
    text-align: center;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .empty p {
    margin: 0;
  }
  .hint {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 12px;
    border-top: var(--hairline) solid var(--separator);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .link {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--accent-hi);
    cursor: pointer;
  }
  .link:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
