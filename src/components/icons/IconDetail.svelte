<script lang="ts" module>
  export type DetailOpts = {
    useColor: boolean;
    color: string;
    size: number;
    strokeWidth: number | null;
    rotate: number;
    hFlip: boolean;
    vFlip: boolean;
  };

  export type CopyKind = "svg" | "svelte" | "react" | "css" | "id" | "install";
</script>

<script lang="ts">
  import type { SvelteMap } from "svelte/reactivity";
  import { t } from "$lib/i18n";
  import { toReact, toSvelte } from "$lib/icons/convert";
  import { DEFAULT_SIZE, MAX_SIZE, MIN_SIZE, buildSvg, clampSize, cssBackground, dimensions, hasStroke, parts, type BuildOptions } from "$lib/icons/svg";
  import { iconifyId, installCommand, isStrokeSet, npmPackage } from "$lib/icons/sets";
  import type { IconData, IconSet, NamedCollection } from "$lib/icons/types";
  import IconView from "./IconView.svelte";
  import IconGrid from "./IconGrid.svelte";
  import LicenseBadge from "./LicenseBadge.svelte";
  import Glyph from "./Glyph.svelte";

  let {
    prefix,
    name,
    icon,
    set,
    favorite,
    collections,
    related,
    relatedState,
    bodies,
    favorites,
    opts = $bindable(),
    busy = null,
    isPalette,
    onclose,
    onfavorite,
    onaddto,
    oncreate,
    oncopy,
    onsave,
    onopen,
    onneed,
  }: {
    prefix: string;
    name: string;
    icon: IconData | null | undefined;
    set: IconSet | null;
    favorite: boolean;
    collections: NamedCollection[];
    related: string[];
    relatedState: "loading" | "ready" | "offline";
    bodies: SvelteMap<string, IconData | null>;
    favorites: Set<string>;
    opts: DetailOpts;
    busy?: "svg" | "png" | null;
    isPalette: (prefix: string) => boolean;
    onclose: () => void;
    onfavorite: () => void;
    onaddto: (collectionId: string) => void;
    oncreate: (name: string) => void;
    oncopy: (kind: CopyKind, text: string) => void;
    onsave: (kind: "svg" | "png", svg: string, size: { width: number; height: number }) => void;
    onopen: (id: string) => void;
    onneed: (ids: string[]) => void;
  } = $props();

  const PREVIEW_SIZES = [16, 20, 24, 32, 48];
  const PNG_SIZES = [16, 24, 32, 48, 64, 128, 256, 512, 1024];

  let palette = $derived(isPalette(prefix));
  let canStroke = $derived(Boolean(icon && hasStroke(icon.body) && (isStrokeSet(prefix) || !palette)));
  let defaultStroke = $derived.by(() => {
    const m = icon?.body.match(/stroke-width\s*=\s*["']([\d.]+)["']/);
    return m ? Number(m[1]) : 2;
  });
  let size = $derived(clampSize(opts.size));
  let heading = $state<HTMLElement | null>(null);

  export function focus() {
    heading?.focus();
  }

  let build = $derived<BuildOptions>({
    color: opts.useColor ? opts.color : null,
    strokeWidth: canStroke ? opts.strokeWidth : null,
    rotate: opts.rotate,
    hFlip: opts.hFlip,
    vFlip: opts.vFlip,
    size,
  });
  let viewOpts = $derived<BuildOptions>({ ...build, color: palette ? build.color : null });
  let svg = $derived(icon ? buildSvg(icon, build) : "");
  let id = $derived(iconifyId(prefix, name));
  let install = $derived(installCommand(prefix));
  let pkg = $derived(npmPackage(prefix));
  let pngSize = $state(256);
  let pickAdd = $state("");
  let collectionName = $state("");
  let relatedActive = $state(0);
  let swatchStyle = $derived(opts.useColor ? opts.color : null);

  function text(kind: CopyKind): string {
    if (!icon) return "";
    switch (kind) {
      case "svg":
        return svg;
      case "svelte":
        return toSvelte(svg);
      case "react":
        return toReact(svg, name);
      case "css":
        return cssBackground(icon, build);
      case "id":
        return id;
      case "install":
        return install ?? "";
    }
  }

  function save(kind: "svg" | "png") {
    if (!icon) return;
    if (kind === "svg") {
      onsave("svg", svg, dimensions(parts(icon, build), size));
      return;
    }
    const box = dimensions(parts(icon, build), pngSize);
    onsave("png", buildSvg(icon, { ...build, size: pngSize, color: build.color ?? "#000" }), box);
  }

  function resetAdjust() {
    opts.useColor = false;
    opts.size = DEFAULT_SIZE;
    opts.strokeWidth = null;
    opts.rotate = 0;
    opts.hFlip = false;
    opts.vFlip = false;
  }

  let creating = $state(false);

  function addTo(value: string) {
    if (!value) return;
    if (value === "new") creating = true;
    else onaddto(value);
    pickAdd = "";
  }

  function create(event: SubmitEvent) {
    event.preventDefault();
    const trimmed = collectionName.trim();
    if (!trimmed) return;
    oncreate(trimmed);
    collectionName = "";
    creating = false;
  }

  let adjusted = $derived(opts.useColor || size !== DEFAULT_SIZE || opts.strokeWidth !== null || opts.rotate !== 0 || opts.hFlip || opts.vFlip);
</script>

<aside class="ic-detail" aria-labelledby="ic-detail-title">
  <header class="ic-detail-head">
    <div class="ic-detail-hero" style:color={swatchStyle}>
      {#if icon}
        <IconView {icon} size={64} {palette} opts={viewOpts} />
      {:else if icon === null}
        <Glyph name="warning" size={24} />
      {:else}
        <span class="spinner" aria-hidden="true"></span>
      {/if}
    </div>
    <div class="ic-detail-copy">
      <h2 id="ic-detail-title" tabindex="-1" bind:this={heading}>{name}</h2>
      <p class="ic-detail-set">
        {set?.name ?? prefix}
        <LicenseBadge {set} />
      </p>
      {#if set?.author}
        <p class="ic-detail-author">{$t("icons.by_author", { author: set.author })}</p>
      {/if}
    </div>
    <button type="button" class="btn btn-icon btn-ghost btn-sm ic-detail-close" aria-label={$t("icons.close_detail") as string} title={$t("icons.close_detail_hint") as string} onclick={onclose}>
      <Glyph name="close" size={14} />
    </button>
  </header>

  {#if creating}
    <form class="ic-create" onsubmit={create}>
      <label class="ic-sr" for="ic-new-name">{$t("icons.collection_name")}</label>
      <!-- svelte-ignore a11y_autofocus -->
      <input id="ic-new-name" class="ic-select ic-create-input" bind:value={collectionName} maxlength="80" placeholder={$t("icons.collection_name_placeholder") as string} autofocus />
      <button type="submit" class="btn btn-secondary btn-sm" disabled={!collectionName.trim()} title={!collectionName.trim() ? ($t("icons.collection_name_needed") as string) : undefined}>{$t("icons.create_and_add")}</button>
      <button type="button" class="btn btn-ghost btn-sm" onclick={() => (creating = false)}>{$t("icons.cancel")}</button>
    </form>
  {/if}

  {#if icon === null}
    <p class="ic-detail-error" role="alert">{$t("icons.icon_unavailable")}</p>
  {/if}

  <div class="ic-detail-actions">
    <button type="button" class="btn btn-secondary btn-sm" aria-pressed={favorite} disabled={!icon} onclick={onfavorite}>
      <Glyph name={favorite ? "star-fill" : "star"} size={13} />
      {favorite ? $t("icons.unfavorite") : $t("icons.favorite")}
    </button>
    <label class="ic-sr" for="ic-add-to">{$t("icons.add_to_collection")}</label>
    <select id="ic-add-to" class="ic-select" bind:value={pickAdd} disabled={!icon} onchange={() => addTo(pickAdd)}>
      <option value="">{$t("icons.add_to_collection")}</option>
      {#each collections as c (c.id)}
        {@const has = c.icons.some((s) => s.prefix === prefix && s.icon.name === name)}
        <option value={c.id} disabled={has}>{has ? $t("icons.already_in", { name: c.name }) : c.name}</option>
      {/each}
      <option value="new">{$t("icons.new_collection_option")}</option>
    </select>
  </div>

  {#if icon}
    <section class="ic-sec" aria-labelledby="ic-sec-preview">
      <h3 id="ic-sec-preview">{$t("icons.preview")}</h3>
      <div class="ic-swatches">
        <div class="ic-swatch ic-swatch-light" style:color={swatchStyle}>
          <span class="ic-sr">{$t("icons.on_light")}</span>
          {#each PREVIEW_SIZES as s (s)}
            <span class="ic-swatch-cell" title={$t("icons.size_px", { size: s }) as string}><IconView {icon} size={s} {palette} opts={viewOpts} /></span>
          {/each}
        </div>
        <div class="ic-swatch ic-swatch-dark" style:color={swatchStyle}>
          <span class="ic-sr">{$t("icons.on_dark")}</span>
          {#each PREVIEW_SIZES as s (s)}
            <span class="ic-swatch-cell" title={$t("icons.size_px", { size: s }) as string}><IconView {icon} size={s} {palette} opts={viewOpts} /></span>
          {/each}
        </div>
        <div class="ic-swatch-sizes" aria-hidden="true">
          {#each PREVIEW_SIZES as s (s)}<span>{s}</span>{/each}
        </div>
      </div>
    </section>

    <section class="ic-sec" aria-labelledby="ic-sec-adjust">
      <div class="ic-sec-head">
        <h3 id="ic-sec-adjust">{$t("icons.adjust")}</h3>
        <button type="button" class="btn btn-ghost btn-sm" disabled={!adjusted} onclick={resetAdjust}>{$t("icons.reset")}</button>
      </div>
      <div class="ic-fields">
        <div class="ic-field">
          <span class="ic-field-label" id="ic-color-label">{$t("icons.color")}</span>
          <div class="ic-seg" role="radiogroup" aria-labelledby="ic-color-label">
            <button type="button" role="radio" aria-checked={!opts.useColor} class:on={!opts.useColor} onclick={() => (opts.useColor = false)}>{$t("icons.color_current")}</button>
            <button type="button" role="radio" aria-checked={opts.useColor} class:on={opts.useColor} onclick={() => (opts.useColor = true)}>{$t("icons.color_custom")}</button>
          </div>
          {#if opts.useColor}
            <input class="ic-color" type="color" bind:value={opts.color} aria-label={$t("icons.color_pick") as string} />
            <code class="ic-color-code">{opts.color}</code>
          {/if}
        </div>
        {#if palette}
          <p class="ic-field-hint">{$t("icons.palette_hint")}</p>
        {/if}

        <label class="ic-field">
          <span class="ic-field-label">{$t("icons.size")}</span>
          <input class="ic-num" type="number" min={MIN_SIZE} max={MAX_SIZE} step="1" bind:value={opts.size} onchange={() => (opts.size = size)} onblur={() => (opts.size = size)} />
          <span class="ic-unit">{$t("icons.unit_px")}</span>
        </label>

        {#if canStroke}
          <label class="ic-field">
            <span class="ic-field-label">{$t("icons.stroke")}</span>
            <input type="range" min="0.5" max="3" step="0.25" value={opts.strokeWidth ?? defaultStroke} oninput={(e) => (opts.strokeWidth = Number((e.currentTarget as HTMLInputElement).value))} />
            <span class="ic-unit">{(opts.strokeWidth ?? defaultStroke).toFixed(2)}</span>
          </label>
        {:else}
          <p class="ic-field-hint">{$t("icons.stroke_unavailable")}</p>
        {/if}

        <div class="ic-field">
          <span class="ic-field-label">{$t("icons.transform")}</span>
          <div class="ic-tools">
            <button type="button" class="btn btn-icon btn-sm btn-secondary" aria-label={$t("icons.rotate") as string} title={$t("icons.rotate_state", { deg: opts.rotate * 90 }) as string} onclick={() => (opts.rotate = (opts.rotate + 1) % 4)}>
              <Glyph name="rotate" size={14} />
            </button>
            <button type="button" class="btn btn-icon btn-sm btn-secondary" aria-label={$t("icons.flip_h") as string} aria-pressed={opts.hFlip} onclick={() => (opts.hFlip = !opts.hFlip)}>
              <Glyph name="flip-h" size={14} />
            </button>
            <button type="button" class="btn btn-icon btn-sm btn-secondary" aria-label={$t("icons.flip_v") as string} aria-pressed={opts.vFlip} onclick={() => (opts.vFlip = !opts.vFlip)}>
              <Glyph name="flip-v" size={14} />
            </button>
            <span class="ic-unit">{opts.rotate * 90}°</span>
          </div>
        </div>
      </div>
    </section>

    <section class="ic-sec" aria-labelledby="ic-sec-copy">
      <h3 id="ic-sec-copy">{$t("icons.copy_as")}</h3>
      <div class="ic-copy">
        <button type="button" class="btn btn-primary btn-sm" onclick={() => oncopy("svg", text("svg"))}><Glyph name="copy" size={13} />{$t("icons.copy_svg")}</button>
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => oncopy("svelte", text("svelte"))}>{$t("icons.copy_svelte")}</button>
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => oncopy("react", text("react"))}>{$t("icons.copy_react")}</button>
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => oncopy("css", text("css"))}>{$t("icons.copy_css")}</button>
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => oncopy("id", text("id"))}><code>{id}</code></button>
      </div>
      {#if install && pkg}
        <div class="ic-install">
          <code>{install}</code>
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => oncopy("install", text("install"))}>{$t("icons.copy_install")}</button>
        </div>
      {:else}
        <p class="ic-field-hint">{$t("icons.no_package")}</p>
      {/if}
    </section>

    <section class="ic-sec" aria-labelledby="ic-sec-save">
      <h3 id="ic-sec-save">{$t("icons.save")}</h3>
      <div class="ic-copy">
        <button type="button" class="btn btn-secondary btn-sm" disabled={busy !== null} onclick={() => save("svg")}>
          {#if busy === "svg"}<span class="spinner ic-spin" aria-hidden="true"></span>{:else}<Glyph name="download" size={13} />{/if}
          {$t("icons.save_svg")}
        </button>
        <span class="ic-png">
          <button type="button" class="btn btn-secondary btn-sm" disabled={busy !== null} onclick={() => save("png")}>
            {#if busy === "png"}<span class="spinner ic-spin" aria-hidden="true"></span>{:else}<Glyph name="download" size={13} />{/if}
            {$t("icons.save_png")}
          </button>
          <label class="ic-sr" for="ic-png-size">{$t("icons.png_size")}</label>
          <select id="ic-png-size" class="ic-select" bind:value={pngSize}>
            {#each PNG_SIZES as s (s)}<option value={s}>{$t("icons.size_px", { size: s })}</option>{/each}
          </select>
        </span>
      </div>
    </section>
  {/if}

  <section class="ic-sec" aria-labelledby="ic-sec-related">
    <h3 id="ic-sec-related">{$t("icons.related", { set: set?.name ?? prefix })}</h3>
    {#if relatedState === "loading"}
      <p class="ic-field-hint"><span class="spinner ic-spin" aria-hidden="true"></span> {$t("icons.related_loading")}</p>
    {:else if relatedState === "offline"}
      <p class="ic-field-hint"><Glyph name="offline" size={12} /> {$t("icons.related_offline")}</p>
    {:else if related.length === 0}
      <p class="ic-field-hint">{$t("icons.related_none")}</p>
    {:else}
      <IconGrid
        ids={related}
        {bodies}
        {favorites}
        bind:active={relatedActive}
        label={$t("icons.related", { set: set?.name ?? prefix }) as string}
        height={150}
        cell={44}
        compact
        {isPalette}
        onopen={(rid) => onopen(rid)}
        {onneed}
      />
    {/if}
  </section>

</aside>

<style>
  .ic-detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ic-detail-head {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
  }

  .ic-detail-hero {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 88px;
    height: 88px;
    flex-shrink: 0;
    border-radius: var(--radius-lg);
    background: var(--fill-1);
    color: var(--text);
  }

  .ic-detail-copy {
    flex: 1;
    min-width: 0;
  }

  .ic-detail-copy h2 {
    margin: 0 0 var(--space-1);
    overflow-wrap: anywhere;
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--text-md);
    font-weight: 600;
  }

  .ic-detail-set {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  .ic-detail-author {
    margin: var(--space-1) 0 0;
    color: var(--text-dim);
    font-size: var(--text-xs);
  }

  .ic-detail-error {
    margin: 0;
    color: var(--text);
    font-size: var(--text-sm);
  }

  .ic-detail-actions,
  .ic-copy {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .ic-detail-actions .btn,
  .ic-copy .btn {
    gap: var(--space-1);
  }

  .ic-select,
  .ic-num {
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
  }

  .ic-num {
    width: 72px;
    font-variant-numeric: tabular-nums;
  }

  .ic-select:focus-visible,
  .ic-num:focus-visible,
  .ic-color:focus-visible,
  .ic-seg button:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .ic-sec {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-top: var(--space-3);
    border-top: var(--hairline) solid var(--separator);
  }

  .ic-sec h3 {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
  }

  .ic-sec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .ic-swatches {
    display: grid;
    gap: var(--space-1);
  }

  .ic-swatch,
  .ic-swatch-sizes {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    align-items: center;
    justify-items: center;
  }

  .ic-swatch {
    min-height: 64px;
    border-radius: var(--radius-md);
  }

  .ic-swatch-light {
    background: color-mix(in srgb, var(--on-status) 94%, var(--tertiary));
    color: color-mix(in srgb, rgb(var(--shadow-ink)) 88%, var(--tertiary));
  }

  .ic-swatch-dark {
    background: color-mix(in srgb, rgb(var(--shadow-ink)) 86%, var(--tertiary));
    color: color-mix(in srgb, var(--on-status) 94%, var(--tertiary));
  }

  .ic-swatch-sizes span {
    color: var(--text-dim);
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
  }

  .ic-swatch-cell {
    display: inline-flex;
  }

  .ic-fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .ic-field {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .ic-field-label {
    width: 72px;
    color: var(--text-muted);
  }

  .ic-field input[type="range"] {
    flex: 1;
    min-width: 100px;
    accent-color: var(--accent);
  }

  .ic-field-hint {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-xs);
  }

  .ic-unit {
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }

  .ic-seg {
    display: inline-flex;
    padding: 2px;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
  }

  .ic-seg button {
    height: calc(var(--control-h) - 4px);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-xs);
    background: transparent;
    color: var(--text-muted);
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 550;
    cursor: pointer;
  }

  .ic-seg button.on {
    background: var(--surface-hi);
    color: var(--text);
    box-shadow: var(--elev-1);
  }

  .ic-color {
    width: 32px;
    height: var(--control-h);
    padding: 0;
    border: none;
    background: transparent;
    cursor: pointer;
  }

  .ic-color-code {
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  .ic-tools {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .ic-copy code {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  .ic-install {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-1) var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill-1);
  }

  .ic-install code {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ic-create {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .ic-create-input {
    flex: 1;
    min-width: 140px;
  }

  .ic-png {
    display: inline-flex;
    gap: var(--space-1);
  }

  .ic-spin {
    width: 12px;
    height: 12px;
  }

  .ic-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
