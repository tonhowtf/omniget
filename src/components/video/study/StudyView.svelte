<script lang="ts">
  // Study: review due flashcards (flip, grade 0-3), browse every card and
  // send them to Anki. Cards are made in the player (C, or from a transcript
  // line) or here by hand.
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { fmtTime } from "$lib/video/format";
  import type { Card } from "$lib/video/types";
  import VIcon from "$components/video/player/VIcon.svelte";

  let tab = $state<"review" | "all">("review");
  let due = $state<Card[]>([]);
  let all = $state<Card[]>([]);
  let loading = $state(true);
  let flipped = $state(false);
  let reviewed = $state(0);
  let selected = $state<Set<number>>(new Set());
  let exporting = $state(false);
  let adding = $state(false);
  let front = $state("");
  let back = $state("");

  $effect(() => {
    void load();
  });

  async function load() {
    loading = true;
    try {
      [due, all] = await Promise.all([api.cardsDue(), api.cards(null)]);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      loading = false;
    }
  }

  let card = $derived(due[0] ?? null);

  async function grade(g: 0 | 1 | 2 | 3) {
    const c = card;
    if (!c) return;
    try {
      const updated = await api.cardReview(c.id, g);
      reviewed++;
      flipped = false;
      due = due.slice(1);
      // "again" comes back in this session
      if (g === 0) due = [...due, updated];
      all = all.map((x) => (x.id === updated.id ? updated : x));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function keys(e: KeyboardEvent) {
    if (tab !== "review" || !card) return;
    const el = e.target as HTMLElement | null;
    if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA")) return;
    if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      flipped = !flipped;
    } else if (flipped && /^[1-4]$/.test(e.key)) {
      void grade((Number(e.key) - 1) as 0 | 1 | 2 | 3);
    }
  }

  async function remove(id: number) {
    try {
      await api.cardDelete(id);
      all = all.filter((c) => c.id !== id);
      due = due.filter((c) => c.id !== id);
      const s = new Set(selected);
      s.delete(id);
      selected = s;
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function exportAnki() {
    const ids = selected.size ? [...selected] : all.map((c) => c.id);
    if (!ids.length) return;
    exporting = true;
    try {
      const r = await api.ankiExport(ids);
      showToast("success", $t("video.study.anki_done", { count: r.added }) as string);
    } catch (e) {
      showToast("error", `${$t("video.study.anki_failed")}: ${String(e)}`);
    } finally {
      exporting = false;
    }
  }

  async function addManual(e: Event) {
    e.preventDefault();
    if (!front.trim()) return;
    try {
      const c = await api.cardAdd("manual", front.trim(), back.trim(), null, null);
      all = [c, ...all];
      due = [...due, c];
      front = "";
      back = "";
      adding = false;
    } catch (err) {
      showToast("error", String(err));
    }
  }

  function toggleSel(id: number) {
    const s = new Set(selected);
    if (s.has(id)) s.delete(id);
    else s.add(id);
    selected = s;
  }

  function dueLabel(c: Card): string {
    const days = Math.round((c.due - Date.now()) / 86_400_000);
    if (days <= 0) return $t("video.study.due_now") as string;
    return $t("video.study.due_in", { days }) as string;
  }

  const GRADES: { g: 0 | 1 | 2 | 3; key: string }[] = [
    { g: 0, key: "again" },
    { g: 1, key: "hard" },
    { g: 2, key: "good" },
    { g: 3, key: "easy" },
  ];
</script>

<svelte:window onkeydown={keys} />

<div class="study">
  <header>
    <div>
      <h1>{$t("video.study.title")}</h1>
      <p class="sub">{$t("video.study.summary", { due: due.length, total: all.length })}</p>
    </div>
    <div class="seg mac-segmented" role="tablist">
      <button role="tab" class="mac-segmented-btn" class:active={tab === "review"} aria-selected={tab === "review"} onclick={() => (tab = "review")}>{$t("video.study.review")}</button>
      <button role="tab" class="mac-segmented-btn" class:active={tab === "all"} aria-selected={tab === "all"} onclick={() => (tab = "all")}>{$t("video.study.all_cards")}</button>
    </div>
  </header>

  {#if loading}
    <div class="loading"><VIcon name="loader" size={22} spin /></div>
  {:else if tab === "review"}
    {#if card}
      <p class="count">{$t("video.study.progress", { done: reviewed, left: due.length })}</p>
      <div class="flip" class:flipped>
        <button class="face front" onclick={() => (flipped = true)} aria-label={$t("video.study.show_answer") as string} tabindex={flipped ? -1 : 0}>
          <span class="k">{$t("video.study.front")}</span>
          <p>{card.front}</p>
          {#if card.image}{@const src = api.serverUrl(card.image)}{#if src}<img {src} alt="" />{/if}{/if}
          <span class="tap">{$t("video.study.tap_to_flip")}</span>
        </button>
        <div class="face back" aria-hidden={!flipped}>
          <span class="k">{$t("video.study.back")}</span>
          <p class="q">{card.front}</p>
          <p>{card.back || "—"}</p>
          {#if card.t_ms != null}<span class="ts"><VIcon name="clock" size={11} /> {fmtTime(card.t_ms)}</span>{/if}
        </div>
      </div>
      {#if flipped}
        <div class="grades">
          {#each GRADES as gr, i (gr.g)}
            <button class="grade g{gr.g}" onclick={() => grade(gr.g)}>
              <strong>{$t(`video.study.grade_${gr.key}`)}</strong>
              <kbd>{i + 1}</kbd>
            </button>
          {/each}
        </div>
      {:else}
        <div class="grades"><button class="btn btn-primary" onclick={() => (flipped = true)}>{$t("video.study.show_answer")} <kbd>Space</kbd></button></div>
      {/if}
    {:else}
      <div class="empty">
        <img src="/icons/video/flashcards-128.webp" alt="" width="84" height="84" />
        <h2>{all.length ? $t("video.study.caught_up") : $t("video.study.empty_title")}</h2>
        <p>{all.length ? $t("video.study.caught_up_hint") : $t("video.study.empty_hint")}</p>
        {#if reviewed}<p class="done">{$t("video.study.reviewed", { count: reviewed })}</p>{/if}
      </div>
    {/if}
  {:else}
    <div class="bar">
      <button class="btn btn-sm" onclick={() => (adding = !adding)}><VIcon name="plus" size={13} /> {$t("video.study.new_card")}</button>
      <span class="sp"></span>
      {#if selected.size}<span class="sel">{$t("video.study.selected", { count: selected.size })}</span>{/if}
      <button class="btn btn-sm btn-primary" onclick={exportAnki} disabled={exporting || !all.length}>
        <VIcon name={exporting ? "loader" : "export"} size={13} spin={exporting} />
        {selected.size ? $t("video.study.anki_selected") : $t("video.study.anki_all")}
      </button>
    </div>
    {#if adding}
      <form class="add" onsubmit={addManual}>
        <textarea rows="2" bind:value={front} placeholder={$t("video.study.front") as string}></textarea>
        <textarea rows="2" bind:value={back} placeholder={$t("video.study.back") as string}></textarea>
        <div class="aa">
          <button type="button" class="btn btn-sm" onclick={() => (adding = false)}>{$t("video.cancel")}</button>
          <button type="submit" class="btn btn-sm btn-primary" disabled={!front.trim()}>{$t("video.study.add_card")}</button>
        </div>
      </form>
    {/if}
    {#if all.length}
      <ul class="cards">
        {#each all as c (c.id)}
          <li>
            <input type="checkbox" checked={selected.has(c.id)} onchange={() => toggleSel(c.id)} aria-label={c.front} />
            <div class="ct">
              <p class="cf">{c.front}</p>
              {#if c.back}<p class="cb">{c.back}</p>{/if}
            </div>
            {#if c.t_ms != null}<span class="ts">{fmtTime(c.t_ms)}</span>{/if}
            <span class="due">{dueLabel(c)}</span>
            <button class="ic" onclick={() => remove(c.id)} aria-label={$t("video.study.delete") as string} title={$t("video.study.delete") as string}><VIcon name="trash" size={13} /></button>
          </li>
        {/each}
      </ul>
    {:else}
      <div class="empty">
        <img src="/icons/video/flashcards-128.webp" alt="" width="84" height="84" />
        <h2>{$t("video.study.empty_title")}</h2>
        <p>{$t("video.study.empty_hint")}</p>
      </div>
    {/if}
  {/if}
</div>

<style>
  .study {
    max-width: 860px;
    padding-bottom: var(--space-7);
  }
  header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
    flex-wrap: wrap;
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    letter-spacing: var(--track-tight);
  }
  .sub {
    margin: 2px 0 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .loading {
    display: grid;
    place-items: center;
    height: 200px;
    color: var(--text-muted);
  }
  .count {
    margin: 0 0 var(--space-2);
    text-align: center;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .flip {
    position: relative;
    max-width: 560px;
    height: 300px;
    margin: 0 auto;
    perspective: 1200px;
  }
  .face {
    all: unset;
    box-sizing: border-box;
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: var(--space-5);
    border-radius: var(--radius-2xl);
    background: var(--surface);
    box-shadow: var(--elev-2);
    text-align: center;
    backface-visibility: hidden;
    -webkit-backface-visibility: hidden;
    transition: transform var(--duration-bounce) var(--ease-out);
    overflow: hidden;
  }
  .front {
    cursor: pointer;
  }
  .front:focus-visible {
    outline: var(--focus-ring);
  }
  .back {
    transform: rotateY(180deg);
  }
  .flipped .front {
    transform: rotateY(-180deg);
  }
  .flipped .back {
    transform: rotateY(0);
  }
  .face p {
    margin: 0;
    font-size: var(--text-lg);
    line-height: var(--leading-lg);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .face p.q {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .face img {
    max-height: 110px;
    border-radius: var(--radius-md);
  }
  .k {
    position: absolute;
    top: 14px;
    left: 18px;
    font-size: var(--text-caption);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
    color: var(--text-muted);
  }
  .tap {
    position: absolute;
    bottom: 14px;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .ts {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-xs);
    color: var(--accent-hi);
    font-variant-numeric: tabular-nums;
  }
  .grades {
    display: flex;
    justify-content: center;
    gap: 8px;
    margin-top: var(--space-4);
  }
  .grade {
    all: unset;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    min-width: 88px;
    padding: 8px 12px;
    border-radius: var(--radius-lg);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    cursor: pointer;
    font-size: var(--text-sm);
  }
  .grade:hover {
    background: var(--fill-2);
  }
  .grade:focus-visible {
    outline: var(--focus-ring);
  }
  .g0 strong {
    color: var(--danger);
  }
  .g1 strong {
    color: var(--warning);
  }
  .g2 strong {
    color: var(--success);
  }
  .g3 strong {
    color: var(--blue);
  }
  kbd {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    padding: 0 5px;
    border-radius: var(--radius-xs);
    background: var(--fill-2);
    color: var(--text-muted);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-7) auto;
    max-width: 420px;
    text-align: center;
  }
  .empty h2 {
    margin: var(--space-2) 0 0;
    font-size: var(--text-lg);
  }
  .empty p {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .empty .done {
    color: var(--success);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: var(--space-3);
  }
  .sp {
    flex: 1;
  }
  .sel {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .add {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-bottom: var(--space-3);
    padding: 12px;
    border-radius: var(--radius-lg);
    background: var(--fill-1);
  }
  .add textarea {
    resize: vertical;
    padding: 8px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
  }
  .aa {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .cards {
    list-style: none;
    margin: 0;
    padding: 0;
    border-radius: var(--radius-xl);
    background: var(--surface-mut);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .cards li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
  }
  .cards li + li {
    border-top: var(--hairline) solid var(--separator);
  }
  .cards input {
    accent-color: var(--accent);
  }
  .ct {
    flex: 1;
    min-width: 0;
  }
  .cf,
  .cb {
    margin: 0;
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cb {
    color: var(--text-dim);
  }
  .due {
    font-size: var(--text-xs);
    color: var(--text-muted);
    white-space: nowrap;
  }
  .ic {
    all: unset;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    color: var(--text-muted);
    cursor: pointer;
  }
  .ic:hover {
    background: var(--fill-2);
    color: var(--danger);
  }
  .ic:focus-visible {
    outline: var(--focus-ring);
  }
  @media (prefers-reduced-motion: reduce) {
    .face {
      transition: none;
    }
  }
</style>
