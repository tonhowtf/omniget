<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { locale, t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { goTo } from "$lib/canvas/palette/navigate";
  import { MAX_ROWS, isUrgent, orderInbox, reasonKey, splitVisible, type InboxItem } from "$lib/canvas/attention/needs-you";

  let { mini = false }: { mini?: boolean } = $props();

  const engine = useEngine();

  let items = $state<InboxItem[]>([]);
  let now = $state(Date.now());
  let expanded = $state(false);
  let view = $derived(expanded ? { shown: items, more: 0 } : splitVisible(items));
  let urgent = $derived(items.some((i) => isUrgent(i.reason)));
  let countText = $derived($t("canvas.needsYou.count", { count: items.length }) as string);

  function span(ms: number, lang: string): string {
    const minutes = Math.max(1, Math.round(ms / 60000));
    const hours = minutes >= 90;
    const value = hours ? Math.round(minutes / 60) : minutes;
    try {
      return new Intl.NumberFormat(lang, { style: "unit", unit: hours ? "hour" : "minute", unitDisplay: "narrow" }).format(value);
    } catch {
      return hours ? `${value} h` : `${value} min`;
    }
  }

  function waited(item: InboxItem): string {
    if (!item.sinceMs || now - item.sinceMs < 60000) return "";
    return span(now - item.sinceMs, $locale);
  }

  function open(item: InboxItem) {
    void goTo(engine, { workspaceId: item.workspaceId, floorId: item.floorId || "ground", nodeId: item.nodeId }, { focus: true });
  }

  onMount(() => {
    let alive = true;
    let run = 0;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let clock: ReturnType<typeof setInterval> | null = null;

    const load = async () => {
      const mine = ++run;
      let rows: InboxItem[] = [];
      try {
        rows = ((await invoke("canvas_inbox", { workspaceId: null })) as InboxItem[]) ?? [];
      } catch {
        rows = [];
      }
      if (!alive || mine !== run) return;
      items = orderInbox(rows);
      if (items.length <= MAX_ROWS) expanded = false;
      now = Date.now();
      if (items.length && !clock) {
        clock = setInterval(() => (now = Date.now()), 60000);
      } else if (!items.length && clock) {
        clearInterval(clock);
        clock = null;
      }
    };
    const soon = () => {
      if (timer) return;
      timer = setTimeout(() => {
        timer = null;
        void load();
      }, 300);
    };
    const unlisten: Promise<UnlistenFn | null>[] = [
      listen("canvas://node-state", soon),
      listen("canvas://inbox", soon),
      listen("llm://tool-ask", soon),
      listen("omni://tool-resolved", soon),
      listen("term://state", soon),
      listen("term://exit", soon),
    ].map((p) => p.catch(() => null));
    void load();
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
      if (clock) clearInterval(clock);
      for (const p of unlisten) void p.then((off) => off?.());
    };
  });
</script>

{#if items.length}
  {#if mini}
    <button
      type="button"
      class="ny-count"
      class:calm={!urgent}
      title={$t("canvas.needsYou.next") as string}
      aria-label={countText}
      onclick={() => void engine.nextAttention()}
    >
      {items.length > 9 ? "9+" : items.length}
    </button>
  {:else}
    <section class="ny" aria-labelledby="ny-title">
      <h2 class="ny-head" id="ny-title" aria-label={countText}>
        {$t("canvas.needsYou.title")}
        <span class="ny-badge" class:calm={!urgent} aria-hidden="true">{items.length}</span>
      </h2>
      <ul class="ny-list">
        {#each view.shown as item (item.nodeId)}
          {@const time = waited(item)}
          <li>
            <button type="button" class="ny-row" onclick={() => open(item)}>
              <span class="ny-dot" class:review={!isUrgent(item.reason)} aria-hidden="true"></span>
              <span class="ny-text">
                <bdi class="ny-name">{item.name || $t("canvas.needsYou.agent")}</bdi>
                <span class="ny-why">{$t(`canvas.needsYou.reason.${reasonKey(item.reason)}`)}</span>
              </span>
              {#if time}<span class="ny-time">{time}</span>{/if}
            </button>
          </li>
        {/each}
      </ul>
      {#if view.more}
        <button type="button" class="ny-more" onclick={() => (expanded = true)}>
          {$t("canvas.needsYou.more", { count: view.more })}
        </button>
      {/if}
    </section>
  {/if}
{/if}

<style>
  .ny {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 6px 8px;
  }
  .ny-head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    padding: 2px 8px 4px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
  }
  .ny-badge,
  .ny-count {
    display: inline-grid;
    place-items: center;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 999px;
    background: var(--agent-blocked);
    color: var(--bg);
    font-size: 11px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .ny-badge.calm,
  .ny-count.calm {
    background: var(--agent-success);
  }
  .ny-count {
    width: 30px;
    height: 30px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-md);
    font-size: 13px;
    cursor: pointer;
  }
  .ny-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ny-row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text);
    text-align: start;
    cursor: pointer;
  }
  .ny-row:hover {
    background: var(--surface-hi);
  }
  .ny-row:focus-visible,
  .ny-more:focus-visible,
  .ny-count:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 1px;
  }
  .ny-dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--agent-blocked);
  }
  .ny-dot.review {
    background: var(--agent-success);
  }
  .ny-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .ny-name {
    font-size: 13px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ny-why {
    font-size: 11px;
    line-height: 1.3;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }
  .ny-time {
    flex: none;
    font-size: 11px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .ny-more {
    align-self: flex-start;
    margin: 2px 8px 0;
    padding: 2px 0;
    border: 0;
    background: transparent;
    color: var(--text-muted);
    font-size: 11px;
    cursor: pointer;
  }
  .ny-more:hover {
    color: var(--text);
  }
</style>
