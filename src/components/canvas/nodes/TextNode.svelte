<script lang="ts">
  import { canvasPrefs } from "$lib/canvas/prefs.svelte";
  import { t } from "$lib/i18n";
  import type { CanvasNode } from "$lib/canvas/model/types";

  let {
    node,
    focused = false,
    onpatch,
    onexit,
  }: {
    node: CanvasNode;
    focused?: boolean;
    onpatch?: (patch: Partial<CanvasNode>) => void;
    onexit?: () => void;
  } = $props();

  let draft = $state("");
  let area = $state<HTMLTextAreaElement | null>(null);
  const text = $derived(String(node.data.text ?? ""));
  const size = $derived(Math.min(200, Math.max(8, Number(node.data.fontSize ?? 20) || 20)));
  const WEIGHTS: Record<string, number> = { regular: 400, medium: 500, bold: 700 };
  const weight = $derived(WEIGHTS[String(node.data.fontWeight ?? "regular")] ?? 400);
  const FAMILIES: Record<string, string> = {
    sans: "var(--font-sans, system-ui, sans-serif)",
    serif: "var(--font-serif, ui-serif, Georgia, 'Times New Roman', serif)",
    mono: "var(--font-mono, ui-monospace, Menlo, monospace)",
  };
  const family = $derived(FAMILIES[String(node.data.fontFamily ?? "sans")] ?? FAMILIES.sans);
  const color = $derived.by(() => {
    const c = node.data.color ?? node.color;
    return typeof c === "string" && /^#[0-9a-f]{6}$/i.test(c) ? c : null;
  });

  $effect(() => {
    if (focused) {
      draft = text;
      queueMicrotask(() => {
        area?.focus();
        area?.select();
      });
    }
  });

  let box = $state<HTMLDivElement | null>(null);
  $effect(() => {
    void size;
    void weight;
    void family;
    void text;
    void focused;
    const el = box;
    if (!el || focused) return;
    const raf = requestAnimationFrame(() => {
      const body = el.querySelector<HTMLElement>(".body");
      if (!body) return;
      const needW = Math.ceil(body.scrollWidth);
      const needH = Math.ceil(body.scrollHeight);
      const growW = needW > el.clientWidth + 1;
      const growH = needH > el.clientHeight + 1;
      if (!growW && !growH) return;
      onpatch?.({
        ...(growW ? { w: Math.max(node.w, needW + 2) } : {}),
        ...(growH ? { h: Math.max(node.h, needH + 2) } : {}),
      });
    });
    return () => cancelAnimationFrame(raf);
  });

  function commit() {
    if (draft !== text) onpatch?.({ data: { ...node.data, text: draft } });
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape" || (e.key === "Enter" && (e.metaKey || e.ctrlKey))) {
      e.preventDefault();
      commit();
      onexit?.();
    }
  }
</script>

<div class="text" bind:this={box} style:font-size="{size}px" style:font-weight={weight} style:font-family={family} style:color={color}>
  {#if focused}
    <textarea bind:this={area} bind:value={draft} onblur={commit} onkeydown={onkey} spellcheck={canvasPrefs().spell_check}></textarea>
  {:else if text}
    <div class="body">{text}</div>
  {:else}
    <div class="body empty">{$t("canvas.engine.textPlaceholder", { default: "Text" })}</div>
  {/if}
</div>

<style>
  .text {
    height: 100%;
    overflow: hidden;
    color: var(--text);
    line-height: 1.3;
  }
  .body {
    white-space: pre-wrap;
    overflow-wrap: normal;
  }
  .empty {
    color: var(--text-muted);
  }
  textarea {
    width: 100%;
    height: 100%;
    resize: none;
    border: none;
    outline: none;
    background: transparent;
    color: inherit;
    font: inherit;
    line-height: inherit;
    padding: 0;
  }
</style>
