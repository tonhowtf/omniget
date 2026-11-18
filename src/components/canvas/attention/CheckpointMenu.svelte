<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { locale, t } from "$lib/i18n";
  import { agentActions } from "$lib/native-agent/actions";

  let { nodeId, working = false }: { nodeId: string; working?: boolean } = $props();

  type Item = { turn: number; atMs: number };
  type Listing = { repository: boolean; items: Item[] };
  type Changes = { turn: number; total: number; files: string[]; truncated: boolean };

  let open = $state(false);
  let loading = $state(false);
  let listing = $state<Listing | null>(null);
  let pending = $state<Changes | null>(null);
  let busy = $state(false);
  let notice = $state("");
  let canUndo = $state(false);
  let failed = $state(false);
  let host = $state<HTMLDivElement | null>(null);
  let pop = $state<HTMLDivElement | null>(null);
  let place = $state({ left: 0, top: 0 });
  let run = 0;

  const POP_WIDTH = 320;

  function portal(el: HTMLElement) {
    document.body.appendChild(el);
    return { destroy: () => el.remove() };
  }

  function anchor() {
    const r = host?.getBoundingClientRect();
    if (!r) return;
    const left = Math.min(Math.max(8, r.right - POP_WIDTH), window.innerWidth - POP_WIDTH - 8);
    const below = r.bottom + 6;
    place = { left, top: below + 240 > window.innerHeight ? Math.max(8, window.innerHeight - 380) : below };
  }

  function clock(ms: number): string {
    try {
      return new Date(ms).toLocaleString($locale, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
    } catch {
      return new Date(ms).toISOString();
    }
  }

  async function load() {
    const mine = ++run;
    loading = true;
    failed = false;
    try {
      const got = (await invoke("canvas_checkpoints", { nodeId })) as Listing;
      if (mine === run) listing = got;
    } catch {
      if (mine === run) {
        listing = null;
        failed = true;
      }
    } finally {
      if (mine === run) loading = false;
    }
  }

  function toggle() {
    open = !open;
    pending = null;
    notice = "";
    if (open) {
      anchor();
      void load();
    }
  }

  async function choose(item: Item) {
    const mine = ++run;
    busy = true;
    failed = false;
    try {
      const got = (await invoke("canvas_checkpoint_changes", { nodeId, turn: item.turn })) as Changes;
      if (mine === run) pending = got;
    } catch {
      if (mine === run) failed = true;
    } finally {
      busy = false;
    }
  }

  async function restore() {
    if (!pending || busy) return;
    busy = true;
    failed = false;
    try {
      const done = (await invoke("canvas_checkpoint_restore", { nodeId, turn: pending.turn })) as { files: string[] };
      notice = $t("canvas.checkpoint.done", { count: done.files.length }) as string;
      canUndo = true;
      pending = null;
      await load();
    } catch {
      failed = true;
    } finally {
      busy = false;
    }
  }

  async function undo() {
    if (busy) return;
    busy = true;
    failed = false;
    try {
      const done = (await invoke("canvas_checkpoint_undo", { nodeId })) as { files: string[] };
      notice = $t("canvas.checkpoint.undone", { count: done.files.length }) as string;
      canUndo = false;
    } catch {
      failed = true;
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    if (open && pop) pop.querySelector<HTMLElement>("button:not([disabled])")?.focus({ preventScroll: true });
  });

  onMount(() => {
    const close = (e: PointerEvent) => {
      const target = e.target as Node;
      if (open && !host?.contains(target) && !pop?.contains(target)) open = false;
    };
    const esc = (e: KeyboardEvent) => {
      if (!open || e.key !== "Escape") return;
      open = false;
      host?.querySelector<HTMLElement>("button")?.focus({ preventScroll: true });
    };
    const away = () => {
      if (open) open = false;
    };
    window.addEventListener("pointerdown", close, true);
    window.addEventListener("keydown", esc);
    window.addEventListener("resize", away);
    window.addEventListener("wheel", away, { passive: true });
    return () => {
      run++;
      window.removeEventListener("pointerdown", close, true);
      window.removeEventListener("keydown", esc);
      window.removeEventListener("resize", away);
      window.removeEventListener("wheel", away);
    };
  });
</script>

<div class="cp" bind:this={host}>
  <button type="button" class="cp-open" class:on={open} aria-expanded={open} title={$t("canvas.checkpoint.open") as string} aria-label={$t("canvas.checkpoint.open") as string} onclick={toggle}>
    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 12a8 8 0 1 0 2.3-5.7 M4 4v5h5 M12 8v4l3 2" /></svg>
  </button>
  {#if open}
    <div
      class="cp-pop"
      role="group"
      data-canvas-chrome
      aria-label={$t("canvas.checkpoint.title") as string}
      use:portal
      bind:this={pop}
      style:left="{place.left}px"
      style:top="{place.top}px"
      onpointerdown={(e) => e.stopPropagation()}
    >
      {#if pending}
        <p class="cp-title">{$t("canvas.checkpoint.confirm_title", { turn: pending.turn })}</p>
        <p class="cp-body">
          {pending.total ? $t("canvas.checkpoint.confirm_files", { count: pending.total }) : $t("canvas.checkpoint.confirm_nothing")}
        </p>
        {#if pending.files.length}
          <ul class="cp-files" dir="ltr">
            {#each pending.files.slice(0, 6) as file (file)}<li><bdi>{file}</bdi></li>{/each}
            {#if pending.total > 6}<li class="cp-more">+{pending.total - 6}</li>{/if}
          </ul>
        {/if}
        <p class="cp-body">{$t("canvas.checkpoint.confirm_note")}</p>
        {#if working}<p class="cp-body" role="status">{$t("canvas.checkpoint.wait_turn")}</p>{/if}
        {#if failed}<p class="cp-error" role="alert">{$t("canvas.checkpoint.failed")}</p>{/if}
        <div class="cp-actions">
          <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => (pending = null)}>{$t("canvas.checkpoint.cancel")}</button>
          <button use:agentActions={[]} type="button" class="btn btn-primary btn-sm" disabled={busy || working || !pending.total} onclick={() => void restore()}>
            {$t("canvas.checkpoint.confirm")}
          </button>
        </div>
      {:else}
        <p class="cp-title">{$t("canvas.checkpoint.title")}</p>
        {#if notice}
          <p class="cp-body" role="status">{notice}</p>
          {#if canUndo}
            <button use:agentActions={[]} type="button" class="btn btn-secondary btn-sm cp-undo" disabled={busy} onclick={() => void undo()}>{$t("canvas.checkpoint.undo")}</button>
          {/if}
        {/if}
        {#if failed}
          <p class="cp-error" role="alert">{$t("canvas.checkpoint.failed")}</p>
        {:else if loading && !listing}
          <p class="cp-body" aria-busy="true">…</p>
        {:else if listing && !listing.repository}
          <p class="cp-body">{$t("canvas.checkpoint.none_repo")}</p>
        {:else if listing && !listing.items.length}
          <p class="cp-body">{$t("canvas.checkpoint.none_yet")}</p>
        {:else if listing}
          <ul class="cp-list">
            {#each listing.items as item (item.turn)}
              <li>
                <span class="cp-when">
                  <span class="cp-turn">{$t("canvas.checkpoint.before_turn", { turn: item.turn })}</span>
                  <span class="cp-at">{clock(item.atMs)}</span>
                </span>
                <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => void choose(item)}>{$t("canvas.checkpoint.restore")}</button>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .cp {
    position: relative;
    display: inline-flex;
  }
  .cp-open {
    all: unset;
    display: inline-grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    cursor: pointer;
  }
  .cp-open:hover,
  .cp-open.on {
    background: var(--surface-hi);
    color: var(--text);
  }
  .cp-open:focus-visible {
    outline: var(--focus-ring);
  }
  .cp-open svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .cp-pop {
    position: fixed;
    z-index: 1000;
    width: 320px;
    max-height: min(360px, calc(100vh - 16px));
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface-hi, var(--surface));
    color: var(--text);
    box-shadow: var(--elev-3), inset 0 0 0 var(--hairline, 1px) var(--content-border, transparent);
    cursor: default;
  }
  .cp-title {
    margin: 0;
    font-size: var(--text-sm);
    font-weight: 600;
  }
  .cp-body,
  .cp-error {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.45;
    color: var(--text-muted);
  }
  .cp-error {
    color: var(--error);
  }
  .cp-list,
  .cp-files {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .cp-list li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .cp-when {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .cp-turn {
    font-size: var(--text-sm);
    font-weight: 500;
  }
  .cp-at,
  .cp-more {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .cp-files li {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cp-undo {
    align-self: flex-start;
  }
  .cp-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
