<script lang="ts">
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { IMAGE_EXTENSIONS } from "$lib/palette/api";
  import { normalizeHue, type Oklch } from "$lib/palette/color";
  import { imageSource as src } from "$lib/palette/image.svelte";
  import { newId, uniqueToken } from "$lib/palette/project";
  import { workspace } from "$lib/palette/workspace.svelte";
  import CopyButton from "./CopyButton.svelte";
  import StatusIcon from "$components/converter/StatusIcon.svelte";

  let { dragging = false }: { dragging?: boolean } = $props();

  let colors = $derived(src.result?.colors ?? []);
  let project = $derived(workspace.project!);

  async function choose() {
    const picked = await openDialog({ multiple: false, directory: false, filters: [{ name: $t("palette.image.filter") as string, extensions: IMAGE_EXTENSIONS }] }).catch(() => null);
    if (typeof picked === "string") await src.read(picked);
  }

  function asOklch(c: { l: number; c: number; h: number }): Oklch {
    return { l: c.l, c: c.c, h: normalizeHue(c.h), alpha: 1 };
  }

  function addPalettes() {
    const added = colors.map((c, i) => {
      const name = $t("palette.image.color_name", { num: i + 1, name: src.name }) as string;
      return { id: newId("pal"), name, token: "", base: asOklch(c), hueShift: 0 };
    });
    const list = [...project.palettes];
    for (const [i, pal] of added.entries()) {
      pal.token = uniqueToken(`image-${i + 1}`, list.map((p) => p.token));
      list.push(pal);
    }
    project.palettes = list;
    workspace.touch();
    workspace.showToast($t("palette.image.added", { count: added.length }) as string, "success");
    workspace.tab = "scales";
  }

  function toGradient() {
    const sorted = [...colors].sort((a, b) => b.l - a.l).slice(0, 6);
    project.gradient.stops = sorted.map((c, i) => ({ color: asOklch(c), pos: sorted.length > 1 ? i / (sorted.length - 1) : 0 }));
    workspace.touch();
    workspace.showToast($t("palette.image.gradient_set") as string, "success");
    workspace.tab = "gradients";
  }

  function pick(c: { l: number; c: number; h: number }) {
    workspace.current = asOklch(c);
    workspace.tab = "color";
  }
</script>

<div class="pl-img" aria-busy={src.phase === "reading"}>
  {#if src.phase === "empty"}
    <div class="pl-drop" class:dragging>
      <svg viewBox="0 0 48 48" width="44" height="44" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <rect x="7" y="9" width="34" height="30" rx="3" />
        <circle cx="17" cy="19" r="3.5" />
        <path d="m7 33 10-9 7 6 6-5 11 9" />
      </svg>
      <h3>{$t("palette.image.empty_title")}</h3>
      <p>{$t("palette.image.empty_body")}</p>
      <button type="button" class="btn btn-primary" onclick={choose}>{$t("palette.image.choose")}</button>
    </div>
  {:else}
    <div class="pl-img-top">
      <div class="pl-img-file">
        <strong title={src.path ?? ""}>{src.name}</strong>
        {#if src.result && src.phase !== "error"}
          <span>{$t("palette.image.size", { width: src.result.width, height: src.result.height })}</span>
        {/if}
      </div>
      <label class="pl-count">
        <span class="field-label">{$t("palette.image.count")}</span>
        <input type="range" min="4" max="12" step="1" value={src.count} disabled={src.phase === "reading"} onchange={(e) => void src.setCount(Number(e.currentTarget.value))} />
        <output>{src.count}</output>
      </label>
      <button type="button" class="btn btn-secondary btn-sm" disabled={src.phase === "reading"} onclick={choose}>{$t("palette.image.other")}</button>
      <button type="button" class="btn btn-ghost btn-sm" disabled={src.phase === "reading"} onclick={() => src.clear()}>{$t("palette.image.clear")}</button>
    </div>

    {#if src.phase === "reading"}
      <div class="pl-img-busy" role="status">
        <span class="spinner" aria-hidden="true"></span>
        <p>{$t("palette.image.reading")}</p>
      </div>
    {:else if src.phase === "error"}
      <div class="pl-img-error" role="alert">
        <StatusIcon name="failed" size={18} />
        <div>
          <h3>{$t("palette.image.error_title")}</h3>
          <p>{src.error}</p>
          <p class="pl-dim">{$t("palette.image.error_fix")}</p>
        </div>
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => src.path && src.read(src.path)}>{$t("palette.image.retry")}</button>
      </div>
    {:else if src.result}
      <div class="pl-img-body">
        {#if src.result.preview}
          <img class="pl-img-preview" src={src.result.preview} alt={$t("palette.image.preview_alt", { name: src.name })} />
        {/if}
        <div class="pl-img-colors">
          <div class="pl-share-bar" aria-hidden="true">
            {#each colors as c, i (i)}<span style:flex-grow={Math.max(c.share, 0.005)} style:background={c.hex}></span>{/each}
          </div>
          <ul class="pl-img-list">
            {#each colors as c, i (i)}
              <li>
                <button type="button" class="pl-img-chip" style:background={c.hex} aria-label={$t("palette.image.open_color", { hex: c.hex })} title={$t("palette.image.open_color", { hex: c.hex })} onclick={() => pick(c)}></button>
                <code>{c.hex}</code>
                <span class="pl-share">{$t("palette.image.share", { percent: (c.share * 100).toFixed(1) })}</span>
                <CopyButton text={c.hex} label={$t("palette.copy_value", { value: c.hex })} compact />
              </li>
            {/each}
          </ul>
          <div class="pl-img-actions">
            <button type="button" class="btn btn-primary" onclick={addPalettes}>{$t("palette.image.make_palettes", { count: colors.length })}</button>
            <button type="button" class="btn btn-secondary" onclick={toGradient}>{$t("palette.image.make_gradient")}</button>
          </div>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .pl-img {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .pl-drop {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    min-height: 280px;
    padding: var(--space-7) var(--space-5);
    border: 1.5px dashed var(--content-border);
    border-radius: var(--radius-xl);
    background: var(--surface);
    text-align: center;
    color: var(--text-dim);
    transition: border-color var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
  }

  .pl-drop.dragging {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .pl-drop h3,
  .pl-img-error h3 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .pl-img-error h3 {
    font-size: var(--text-md);
  }

  .pl-drop p {
    margin: 0;
    max-width: 420px;
    font-size: var(--text-base);
  }

  .pl-img-top {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .pl-img-file {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }

  .pl-img-file strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-base);
    color: var(--text);
  }

  .pl-img-file span,
  .pl-dim {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .pl-count {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .pl-count output {
    width: 20px;
    font-variant-numeric: tabular-nums;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .pl-img-busy {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-7);
    border-radius: var(--radius-lg);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .pl-img-busy p {
    margin: 0;
  }

  .pl-img-error {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-img-error > div {
    flex: 1;
    min-width: 0;
  }

  .pl-img-error p {
    margin: var(--space-1) 0 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .pl-img-body {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: var(--space-4);
    align-items: start;
  }

  @media (max-width: 800px) {
    .pl-img-body {
      grid-template-columns: 1fr;
    }
  }

  .pl-img-preview {
    width: 100%;
    max-height: 360px;
    object-fit: contain;
    border-radius: var(--radius-lg);
    background: var(--surface);
  }

  .pl-img-colors {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .pl-share-bar {
    display: flex;
    height: 28px;
    overflow: hidden;
    border-radius: var(--radius-sm);
  }

  .pl-share-bar span {
    flex-basis: 0;
  }

  .pl-img-list {
    margin: 0;
    padding: 0;
    list-style: none;
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-img-list li {
    display: grid;
    grid-template-columns: 24px 80px 1fr auto;
    align-items: center;
    gap: var(--space-2);
    padding: 2px var(--space-1) 2px var(--space-2);
    min-height: var(--row-compact);
  }

  .pl-img-list li + li {
    border-top: var(--hairline) solid var(--separator);
  }

  .pl-img-chip {
    width: 22px;
    height: 22px;
    border: 0;
    border-radius: var(--radius-xs);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    cursor: pointer;
  }

  .pl-img-chip:focus-visible {
    outline: var(--focus-ring);
  }

  .pl-img-list code {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .pl-share {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .pl-img-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  @media (prefers-reduced-motion: reduce) {
    .pl-drop {
      transition: none;
    }
  }
</style>
