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
  let host: HTMLDivElement | null = $state(null);

  type AbcjsModule = {
    renderAbc: (
      element: HTMLElement | string,
      src: string,
      options?: Record<string, unknown>,
    ) => unknown;
  };

  let cachedAbcjs: AbcjsModule | null = null;

  async function loadLib(): Promise<AbcjsModule> {
    if (cachedAbcjs) return cachedAbcjs;
    const mod = (await import("abcjs")) as { default?: AbcjsModule } & AbcjsModule;
    const resolved = (mod.default ?? mod) as AbcjsModule;
    cachedAbcjs = resolved;
    return resolved;
  }

  function readThemeOptions(): Record<string, unknown> {
    if (typeof window === "undefined") return {};
    const cs = window.getComputedStyle(document.documentElement);
    const fg = cs.getPropertyValue("--text").trim() || "#222";
    return {
      responsive: "resize",
      staffwidth: 740,
      foregroundColor: fg,
      wrap: {
        minSpacing: 1.8,
        maxSpacing: 2.7,
        preferredMeasuresPerLine: 4,
      },
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
      const abcjs = await loadLib();
      if (token !== renderToken) return;
      renderState = { kind: "rendered" };
      await Promise.resolve();
      if (token !== renderToken) return;
      if (!host) {
        renderState = { kind: "error", message: "container ausente" };
        return;
      }
      host.innerHTML = "";
      abcjs.renderAbc(host, trimmed, readThemeOptions());
      const _id = `abc-${blockId}-${token}`;
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
    if (host) {
      try {
        host.innerHTML = "";
      } catch {}
    }
  });
</script>

<div class="abc-block" data-abc data-mode={mode}>
  <header class="abc-head" contenteditable="false">
    <span class="abc-icon" aria-hidden="true">♪</span>
    <span class="abc-label">abc</span>
    <button
      type="button"
      class="abc-toggle"
      onclick={toggleMode}
      title={mode === "render" ? "Editar source" : "Voltar pra partitura"}
    >
      {mode === "render" ? "‹/›" : "▶"}
    </button>
  </header>

  {#if mode === "source"}
    <textarea
      class="abc-source"
      value={editingValue}
      oninput={onSourceInput}
      onblur={onSourceBlur}
      spellcheck="false"
      rows={Math.max(6, editingValue.split("\n").length)}
      aria-label="Source da partitura (notação ABC)"
    ></textarea>
  {:else if renderState.kind === "idle"}
    <p class="abc-state">Sem source. Clique em ‹/› para editar.</p>
  {:else if renderState.kind === "loading"}
    <p class="abc-state">renderizando…</p>
  {:else if renderState.kind === "error"}
    <div class="abc-error">
      <p class="abc-error-msg">erro: {renderState.message}</p>
      <button type="button" class="abc-edit-btn" onclick={toggleMode}
        >Editar source</button>
    </div>
  {/if}

  <div
    class="abc-host"
    bind:this={host}
    style:display={mode === "render" && renderState.kind === "rendered"
      ? "block"
      : "none"}
  ></div>
</div>

<style>
  .abc-block {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    background: color-mix(in oklab, var(--input-border) 12%, transparent);
    border-left: 3px solid var(--accent);
    border-radius: 0 var(--border-radius) var(--border-radius) 0;
    font-size: 12px;
  }
  .abc-head {
    display: flex;
    align-items: center;
    gap: 6px;
    user-select: none;
  }
  .abc-icon {
    color: var(--accent);
    font-size: 13px;
  }
  .abc-label {
    flex: 1 1 auto;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: var(--secondary);
  }
  .abc-toggle {
    border: 0;
    background: transparent;
    color: var(--tertiary);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 3px;
  }
  .abc-toggle:hover {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--accent);
  }
  .abc-source {
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
    min-height: 120px;
  }
  .abc-source:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
  .abc-state {
    margin: 0;
    color: var(--tertiary);
    font-size: 11px;
  }
  .abc-host {
    width: 100%;
    background: var(--surface);
    border-radius: var(--border-radius);
    padding: 12px;
    box-sizing: border-box;
    overflow-x: auto;
  }
  .abc-host :global(svg) {
    max-width: 100%;
    height: auto;
  }
  .abc-error {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px 8px;
    background: color-mix(in oklab, var(--error) 10%, transparent);
    border: 1px solid color-mix(in oklab, var(--error) 30%, transparent);
    border-radius: var(--border-radius);
    color: var(--error);
  }
  .abc-error-msg {
    margin: 0;
    font-size: 11px;
    font-family: var(--font-mono, ui-monospace, monospace);
    word-break: break-word;
  }
  .abc-edit-btn {
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
  .abc-edit-btn:hover {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
  }
</style>
