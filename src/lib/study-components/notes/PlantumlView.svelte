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
    | { kind: "error"; message: string }
    | { kind: "remote-rendering" }
    | { kind: "remote-rendered" };

  let renderState = $state<RenderState>({ kind: "idle" });
  let mode = $state<"render" | "source">("render");
  let renderToken = 0;
  let host: HTMLDivElement | null = $state(null);

  type GraphvizLike = {
    layout: (dot: string, format: string, engine: string) => string;
  };
  type PlantumlLittleModule = {
    convert: (puml: string) => string;
    setup: (opts: { graphviz: GraphvizLike }) => () => void;
    hasGraphvizBridge: () => boolean;
  };

  type Loaded = { convert: (puml: string) => string };

  let loadPromise: Promise<Loaded> | null = null;

  async function loadLib(): Promise<Loaded> {
    if (loadPromise) return loadPromise;
    loadPromise = (async () => {
      const [pumlMod, gvMod] = await Promise.all([
        import("@kookyleo/plantuml-little-web") as unknown as Promise<PlantumlLittleModule>,
        import("@kookyleo/graphviz-anywhere-web") as unknown as Promise<{
          Graphviz: { load: () => Promise<GraphvizLike> };
        }>,
      ]);
      const graphviz = await gvMod.Graphviz.load();
      pumlMod.setup({ graphviz });
      return { convert: (puml: string) => pumlMod.convert(puml) };
    })();
    try {
      return await loadPromise;
    } catch (err) {
      loadPromise = null;
      throw err;
    }
  }

  function readAccentColor(): string {
    if (typeof window === "undefined") return "currentColor";
    const cs = window.getComputedStyle(document.documentElement);
    return cs.getPropertyValue("--text").trim() || "currentColor";
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
      const lib = await loadLib();
      if (token !== renderToken) return;
      const svg = lib.convert(trimmed);
      if (token !== renderToken) return;
      if (!host) {
        renderState = { kind: "error", message: "container ausente" };
        return;
      }
      host.innerHTML = svg;
      const _id = `plantuml-${blockId}-${token}`;
      void _id;
      renderState = { kind: "rendered" };
    } catch (err) {
      if (token !== renderToken) return;
      const message = err instanceof Error ? err.message : String(err);
      renderState = { kind: "error", message };
    }
  }

  function deflateBase64(bytes: Uint8Array): string {
    const PLANT_ALPHABET =
      "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-_";
    let result = "";
    for (let i = 0; i < bytes.length; i += 3) {
      const b1 = bytes[i] ?? 0;
      const b2 = bytes[i + 1] ?? 0;
      const b3 = bytes[i + 2] ?? 0;
      const c1 = b1 >> 2;
      const c2 = ((b1 & 0x3) << 4) | (b2 >> 4);
      const c3 = ((b2 & 0xf) << 2) | (b3 >> 6);
      const c4 = b3 & 0x3f;
      result +=
        PLANT_ALPHABET[c1 & 0x3f] +
        PLANT_ALPHABET[c2 & 0x3f] +
        PLANT_ALPHABET[c3 & 0x3f] +
        PLANT_ALPHABET[c4 & 0x3f];
    }
    return result;
  }

  async function compressForUrl(text: string): Promise<string> {
    const G = globalThis as unknown as {
      CompressionStream?: new (format: string) => GenericTransformStream;
    };
    if (!G.CompressionStream) {
      throw new Error("CompressionStream indisponível neste runtime");
    }
    const ds = new G.CompressionStream("deflate-raw");
    const stream = new Blob([text]).stream().pipeThrough(ds);
    const compressed = await new Response(stream).arrayBuffer();
    return deflateBase64(new Uint8Array(compressed));
  }

  async function renderRemote(src: string) {
    const token = ++renderToken;
    const trimmed = src.trim();
    if (!trimmed) return;
    renderState = { kind: "remote-rendering" };
    try {
      const encoded = await compressForUrl(trimmed);
      if (token !== renderToken) return;
      const url = `https://www.plantuml.com/plantuml/svg/~1${encoded}`;
      const resp = await fetch(url);
      if (!resp.ok) {
        renderState = {
          kind: "error",
          message: `plantuml.com respondeu ${resp.status}`,
        };
        return;
      }
      const svg = await resp.text();
      if (token !== renderToken) return;
      if (!host) {
        renderState = { kind: "error", message: "container ausente" };
        return;
      }
      host.innerHTML = svg;
      renderState = { kind: "remote-rendered" };
    } catch (err) {
      if (token !== renderToken) return;
      const message = err instanceof Error ? err.message : String(err);
      renderState = { kind: "error", message: `fallback: ${message}` };
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

  void readAccentColor;
</script>

<div class="puml-block" data-plantuml data-mode={mode}>
  <header class="puml-head" contenteditable="false">
    <span class="puml-icon" aria-hidden="true">⚙</span>
    <span class="puml-label">plantuml</span>
    {#if renderState.kind === "remote-rendered"}
      <span class="puml-remote-badge" title="renderizado via plantuml.com">via plantuml.com</span>
    {/if}
    <button
      type="button"
      class="puml-toggle"
      onclick={toggleMode}
      title={mode === "render" ? "Editar source" : "Voltar pro diagrama"}
    >
      {mode === "render" ? "‹/›" : "▶"}
    </button>
  </header>

  {#if mode === "source"}
    <textarea
      class="puml-source"
      value={editingValue}
      oninput={onSourceInput}
      onblur={onSourceBlur}
      spellcheck="false"
      rows={Math.max(6, editingValue.split("\n").length)}
      aria-label="Source do diagrama PlantUML"
    ></textarea>
  {:else if renderState.kind === "idle"}
    <p class="puml-state">Sem source. Clique em ‹/› para editar.</p>
  {:else if renderState.kind === "loading"}
    <p class="puml-state">renderizando…</p>
  {:else if renderState.kind === "remote-rendering"}
    <p class="puml-state">enviando a plantuml.com…</p>
  {:else if renderState.kind === "error"}
    <div class="puml-error">
      <p class="puml-error-msg">erro local: {renderState.message}</p>
      <div class="puml-error-actions">
        <button
          type="button"
          class="puml-edit-btn"
          onclick={toggleMode}>Editar source</button>
        <button
          type="button"
          class="puml-remote-btn"
          onclick={() => void renderRemote(source)}
          title="Envia o diagrama pra plantuml.com (terceiro pode logar)"
        >Render via plantuml.com</button>
      </div>
    </div>
  {/if}

  <div
    class="puml-host"
    bind:this={host}
    style:display={mode === "render" &&
    (renderState.kind === "rendered" || renderState.kind === "remote-rendered")
      ? "block"
      : "none"}
  ></div>
</div>

<style>
  .puml-block {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    background: color-mix(in oklab, var(--input-border) 12%, transparent);
    border-left: 3px solid var(--accent);
    border-radius: 0 var(--border-radius) var(--border-radius) 0;
    font-size: 12px;
  }
  .puml-head {
    display: flex;
    align-items: center;
    gap: 6px;
    user-select: none;
  }
  .puml-icon {
    color: var(--accent);
    font-size: 13px;
  }
  .puml-label {
    flex: 1 1 auto;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: var(--secondary);
  }
  .puml-remote-badge {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 3px;
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--accent);
  }
  .puml-toggle {
    border: 0;
    background: transparent;
    color: var(--tertiary);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 3px;
  }
  .puml-toggle:hover {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--accent);
  }
  .puml-source {
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
  .puml-source:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
  .puml-state {
    margin: 0;
    color: var(--tertiary);
    font-size: 11px;
  }
  .puml-host {
    width: 100%;
    background: var(--surface);
    border-radius: var(--border-radius);
    padding: 12px;
    box-sizing: border-box;
    overflow-x: auto;
  }
  .puml-host :global(svg) {
    max-width: 100%;
    height: auto;
  }
  .puml-error {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px 8px;
    background: color-mix(in oklab, var(--error) 10%, transparent);
    border: 1px solid color-mix(in oklab, var(--error) 30%, transparent);
    border-radius: var(--border-radius);
    color: var(--error);
  }
  .puml-error-msg {
    margin: 0;
    font-size: 11px;
    font-family: var(--font-mono, ui-monospace, monospace);
    word-break: break-word;
  }
  .puml-error-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .puml-edit-btn,
  .puml-remote-btn {
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
  .puml-edit-btn:hover,
  .puml-remote-btn:hover {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
  }
  .puml-remote-btn {
    border: 1px dashed color-mix(in oklab, var(--accent) 40%, transparent);
  }
</style>
