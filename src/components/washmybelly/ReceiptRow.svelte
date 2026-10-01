<script lang="ts">
  import { t, locale } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import type { ReceiptSummary } from "$lib/washmybelly/api";
  import { errorText, explainDetail, formatBytes, formatTime } from "$lib/washmybelly/format";
  import { freesSpace, kindKey, kindModule, splitByStatus } from "$lib/washmybelly/ui";
  import { moduleById } from "./registry";
  import Icon from "./ui/Icon.svelte";

  let {
    receipt,
    expanded = false,
    onToggle,
    onRestore,
    restoreLines = null,
  }: {
    receipt: ReceiptSummary;
    expanded?: boolean;
    onToggle: () => void;
    onRestore: (r: ReceiptSummary) => void;
    restoreLines?: { label: string; ok: boolean; detail: string }[] | null;
  } = $props();

  const DONE_PREVIEW = 40;

  let mod = $derived(moduleById(kindModule(receipt.kind)) ?? moduleById("receipts"));
  let detail = $state<ReceiptSummary | null>(null);
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  let fetched = false;
  let showAllDone = $state(false);
  let items = $derived(detail?.items ?? receipt.items ?? null);
  let split = $derived(splitByStatus(items));
  let restored = $derived(!!receipt.restored_at);
  let frees = $derived(freesSpace(receipt.kind));

  $effect(() => {
    if (!expanded || fetched || receipt.items) return;
    fetched = true;
    loading = true;
    api
      .call<{ receipts: ReceiptSummary[] }>("wmb_receipts", { receipt_id: receipt.receipt_id, limit: 1 })
      .then((r) => {
        const hit = r.receipts?.find((x) => x.receipt_id === receipt.receipt_id);
        detail = hit ?? null;
      })
      .catch((e) => (loadError = errorText(e)))
      .finally(() => (loading = false));
  });

  function kindLabel(kind: string): string {
    const k = kindKey(kind);
    const s = $t(k) as string;
    return s === k ? kind.replace(/_/g, " ") : s;
  }

  function reason(d: string): string {
    const x = explainDetail(d);
    if (!x.key) return x.text;
    const s = $t(x.key) as string;
    return s === x.key ? x.text || d : s;
  }
</script>

<li class="wmb-rr" class:open={expanded}>
  <button class="wmb-rr-head" aria-expanded={expanded} onclick={onToggle}>
    {#if mod}
      <span class="tile" style:--from={mod.from} style:--to={mod.to} aria-hidden="true">
        <span class="glyph" style:--glyph="url(/icons/{mod.glyph}.svg)"></span>
      </span>
    {/if}
    <span class="main">
      <span class="title">{kindLabel(receipt.kind)}</span>
      <span class="sub">
        {formatTime(receipt.created, $locale ?? undefined)}
        · {frees ? $t("washmybelly.plan.freed", { size: formatBytes(receipt.bytes_freed) }) : $t("washmybelly.plan.done_count", { count: receipt.done })}
        {#if receipt.skipped}· {$t("washmybelly.plan.skipped", { count: receipt.skipped })}{/if}
        {#if receipt.failed}· <span class="bad">{$t("washmybelly.plan.failed", { count: receipt.failed })}</span>{/if}
      </span>
    </span>
    {#if restored}
      <span class="tag tag-info">{$t("washmybelly.receipts.restored")}</span>
    {:else if receipt.restorable}
      <span class="tag tag-success">{$t("washmybelly.receipts.can_undo")}</span>
    {/if}
    <span class="chev" class:open={expanded} aria-hidden="true"><Icon name="chevron" size={12} /></span>
  </button>

  {#if expanded}
    <div class="wmb-rr-body">
      {#if restoreLines}
        <div class="notice notice-success">
          <Icon name="undo" size={16} />
          <div class="notice-body">
            <span class="notice-title">{$t("washmybelly.receipts.restore_done")}</span>
            {#each restoreLines as l, i (i)}
              <span class="notice-text line" class:bad={!l.ok}>{l.label}: {l.ok ? l.detail : reason(l.detail)}</span>
            {/each}
          </div>
        </div>
      {/if}

      {#if loading}
        <div class="skel"><span class="skeleton"></span><span class="skeleton" style:width="60%"></span></div>
      {:else if loadError}
        <p class="bad small">{loadError}</p>
      {:else if items}
        <ul class="items">
          {#each [...split.failed, ...split.skipped] as it (it.item_id)}
            <li>
              <span class="st" class:bad={it.status === "failed"}><Icon name={it.status === "failed" ? "error" : "skip"} size={13} /></span>
              <span class="text"><span class="label">{it.label}</span><span class="why">{reason(it.detail)}</span></span>
            </li>
          {/each}
          {#each showAllDone ? split.done : split.done.slice(0, DONE_PREVIEW) as it (it.item_id)}
            <li>
              <span class="st good"><Icon name="check" size={13} /></span>
              <span class="text"><span class="label">{it.label}</span>{#if it.undo}<span class="why">{$t(`washmybelly.receipts.undo_kind.${it.undo}`)}</span>{/if}</span>
              {#if frees && it.bytes}<span class="num">{formatBytes(it.bytes)}</span>{/if}
            </li>
          {/each}
          {#if !showAllDone && split.done.length > DONE_PREVIEW}
            <li class="more"><button class="linkish" onclick={() => (showAllDone = true)}>{$t("washmybelly.plan.more_items", { count: split.done.length - DONE_PREVIEW })}</button></li>
          {/if}
        </ul>
      {:else}
        <p class="small dim">{$t("washmybelly.receipts.no_item_detail", { done: receipt.done, skipped: receipt.skipped, failed: receipt.failed })}</p>
      {/if}

      <div class="actions">
        {#if restored}
          <span class="small dim">{$t("washmybelly.receipts.restored_at", { time: formatTime(receipt.restored_at ?? 0, $locale ?? undefined, true) })}</span>
        {:else if receipt.restorable}
          <span class="small dim">{$t("washmybelly.receipts.restore_hint")}</span>
          <button class="btn btn-secondary btn-sm" onclick={() => onRestore(receipt)}><Icon name="undo" size={13} />{$t("washmybelly.receipts.restore")}</button>
        {:else}
          <span class="small dim">{$t("washmybelly.receipts.not_restorable")}</span>
        {/if}
      </div>
    </div>
  {/if}
</li>

<style>
  .wmb-rr {
    position: relative;
    list-style: none;
  }
  :global(.wmb-rr + .wmb-rr::before) {
    content: "";
    position: absolute;
    top: 0;
    left: calc(var(--space-3) + 28px + var(--space-3));
    right: 0;
    height: var(--hairline);
    background: var(--separator);
  }
  .wmb-rr-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    min-height: 54px;
    padding: var(--space-2) var(--space-3);
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }
  @media (hover: hover) {
    .wmb-rr-head:hover {
      background: var(--fill-1);
    }
  }
  .wmb-rr-head:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }
  .tile {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    border-radius: 7px;
    background: linear-gradient(180deg, var(--from), var(--to));
    box-shadow: inset 0 0 0 0.5px rgb(0 0 0 / 0.12), inset 0 1px 0 rgb(255 255 255 / 0.16);
  }
  .glyph {
    width: 17px;
    height: 17px;
    background: #fff;
    -webkit-mask: var(--glyph) center / contain no-repeat;
    mask: var(--glyph) center / contain no-repeat;
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .title {
    font-size: var(--text-base);
    font-weight: 500;
  }
  .sub {
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chev {
    display: flex;
    color: var(--text-faint);
    transition: transform var(--duration-fast) var(--ease-out);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .wmb-rr-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: 0 var(--space-3) var(--space-3) calc(var(--space-3) + 28px + var(--space-3));
  }
  .items {
    margin: 0;
    padding: 0;
    list-style: none;
    max-height: 300px;
    overflow-y: auto;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .items li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px var(--space-3);
  }
  .items li + li {
    box-shadow: inset 0 var(--hairline) 0 var(--separator);
  }
  .items .more {
    justify-content: center;
  }
  .st {
    display: flex;
    color: var(--text-dim);
  }
  .st.good {
    color: var(--success);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .label {
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .why {
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .num {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .small {
    margin: 0;
    font-size: var(--text-sm);
  }
  .dim {
    color: var(--text-dim);
  }
  .bad {
    color: var(--danger);
  }
  .line {
    display: block;
  }
  .skel {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .linkish {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent-hi);
    font: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
  }
</style>
