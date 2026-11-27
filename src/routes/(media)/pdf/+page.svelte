<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { t } from "$lib/i18n";
  import NavIcon from "#components/shell/NavIcon.svelte";
  import OrganizeView from "#components/pdf/OrganizeView.svelte";
  import CompressView from "#components/pdf/CompressView.svelte";
  import ImagesView from "#components/pdf/ImagesView.svelte";

  type Tab = "organize" | "compress" | "images";
  const TABS: Tab[] = ["organize", "compress", "images"];

  const requested = page.url.searchParams.get("tab");
  let tab = $state<Tab>(TABS.includes(requested as Tab) ? (requested as Tab) : "organize");
  let dragging = $state(false);
  let busy = $state({ organize: false, compress: false, images: false });
  let organize = $state<OrganizeView | null>(null);
  let compress = $state<CompressView | null>(null);
  let images = $state<ImagesView | null>(null);

  function route(paths: string[]) {
    const pdfs = paths.filter((p) => /\.pdf$/i.test(p));
    const others = paths.filter((p) => !/\.pdf$/i.test(p));
    if (others.length > 0 && (pdfs.length === 0 || tab === "images")) {
      tab = "images";
      void images?.add(others);
      return;
    }
    if (tab === "images") tab = "organize";
    if (tab === "compress") void compress?.add(pdfs);
    else void organize?.add(pdfs);
  }

  onMount(() => {
    let disposed = false;
    const cleanups: (() => void)[] = [];
    const opened = page.url.searchParams.getAll("open");
    if (opened.length) queueMicrotask(() => route(opened));
    try {
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          const p = event.payload as { type: string; paths?: string[] };
          if (p.type === "enter" || p.type === "over") dragging = true;
          else if (p.type === "leave") dragging = false;
          else if (p.type === "drop") {
            dragging = false;
            if (p.paths?.length) route(p.paths);
          }
        })
        .then((fn) => (disposed ? fn() : cleanups.push(fn)))
        .catch(() => {});
    } catch {
      dragging = false;
    }
    return () => {
      disposed = true;
      cleanups.forEach((fn) => fn());
    };
  });
</script>

<section class="pdf" class:dragging aria-busy={busy[tab]}>
  <header class="pdf-head">
    <NavIcon icon="pdf" size={56} />
    <div class="pdf-head-copy">
      <h1>{$t("pdf.title")}</h1>
      <p class="pdf-subtitle">{$t("pdf.subtitle")}</p>
    </div>
  </header>

  <div class="segmented pdf-tabs" role="tablist" aria-label={$t("pdf.title") as string}>
    {#each TABS as id (id)}
      <button
        type="button"
        role="tab"
        id="pdf-tab-{id}"
        class="segmented-btn"
        class:active={tab === id}
        aria-selected={tab === id}
        aria-controls="pdf-panel-{id}"
        onclick={() => (tab = id)}
      >
        {$t(`pdf.tab_${id}`)}
        {#if busy[id] && tab !== id}<span class="spinner pv-spin" aria-hidden="true"></span>{/if}
      </button>
    {/each}
  </div>

  <div class="pdf-panel" id="pdf-panel-organize" role="tabpanel" aria-labelledby="pdf-tab-organize" hidden={tab !== "organize"}>
    <OrganizeView bind:this={organize} active={tab === "organize"} onbusy={(b) => (busy.organize = b)} />
  </div>
  <div class="pdf-panel" id="pdf-panel-compress" role="tabpanel" aria-labelledby="pdf-tab-compress" hidden={tab !== "compress"}>
    <CompressView bind:this={compress} onbusy={(b) => (busy.compress = b)} />
  </div>
  <div class="pdf-panel" id="pdf-panel-images" role="tabpanel" aria-labelledby="pdf-tab-images" hidden={tab !== "images"}>
    <ImagesView bind:this={images} onbusy={(b) => (busy.images = b)} />
  </div>

  {#if dragging}
    <div class="pdf-dropping" aria-hidden="true">
      <p>{tab === "images" ? $t("pdf.drop_images") : $t("pdf.drop_pdfs")}</p>
    </div>
  {/if}
</section>

<style>
  .pdf {
    --pdf-tone-0: #3b82f6;
    --pdf-tone-1: #f59e0b;
    --pdf-tone-2: #10b981;
    --pdf-tone-3: #ec4899;
    --pdf-tone-4: #8b5cf6;
    --pdf-tone-5: #ef4444;
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    width: 100%;
    max-width: 1180px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .pdf-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .pdf-head-copy {
    min-width: 0;
  }

  .pdf-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .pdf-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .pdf-tabs {
    align-self: flex-start;
  }

  .pdf-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .pdf-panel[hidden] {
    display: none;
  }

  .pdf :global(.pv-spin) {
    width: 12px;
    height: 12px;
  }

  .pdf :global(.pv-drop) {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    min-height: 320px;
    padding: var(--space-7) var(--space-5);
    border: 1.5px dashed var(--content-border);
    border-radius: var(--radius-xl);
    background: var(--surface);
    text-align: center;
    transition: border-color var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
  }

  .pdf.dragging :global(.pv-drop) {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .pdf :global(.pv-drop-art) {
    color: var(--text-dim);
  }

  .pdf :global(.pv-drop-title) {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .pdf :global(.pv-drop-body) {
    margin: 0;
    max-width: 440px;
    font-size: var(--text-base);
    color: var(--text-dim);
  }

  .pdf :global(.pv-drop-actions) {
    display: flex;
    gap: var(--space-2);
  }

  .pdf :global(.pv-files) {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .pdf :global(.pv-file) {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    max-width: 100%;
    padding: var(--space-1) var(--space-1) var(--space-1) var(--space-3);
    border-radius: var(--radius-full, 999px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-size: var(--text-sm);
  }

  .pdf :global(.pv-file.bad) {
    color: var(--text-dim);
  }

  .pdf :global(.pv-tone) {
    flex-shrink: 0;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--tone);
  }

  .pdf :global(.pv-file-name) {
    overflow: hidden;
    max-width: 280px;
    color: var(--text);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pdf :global(.pv-file-meta) {
    color: var(--text-dim);
    font-size: var(--text-xs);
    white-space: nowrap;
  }

  .pdf :global(.pv-file-x) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 22px;
    height: 22px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }

  .pdf :global(.pv-file-x:hover:not(:disabled)) {
    background: color-mix(in srgb, var(--text) 8%, transparent);
    color: var(--text);
  }

  .pdf :global(.pv-file-x:disabled) {
    opacity: 0.4;
    cursor: default;
  }

  .pdf :global(.pv-notice),
  .pdf :global(.pv-result) {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pdf :global(.pv-notice p) {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .pdf :global(.pv-result-copy) {
    flex: 1;
    min-width: 0;
  }

  .pdf :global(.pv-result-copy h2) {
    margin: 0 0 var(--space-1);
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .pdf :global(.pv-result-copy p) {
    margin: 0;
    overflow: hidden;
    font-size: var(--text-sm);
    color: var(--text-dim);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pdf :global(.pv-result-actions) {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .pdf :global(.pv-tools) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-4);
  }

  .pdf :global(.pv-tools-group) {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .pdf :global(.pv-count) {
    min-width: 96px;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }

  .pdf :global(.pv-hint) {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .pdf :global(.pv-block) {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .pdf :global(.pv-block-title) {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .pdf :global(.pv-presets) {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: var(--space-2);
  }

  .pdf :global(.pv-preset) {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3) var(--space-4);
    border: 0;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    text-align: left;
    cursor: pointer;
  }

  .pdf :global(.pv-preset.active) {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }

  .pdf :global(.pv-preset:disabled) {
    cursor: default;
    opacity: 0.7;
  }

  .pdf :global(.pv-preset-name) {
    font-size: var(--text-sm);
    font-weight: 650;
    color: var(--text);
  }

  .pdf :global(.pv-preset-body) {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .pdf :global(.pv-check) {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .pdf :global(.pv-list) {
    margin: 0;
    padding: 0;
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pdf :global(.pv-row) {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
  }

  .pdf :global(.pv-row + .pv-row) {
    border-top: var(--hairline) solid var(--content-border);
  }

  .pdf :global(.pv-row-copy) {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .pdf :global(.pv-row-name) {
    overflow: hidden;
    font-size: var(--text-sm);
    color: var(--text);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pdf :global(.pv-row-meta) {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .pdf :global(.pv-pics) {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--space-3);
    margin: 0;
    padding: var(--space-3);
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pdf :global(.pv-pic) {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .pdf :global(.pv-pic-frame) {
    display: flex;
    align-items: center;
    justify-content: center;
    aspect-ratio: 1;
    border-radius: var(--radius-md);
    background: var(--pane-bg);
    overflow: hidden;
  }

  .pdf :global(.pv-pic-frame img) {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }

  .pdf :global(.pv-pic-name) {
    overflow: hidden;
    font-size: var(--text-xs);
    color: var(--text-dim);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pdf :global(.pv-pic-name b) {
    color: var(--text-muted);
  }

  .pdf :global(.pv-pic-actions) {
    display: flex;
    justify-content: flex-end;
    gap: 2px;
  }

  .pdf :global(.pv-bar) {
    position: sticky;
    bottom: 0;
    z-index: 1;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) 0;
    background: var(--pane-bg);
  }

  .pdf :global(.pv-bar-side),
  .pdf :global(.pv-bar-main) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .pdf :global(.pv-bar-note) {
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .pdf-dropping {
    position: absolute;
    inset: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 2px dashed var(--accent);
    border-radius: var(--radius-xl);
    background: color-mix(in srgb, var(--pane-bg) 82%, transparent);
    pointer-events: none;
  }

  .pdf-dropping p {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  @media (prefers-reduced-motion: reduce) {
    .pdf :global(.pv-drop) {
      transition: none;
    }
  }
</style>
