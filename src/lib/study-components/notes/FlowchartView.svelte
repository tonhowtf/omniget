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
  let svgHost: HTMLDivElement | null = $state(null);

  type FlowchartInstance = {
    clean: () => void;
    drawSVG: (
      container: HTMLElement | string,
      options?: Record<string, unknown>,
    ) => void;
  };

  type FlowchartApi = {
    parse: (code: string) => FlowchartInstance;
  };

  let cachedFlowchart: FlowchartApi | null = null;
  let lastInstance: FlowchartInstance | null = null;

  async function loadFlowchart(): Promise<FlowchartApi> {
    if (cachedFlowchart) return cachedFlowchart;
    const mod = (await import("flowchart.js")) as {
      default: FlowchartApi;
    } & FlowchartApi;
    const api = (mod.default ?? mod) as FlowchartApi;
    cachedFlowchart = api;
    return api;
  }

  function readThemeOptions(): Record<string, unknown> {
    if (typeof window === "undefined") {
      return {};
    }
    const cs = window.getComputedStyle(document.documentElement);
    const accent = cs.getPropertyValue("--accent").trim() || "#4a90e2";
    const text = cs.getPropertyValue("--text").trim() || "#222";
    const surface = cs.getPropertyValue("--surface").trim() || "#fff";
    return {
      "line-color": accent,
      "font-color": text,
      "element-color": text,
      fill: surface,
      "line-width": 2,
      "font-size": 14,
      "yes-text": "sim",
      "no-text": "não",
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
      const api = await loadFlowchart();
      if (token !== renderToken) return;
      const instance = api.parse(trimmed);
      if (lastInstance) {
        try {
          lastInstance.clean();
        } catch {}
        lastInstance = null;
      }
      if (svgHost) {
        svgHost.innerHTML = "";
      }
      renderState = { kind: "rendered" };
      await Promise.resolve();
      if (token !== renderToken) return;
      if (!svgHost) {
        renderState = { kind: "error", message: "container ausente" };
        return;
      }
      instance.drawSVG(svgHost, readThemeOptions());
      lastInstance = instance;
      const _id = `fc-${blockId}-${token}`;
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
        lastInstance.clean();
      } catch {}
      lastInstance = null;
    }
  });
</script>

<div class="flowchart-block" data-flowchart data-mode={mode}>
  <header class="flowchart-head" contenteditable="false">
    <span class="flowchart-icon" aria-hidden="true">⇄</span>
    <span class="flowchart-label">flowchart</span>
    <button
      type="button"
      class="flowchart-toggle"
      onclick={toggleMode}
      title={mode === "render" ? "Editar source" : "Voltar pro diagrama"}
    >
      {mode === "render" ? "‹/›" : "▶"}
    </button>
  </header>

  {#if mode === "source"}
    <textarea
      class="flowchart-source"
      value={editingValue}
      oninput={onSourceInput}
      onblur={onSourceBlur}
      spellcheck="false"
      rows={Math.max(4, editingValue.split("\n").length)}
      aria-label="Source do diagrama flowchart"
    ></textarea>
  {:else if renderState.kind === "idle"}
    <p class="flowchart-state">Sem source. Clique em ‹/› para editar.</p>
  {:else if renderState.kind === "loading"}
    <p class="flowchart-state">renderizando…</p>
  {:else if renderState.kind === "error"}
    <div class="flowchart-error">
      <p class="flowchart-error-msg">erro: {renderState.message}</p>
      <button type="button" class="flowchart-edit-btn" onclick={toggleMode}
        >Editar source</button>
    </div>
  {/if}

  <div
    class="flowchart-svg"
    bind:this={svgHost}
    style:display={mode === "render" && renderState.kind === "rendered"
      ? "flex"
      : "none"}
  ></div>
</div>

<style>
  .flowchart-block {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    background: color-mix(in oklab, var(--input-border) 12%, transparent);
    border-left: 3px solid var(--accent);
    border-radius: 0 var(--border-radius) var(--border-radius) 0;
    font-size: 12px;
  }
  .flowchart-head {
    display: flex;
    align-items: center;
    gap: 6px;
    user-select: none;
  }
  .flowchart-icon {
    color: var(--accent);
    font-size: 13px;
  }
  .flowchart-label {
    flex: 1 1 auto;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: var(--secondary);
  }
  .flowchart-toggle {
    border: 0;
    background: transparent;
    color: var(--tertiary);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 3px;
  }
  .flowchart-toggle:hover {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--accent);
  }
  .flowchart-source {
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
  .flowchart-source:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
  .flowchart-state {
    margin: 0;
    color: var(--tertiary);
    font-size: 11px;
  }
  .flowchart-svg {
    justify-content: center;
    overflow-x: auto;
    background: var(--surface);
    border-radius: var(--border-radius);
    padding: 8px;
  }
  .flowchart-svg :global(svg) {
    max-width: 100%;
    height: auto;
  }
  .flowchart-error {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px 8px;
    background: color-mix(in oklab, var(--error) 10%, transparent);
    border: 1px solid color-mix(in oklab, var(--error) 30%, transparent);
    border-radius: var(--border-radius);
    color: var(--error);
  }
  .flowchart-error-msg {
    margin: 0;
    font-size: 11px;
    font-family: var(--font-mono, ui-monospace, monospace);
    word-break: break-word;
  }
  .flowchart-edit-btn {
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
  .flowchart-edit-btn:hover {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
  }
</style>
