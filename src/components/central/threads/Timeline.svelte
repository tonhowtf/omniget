<script lang="ts">
  // Virtualized timeline: rows measured with one ResizeObserver, only the
  // visible window (plus overscan) is in the DOM. Follows the end while the
  // user is within 40 px of it; wheel/keys up break the follow and a "scroll
  // to end" pill appears. "Load earlier turns" keeps the reading position.
  // A minimap rail of user turns sits on the left gutter.
  import { tick, untrack } from "svelte";
  import { t } from "$lib/i18n";
  import type { Row } from "$lib/central/threads-ui/timeline";
  import TimelineRow, { type TimelineActions } from "./TimelineRow.svelte";
  import Icon from "./Icon.svelte";
  import type { Snippet } from "svelte";

  let {
    rows,
    threadId,
    cwd,
    actions,
    expandedIds,
    hasMore = false,
    loadingOlder = false,
    onloadolder,
    footer,
    bottomInset = 0,
  }: {
    rows: Row[];
    threadId: string;
    cwd: string | null;
    actions: TimelineActions;
    expandedIds: Set<string>;
    hasMore?: boolean;
    loadingOlder?: boolean;
    onloadolder?: () => void | Promise<void>;
    footer?: Snippet;
    bottomInset?: number;
  } = $props();

  const EST = 72;
  const OVERSCAN_PX = 900;
  const END_BAND = 40;

  let scroller = $state<HTMLDivElement | null>(null);
  let heights = $state<Record<string, number>>({});
  let scrollTop = $state(0);
  let viewport = $state(600);
  let following = $state(true);
  let atEnd = $state(true);
  let showPill = $state(false);
  let pillTimer: ReturnType<typeof setTimeout> | null = null;
  let hoverTick = $state<number | null>(null);

  const ro = typeof ResizeObserver !== "undefined"
    ? new ResizeObserver((entries) => {
        let changed = false;
        const next = { ...heights };
        for (const e of entries) {
          const id = (e.target as HTMLElement).dataset.rowId;
          if (!id) continue;
          const h = Math.ceil(e.borderBoxSize?.[0]?.blockSize ?? (e.target as HTMLElement).offsetHeight);
          if (h > 0 && next[id] !== h) {
            next[id] = h;
            changed = true;
          }
        }
        if (changed) {
          heights = next;
          if (following) queueMicrotask(stickToEnd);
        }
      })
    : null;

  function measure(node: HTMLElement) {
    ro?.observe(node);
    return { destroy: () => ro?.unobserve(node) };
  }

  let offsets = $derived.by(() => {
    const out = new Array<number>(rows.length + 1);
    out[0] = 0;
    for (let i = 0; i < rows.length; i++) out[i + 1] = out[i] + (heights[rows[i].id] ?? EST);
    return out;
  });
  let total = $derived(offsets[rows.length] ?? 0);

  function indexAt(y: number): number {
    let lo = 0;
    let hi = rows.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (offsets[mid + 1] <= y) lo = mid + 1;
      else hi = mid;
    }
    return Math.min(lo, Math.max(0, rows.length - 1));
  }

  let range = $derived.by(() => {
    if (!rows.length) return { start: 0, end: -1 };
    const start = indexAt(Math.max(0, scrollTop - OVERSCAN_PX));
    const end = indexAt(scrollTop + viewport + OVERSCAN_PX);
    return { start, end };
  });
  let visible = $derived(rows.slice(range.start, range.end + 1));
  let padTop = $derived(offsets[range.start] ?? 0);
  let padBottom = $derived(Math.max(0, total - (offsets[range.end + 1] ?? total)));

  function stickToEnd() {
    const el = scroller;
    if (!el || !following) return;
    el.scrollTop = el.scrollHeight;
  }

  function onScroll() {
    const el = scroller;
    if (!el) return;
    scrollTop = el.scrollTop;
    viewport = el.clientHeight;
    atEnd = el.scrollHeight - el.scrollTop - el.clientHeight <= END_BAND;
    if (atEnd) following = true;
    if (atEnd) {
      if (pillTimer) clearTimeout(pillTimer);
      showPill = false;
    } else if (!showPill && !pillTimer) {
      pillTimer = setTimeout(() => {
        pillTimer = null;
        showPill = !atEnd;
      }, 150);
    }
    if (el.scrollTop < 200 && hasMore && !loadingOlder && onloadolder) void loadOlder();
  }

  function breakFollow() {
    if (!atEnd || (scroller && scroller.scrollHeight > scroller.clientHeight)) following = false;
  }

  function onWheel(e: WheelEvent) {
    if (e.deltaY < 0) breakFollow();
  }

  function onKey(e: KeyboardEvent) {
    if ((e.key === "PageUp" || e.key === "Home" || e.key === "ArrowUp") && !isEditable(e.target)) breakFollow();
  }

  function isEditable(el: EventTarget | null): boolean {
    const n = el as HTMLElement | null;
    return !!n && (n.isContentEditable || n.tagName === "INPUT" || n.tagName === "TEXTAREA");
  }

  export function scrollToEnd(smooth = true) {
    following = true;
    scroller?.scrollTo({ top: scroller.scrollHeight, behavior: smooth ? "smooth" : "auto" });
  }

  export async function scrollToRow(id: string) {
    const i = rows.findIndex((r) => r.id === id);
    if (i < 0 || !scroller) return;
    following = false;
    scroller.scrollTo({ top: Math.max(0, offsets[i] - 24), behavior: "smooth" });
  }

  async function loadOlder() {
    if (!onloadolder || !scroller) return;
    const anchor = rows[indexAt(scroller.scrollTop)];
    const delta = anchor ? scroller.scrollTop - offsets[rows.indexOf(anchor)] : 0;
    await onloadolder();
    await tick();
    if (anchor) {
      const i = rows.findIndex((r) => r.id === anchor.id);
      if (i >= 0 && scroller) scroller.scrollTop = offsets[i] + delta;
    }
  }

  // New rows / streamed text: stay at the end while following.
  $effect(() => {
    rows;
    total;
    untrack(() => {
      if (following) void tick().then(stickToEnd);
    });
  });

  // Thread switch: jump to the end immediately.
  $effect(() => {
    threadId;
    untrack(() => {
      following = true;
      heights = {};
      void tick().then(() => {
        stickToEnd();
        onScroll();
      });
    });
  });

  $effect(() => {
    const el = scroller;
    if (!el || typeof ResizeObserver === "undefined") return;
    const r = new ResizeObserver(() => {
      viewport = el.clientHeight;
      if (following) stickToEnd();
    });
    r.observe(el);
    return () => r.disconnect();
  });

  // ── Minimap ───────────────────────────────────────────────────────────
  type Tick = { id: string; index: number; text: string; answer: string };
  let ticks = $derived.by(() => {
    const out: Tick[] = [];
    for (let i = 0; i < rows.length; i++) {
      const r = rows[i];
      if (r.kind !== "user") continue;
      let answer = "";
      for (let j = i + 1; j < rows.length; j++) {
        const n = rows[j];
        if (n.kind === "user") break;
        if (n.kind === "meta") {
          answer = n.answer;
          break;
        }
      }
      out.push({ id: r.id, index: i, text: r.message.text, answer });
    }
    return out;
  });
  let inView = $derived.by(() => {
    const top = scrollTop;
    const bottom = scrollTop + viewport;
    const set = new Set<string>();
    for (let k = 0; k < ticks.length; k++) {
      const start = offsets[ticks[k].index];
      const end = k + 1 < ticks.length ? offsets[ticks[k + 1].index] : total;
      if (end > top && start < bottom) set.add(ticks[k].id);
    }
    return set;
  });

  function jump(k: number) {
    const tk = ticks[k];
    if (tk) void scrollToRow(tk.id);
  }

  function onRailKey(e: KeyboardEvent, k: number) {
    let next = k;
    if (e.key === "ArrowDown") next = Math.min(ticks.length - 1, k + 1);
    else if (e.key === "ArrowUp") next = Math.max(0, k - 1);
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = ticks.length - 1;
    else return;
    e.preventDefault();
    const btns = (e.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLButtonElement>("button.tick");
    btns?.[next]?.focus();
    jump(next);
  }
</script>

<div class="timeline-wrap">
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="scroller"
    bind:this={scroller}
    onscroll={onScroll}
    onwheel={onWheel}
    onkeydown={onKey}
    onpointerdown={(e) => {
      if (e.target === scroller) breakFollow();
    }}
    tabindex="0"
    role="log"
    aria-label={$t("llm.central.threads.timeline")}
    aria-live="off"
  >
    <div class="column">
      {#if hasMore}
        <div class="older">
          <button type="button" class="older-btn" disabled={loadingOlder} onclick={loadOlder}>
            {loadingOlder ? $t("llm.central.threads.loading_older") : $t("llm.central.threads.load_older")}
          </button>
        </div>
      {/if}
      {#if !rows.length}
        <p class="empty">{$t("llm.central.threads.empty_thread")}</p>
      {/if}
      <div style:height="{padTop}px" aria-hidden="true"></div>
      {#each visible as row (row.id)}
        <div class="row" data-row-id={row.id} data-kind={row.kind} use:measure>
          <TimelineRow {row} {cwd} {actions} {expandedIds} />
        </div>
      {/each}
      <div style:height="{padBottom}px" aria-hidden="true"></div>
      {#if footer}{@render footer()}{/if}
      <div style:height="{bottomInset}px" aria-hidden="true"></div>
    </div>
  </div>

  {#if ticks.length >= 2}
    <nav class="minimap" aria-label={$t("llm.central.threads.minimap")}>
      <button type="button" class="nav-btn" aria-label={$t("llm.central.threads.prev_turn")} onclick={() => jump(Math.max(0, ticks.findIndex((x) => inView.has(x.id)) - 1))}>
        <Icon name="caret-up" size={10} />
      </button>
      <div class="rail">
        {#each ticks as tk, k (tk.id)}
          <button
            type="button"
            class="tick"
            class:in={inView.has(tk.id)}
            class:near={hoverTick != null && Math.abs(hoverTick - k) === 1}
            class:hot={hoverTick === k}
            aria-label={tk.text.slice(0, 80)}
            onmouseenter={() => (hoverTick = k)}
            onmouseleave={() => (hoverTick = null)}
            onfocus={() => (hoverTick = k)}
            onblur={() => (hoverTick = null)}
            onclick={() => jump(k)}
            onkeydown={(e) => onRailKey(e, k)}
          ></button>
        {/each}
      </div>
      <button type="button" class="nav-btn" aria-label={$t("llm.central.threads.next_turn")} onclick={() => {
        const last = ticks.map((x) => inView.has(x.id)).lastIndexOf(true);
        jump(Math.min(ticks.length - 1, last + 1));
      }}>
        <Icon name="caret-down" size={10} />
      </button>
      {#if hoverTick != null && ticks[hoverTick]}
        <div class="preview" role="tooltip">
          <p class="p-user">{ticks[hoverTick].text.split("\n")[0]}</p>
          {#if ticks[hoverTick].answer}<p class="p-answer">{ticks[hoverTick].answer}</p>{/if}
        </div>
      {/if}
    </nav>
  {/if}

  {#if showPill && !atEnd}
    <button type="button" class="pill" style:bottom="{bottomInset + 12}px" onpointerdown={(e) => e.preventDefault()} onclick={() => scrollToEnd()}>
      <Icon name="arrow-down" size={12} />{$t("llm.central.threads.scroll_end")}
    </button>
  {/if}
</div>

<style>
  .timeline-wrap { position: relative; flex: 1; min-height: 0; display: flex; }
  .scroller { flex: 1; min-height: 0; overflow-y: auto; overflow-x: hidden; overscroll-behavior-y: contain; overflow-anchor: none; scrollbar-gutter: stable both-edges; outline: none; }
  .column { max-width: 800px; margin: 0 auto; padding: 16px 28px 0 36px; box-sizing: border-box; min-width: 0; }
  .row { min-width: 0; overflow-x: clip; }
  .older { display: flex; justify-content: center; padding: 6px 0 10px; }
  .older-btn { border: 0; background: var(--fill-1); color: var(--text-muted); font-size: 12px; padding: 5px 12px; border-radius: 999px; cursor: pointer; }
  .older-btn:hover:not(:disabled) { color: var(--text); background: var(--fill-2); }
  .empty { color: var(--text-muted); text-align: center; margin: 22vh 0 0; font-size: 14px; }
  .minimap { position: absolute; left: 6px; top: 50%; transform: translateY(-50%); display: flex; flex-direction: column; align-items: flex-start; gap: 4px; max-height: calc(100% - 18rem); z-index: 5; }
  @media (pointer: coarse) { .minimap { display: none; } }
  .rail { display: flex; flex-direction: column; gap: 5px; overflow: hidden; padding: 2px 0; }
  .tick { width: 10px; height: 3px; border: 0; padding: 0; border-radius: 2px; background: color-mix(in srgb, var(--text) 22%, transparent); cursor: pointer; transition: width 120ms var(--ease-out, ease); }
  .tick.in { background: color-mix(in srgb, var(--text) 85%, transparent); }
  .tick.near { width: 16px; }
  .tick.hot, .tick:focus-visible { width: 24px; background: var(--accent); outline: none; }
  .nav-btn { border: 0; background: transparent; color: var(--text-muted); padding: 2px; border-radius: 4px; cursor: pointer; display: inline-flex; opacity: 0.6; }
  .nav-btn:hover { opacity: 1; background: var(--fill-1); }
  .preview { position: absolute; left: 34px; top: 50%; transform: translateY(-50%); width: 280px; padding: 10px 12px; border-radius: 12px; background: var(--popup-bg, var(--surface)); border: 1px solid var(--separator); box-shadow: 0 12px 32px color-mix(in srgb, #000 22%, transparent); pointer-events: none; }
  .p-user { margin: 0 0 6px; font-weight: 600; font-size: 12.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .p-answer { margin: 0; font-size: 12px; color: var(--text-muted); display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
  .pill { position: absolute; left: 50%; transform: translateX(-50%); display: inline-flex; align-items: center; gap: 6px; border: 1px solid var(--separator); background: color-mix(in srgb, var(--popup-bg, var(--surface)) 88%, transparent); backdrop-filter: blur(12px); color: var(--text); font-size: 12px; padding: 6px 12px; border-radius: 999px; cursor: pointer; box-shadow: 0 6px 18px color-mix(in srgb, #000 16%, transparent); z-index: 6; }
</style>
