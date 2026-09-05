<script lang="ts">
  import { onDestroy } from "svelte";

  type Props = {
    source: string;
    blockId: string;
    onSourceChange?: (next: string) => void;
  };

  let { source, blockId, onSourceChange }: Props = $props();

  type RenderState =
    | { kind: "idle" }
    | { kind: "loading" }
    | { kind: "rendered" }
    | { kind: "error"; message: string };

  let renderState = $state<RenderState>({ kind: "idle" });
  let mode = $state<"render" | "source">("render");
  let renderToken = 0;
  let svgHost: SVGSVGElement | null = $state(null);

  type PureNode = { content: string; children?: PureNode[]; payload?: unknown };

  type TransformResult = { root: PureNode };

  type TransformerCtor = new () => {
    transform: (src: string) => TransformResult;
  };

  type MarkmapInstance = {
    setData: (root: PureNode) => Promise<void>;
    fit: () => Promise<void>;
    destroy: () => void;
  };

  type MarkmapStatic = {
    create: (
      svg: SVGSVGElement,
      opts?: Record<string, unknown>,
      data?: PureNode | null,
    ) => MarkmapInstance;
  };

  let cachedTransformerCtor: TransformerCtor | null = null;
  let cachedMarkmap: MarkmapStatic | null = null;
  let lastInstance: MarkmapInstance | null = null;

  async function loadLibs(): Promise<{
    Transformer: TransformerCtor;
    Markmap: MarkmapStatic;
  }> {
    if (!cachedTransformerCtor) {
      const mod = (await import("markmap-lib")) as {
        Transformer: TransformerCtor;
      };
      cachedTransformerCtor = mod.Transformer;
    }
    if (!cachedMarkmap) {
      const mod = (await import("markmap-view")) as { Markmap: MarkmapStatic };
      cachedMarkmap = mod.Markmap;
    }
    return { Transformer: cachedTransformerCtor, Markmap: cachedMarkmap };
  }

  function readThemeOptions(): Record<string, unknown> {
    if (typeof window === "undefined") return {};
    const cs = window.getComputedStyle(document.documentElement);
    const accent = cs.getPropertyValue("--accent").trim() || "#4a90e2";
    return {
      color: () => accent,
      duration: 200,
      paddingX: 16,
      autoFit: true,
    };
  }

  async function render(src: string) {
    const token = ++renderToken;
    const trimmed = src.trim();
    if (!trimmed) {
      renderState = { kind: "idle" };
      return;
    }
    renderState = { kind: "loading" };
    try {
      const { Transformer, Markmap } = await loadLibs();
      if (token !== renderToken) return;
      const transformer = new Transformer();
      const { root } = transformer.transform(trimmed);
      if (lastInstance) {
        try {
          lastInstance.destroy();
        } catch {}
        lastInstance = null;
      }
      renderState = { kind: "rendered" };
      await Promise.resolve();
      if (token !== renderToken) return;
      if (!svgHost) {
        renderState = { kind: "error", message: "container ausente" };
        return;
      }
      svgHost.innerHTML = "";
      const instance = Markmap.create(svgHost, readThemeOptions(), root);
      lastInstance = instance;
      await instance.fit();
      const _id = `mm-${blockId}-${token}`;
      void _id;
    } catch (err) {
      if (token !== renderToken) return;
      const message = err instanceof Error ? err.message : String(err);
      renderState = { kind: "error", message };
    }
  }

  let editingValue = $state(source);

  $effect(() => {
    editingValue = source;
    if (mode === "render") {
      void render(source);
    }
  });

  function toggleMode() {
    if (mode === "render") {
      editingValue = source;
      mode = "source";
    } else {
      mode = "render";
      onSourceChange?.(editingValue);
      void render(editingValue);
    }
  }

  function onSourceInput(event: Event) {
    const target = event.target as HTMLTextAreaElement;
    editingValue = target.value;
  }

  function onSourceBlur() {
    if (editingValue !== source) {
      onSourceChange?.(editingValue);
    }
  }

  onDestroy(() => {
    renderToken++;
    if (lastInstance) {
      try {
        lastInstance.destroy();
      } catch {}
      lastInstance = null;
    }
  });
</script>

<div class="mindmap-block" data-mindmap data-mode={mode}>
  <header class="mindmap-head" contenteditable="false">
    <span class="mindmap-icon" aria-hidden="true">⌘</span>
    <span class="mindmap-label">mindmap</span>
    <button
      type="button"
      class="mindmap-toggle"
      onclick={toggleMode}
      title={mode === "render" ? "Editar source" : "Voltar pro mapa"}
    >
      {mode === "render" ? "‹/›" : "▶"}
    </button>
  </header>

  {#if mode === "source"}
    <textarea
      class="mindmap-source"
      value={editingValue}
      oninput={onSourceInput}
      onblur={onSourceBlur}
      spellcheck="false"
      rows={Math.max(4, editingValue.split("\n").length)}
      aria-label="Source do mindmap (markdown indentado)"
    ></textarea>
  {:else if renderState.kind === "idle"}
    <p class="mindmap-state">Sem source. Clique em ‹/› para editar.</p>
  {:else if renderState.kind === "loading"}
    <p class="mindmap-state">renderizando…</p>
  {:else if renderState.kind === "error"}
    <div class="mindmap-error">
      <p class="mindmap-error-msg">erro: {renderState.message}</p>
      <button type="button" class="mindmap-edit-btn" onclick={toggleMode}
        >Editar source</button>
    </div>
  {/if}

  <svg
    class="mindmap-svg"
    bind:this={svgHost}
    style:display={mode === "render" && renderState.kind === "rendered"
      ? "block"
      : "none"}
  ></svg>
</div>

<style>
  .mindmap-block {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    background: color-mix(in oklab, var(--input-border) 12%, transparent);
    border-left: 3px solid var(--accent);
    border-radius: 0 var(--border-radius) var(--border-radius) 0;
    font-size: 12px;
  }
  .mindmap-head {
    display: flex;
    align-items: center;
    gap: 6px;
    user-select: none;
  }
  .mindmap-icon {
    color: var(--accent);
    font-size: 13px;
  }
  .mindmap-label {
    flex: 1 1 auto;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: var(--secondary);
  }
  .mindmap-toggle {
    border: 0;
    background: transparent;
    color: var(--tertiary);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 3px;
  }
  .mindmap-toggle:hover {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--accent);
  }
  .mindmap-source {
    width: 100%;
    box-sizing: border-box;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
    line-height: 1.5;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    padding: 8px 10px;
    color: var(--text);
    resize: vertical;
    min-height: 100px;
  }
  .mindmap-source:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
  .mindmap-state {
    margin: 0;
    color: var(--tertiary);
    font-size: 11px;
  }
  .mindmap-svg {
    width: 100%;
    height: 360px;
    background: var(--surface);
    border-radius: var(--border-radius);
    padding: 8px;
    box-sizing: border-box;
  }
  .mindmap-error {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px 8px;
    background: color-mix(in oklab, var(--error) 10%, transparent);
    border: 1px solid color-mix(in oklab, var(--error) 30%, transparent);
    border-radius: var(--border-radius);
    color: var(--error);
  }
  .mindmap-error-msg {
    margin: 0;
    font-size: 11px;
    font-family: var(--font-mono, ui-monospace, monospace);
    word-break: break-word;
  }
  .mindmap-edit-btn {
    align-self: flex-start;
    border: 0;
    background: transparent;
    color: var(--accent);
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 3px;
  }
  .mindmap-edit-btn:hover {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
  }
</style>
