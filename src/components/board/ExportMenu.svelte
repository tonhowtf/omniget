<script lang="ts">
  import { t } from "$lib/i18n";
  import type { ExportKind, ExportOptions } from "$lib/board/types";
  import BoardIcon from "./BoardIcon.svelte";

  let {
    opts = $bindable(),
    busy,
    empty,
    onexport,
    oncopy,
    onclose,
  }: {
    opts: ExportOptions;
    busy: ExportKind | "copy" | null;
    empty: boolean;
    onexport: (kind: ExportKind) => void;
    oncopy: () => void;
    onclose: () => void;
  } = $props();

  let kind = $state<ExportKind>("png");
  let panel = $state<HTMLDivElement | null>(null);
  const SCALES = [1, 2, 3] as const;

  let imageKind = $derived(kind !== "excalidraw");
  let needsContent = $derived(imageKind && empty);
  let primaryLabel = $derived(
    kind === "png" ? $t("board.export_png") : kind === "svg" ? $t("board.export_svg") : $t("board.export_copy_file"),
  );

  function onkey(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.stopPropagation();
      onclose();
    }
  }

  $effect(() => {
    panel?.querySelector<HTMLElement>("input:checked")?.focus();
  });
</script>

<div class="em" bind:this={panel} role="dialog" aria-modal="false" aria-labelledby="board-export-title" tabindex="-1" onkeydown={onkey}>
  <div class="em-head">
    <h2 id="board-export-title">{$t("board.export_title")}</h2>
    <button type="button" class="btn btn-ghost btn-icon btn-sm" aria-label={$t("board.close")} onclick={onclose}>
      <BoardIcon name="close" size={14} />
    </button>
  </div>

  <fieldset class="em-group">
    <legend>{$t("board.export_format")}</legend>
    <div class="em-seg">
      {#each [["png", "PNG"], ["svg", "SVG"], ["excalidraw", ".excalidraw"]] as [id, text] (id)}
        <label class="em-seg-item" class:on={kind === id}>
          <input type="radio" name="board-export-kind" value={id} bind:group={kind} />
          <span>{text}</span>
        </label>
      {/each}
    </div>
  </fieldset>

  {#if kind === "png"}
    <fieldset class="em-group">
      <legend>{$t("board.export_scale")}</legend>
      <div class="em-seg">
        {#each SCALES as s (s)}
          <label class="em-seg-item" class:on={opts.scale === s}>
            <input type="radio" name="board-export-scale" value={s} bind:group={opts.scale} />
            <span>{s}×</span>
          </label>
        {/each}
      </div>
    </fieldset>
  {/if}

  {#if imageKind}
    <label class="em-check">
      <input type="checkbox" bind:checked={opts.background} />
      <span>{$t("board.export_background")}</span>
    </label>
    <label class="em-check">
      <input type="checkbox" bind:checked={opts.dark} />
      <span>{$t("board.export_dark")}</span>
    </label>
  {:else}
    <p class="em-hint">{$t("board.export_file_hint")}</p>
  {/if}

  {#if needsContent}
    <p class="em-hint em-warn">{$t("board.export_empty")}</p>
  {/if}

  <div class="em-actions">
    {#if kind === "png"}
      <button type="button" class="btn btn-secondary" disabled={busy !== null || needsContent} onclick={oncopy}>
        {#if busy === "copy"}<span class="spinner em-spin" aria-hidden="true"></span>{:else}<BoardIcon name="copy" size={14} />{/if}
        {$t("board.copy_png")}
      </button>
    {/if}
    <button type="button" class="btn btn-primary" disabled={busy !== null || needsContent} onclick={() => onexport(kind)}>
      {#if busy === kind}<span class="spinner em-spin" aria-hidden="true"></span>{:else}<BoardIcon name="export" size={14} />{/if}
      {primaryLabel}
    </button>
  </div>
</div>

<style>
  .em {
    position: absolute;
    top: calc(100% + var(--space-1));
    right: 0;
    z-index: 20;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    width: 300px;
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--popup-bg);
    box-shadow:
      inset 0 0 0 var(--hairline) var(--content-border),
      0 12px 32px color-mix(in srgb, var(--shadow-ink) calc(var(--elev-alpha-3) * 100%), transparent);
  }

  .em-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .em-head h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
    color: var(--text);
  }

  .em-group {
    margin: 0;
    padding: 0;
    border: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .em-group legend {
    padding: 0;
    margin-bottom: var(--space-1);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .em-seg {
    display: flex;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .em-seg-item {
    flex: 1;
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    height: var(--control-h);
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
    color: var(--text-dim);
    cursor: pointer;
  }

  .em-seg-item input {
    position: absolute;
    inset: 0;
    opacity: 0;
    margin: 0;
    cursor: pointer;
  }

  .em-seg-item.on {
    background: var(--pane-bg);
    color: var(--text);
    font-weight: 600;
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .em-seg-item:has(input:focus-visible) {
    outline: 2px solid var(--focus-ring);
    outline-offset: 1px;
  }

  .em-check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
    cursor: pointer;
  }

  .em-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .em-warn {
    color: var(--text);
  }

  .em-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .em-actions .btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .em-spin {
    width: 12px;
    height: 12px;
    border-width: 1.5px;
  }
</style>
