<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import JsonNode from "./JsonNode.svelte";

  let {
    name = null,
    value,
    depth = 0,
    last = true,
    oncopy,
  }: {
    name?: string | null;
    value: unknown;
    depth?: number;
    last?: boolean;
    oncopy?: (v: unknown) => void;
  } = $props();

  let isArr = $derived(Array.isArray(value));
  let isObj = $derived(value !== null && typeof value === "object");
  let entries = $derived(
    isObj ? (isArr ? (value as unknown[]).map((v, i) => [String(i), v] as const) : Object.entries(value as object)) : [],
  );
  // eslint-disable-next-line svelte/valid-compile
  let open = $state(untrack(() => depth < 2));

  function scalar(v: unknown): { text: string; cls: string } {
    if (v === null) return { text: "null", cls: "null" };
    if (typeof v === "string") return { text: JSON.stringify(v), cls: "str" };
    if (typeof v === "number") return { text: String(v), cls: "num" };
    if (typeof v === "boolean") return { text: String(v), cls: "bool" };
    return { text: String(v), cls: "" };
  }
</script>

<div class="node" style:--d={depth}>
  {#if isObj}
    <div class="line">
      <button type="button" class="tw" aria-expanded={open} onclick={() => (open = !open)} aria-label={open ? $t("llm.central.catalog.view.collapse") : $t("llm.central.catalog.view.expand")}>
        {open ? "▾" : "▸"}
      </button>
      {#if name !== null}<span class="key">{isNaN(Number(name)) ? JSON.stringify(name) : name}</span><span class="p">: </span>{/if}
      <span class="p">{isArr ? "[" : "{"}</span>
      {#if !open}
        <button type="button" class="more" onclick={() => (open = true)}>{entries.length} {isArr ? $t("llm.central.catalog.json.items") : $t("llm.central.catalog.json.keys")}</button>
        <span class="p">{isArr ? "]" : "}"}{last ? "" : ","}</span>
      {/if}
      {#if oncopy}
        <button type="button" class="cp" onclick={() => oncopy?.(value)}>{$t("llm.central.catalog.copy")}</button>
      {/if}
    </div>
    {#if open}
      {#each entries as [k, v], i (k)}
        <JsonNode name={isArr ? k : k} value={v} depth={depth + 1} last={i === entries.length - 1} {oncopy} />
      {/each}
      <div class="line close"><span class="p">{isArr ? "]" : "}"}{last ? "" : ","}</span></div>
    {/if}
  {:else}
    {@const s = scalar(value)}
    <div class="line leaf">
      {#if name !== null}<span class="key">{isNaN(Number(name)) ? JSON.stringify(name) : name}</span><span class="p">: </span>{/if}
      <span class={s.cls}>{s.text}</span><span class="p">{last ? "" : ","}</span>
    </div>
  {/if}
</div>

<style>
  .node {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    line-height: 1.6;
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: 0;
    padding-left: calc(var(--d) * 16px);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .line.leaf {
    padding-left: calc(var(--d) * 16px + 18px);
  }
  .line.close {
    padding-left: calc(var(--d) * 16px + 18px);
  }
  .tw {
    width: 18px;
    flex-shrink: 0;
    border: none;
    background: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 0;
  }
  .key {
    color: var(--purple);
  }
  .p {
    color: var(--text-muted);
  }
  .str {
    color: var(--success);
  }
  .num {
    color: var(--blue);
  }
  .bool,
  .null {
    color: var(--accent-hi);
  }
  .more {
    margin: 0 4px;
    border: none;
    border-radius: var(--radius-xs);
    background: var(--fill-1);
    color: var(--text-muted);
    font-size: var(--text-xs);
    cursor: pointer;
  }
  .cp {
    margin-left: var(--space-2);
    border: none;
    background: none;
    color: var(--text-faint);
    font-size: var(--text-xs);
    cursor: pointer;
    opacity: 0;
  }
  .line:hover .cp,
  .cp:focus-visible {
    opacity: 1;
  }
</style>
