<script lang="ts">
  /**
   * Grade de pins (masonry por colunas CSS) com seleção. Mostra os sinais
   * que o Pinterest esconde: anúncio, IA, tipo, saves.
   */
  import { t } from "$lib/i18n";
  import { openUrl } from "$lib/tools/rt";
  import { fmtCount, kindKey, looksAi, preview, type Pin } from "$lib/tools/pinterest";

  let {
    pins,
    selected = $bindable(new Set<string>()),
    selectable = true,
    onpick,
  }: { pins: Pin[]; selected?: Set<string>; selectable?: boolean; onpick?: (p: Pin) => void } = $props();

  function toggle(id: string) {
    const s = new Set(selected);
    if (s.has(id)) s.delete(id);
    else s.add(id);
    selected = s;
  }
</script>

<div class="pin-grid">
  {#each pins as p (p.id)}
    <figure class="pin" class:on={selected.has(p.id)}>
      <button class="pin-img" type="button" onclick={() => (selectable ? toggle(p.id) : onpick?.(p))} aria-pressed={selectable ? selected.has(p.id) : undefined} title={p.title}>
        {#if preview(p)}
          <img src={preview(p)} alt={p.title || p.alt_text} loading="lazy" style:background={p.dominant_color ?? "var(--fill-1)"} />
        {:else}
          <div class="pin-blank" style:background={p.dominant_color ?? "var(--fill-1)"}></div>
        {/if}
        <span class="badges">
          {#if p.kind !== "image"}<span class="tag">{$t(kindKey(p))}</span>{/if}
          {#if p.is_promoted}<span class="tag tag-warning">{$t("tools.pinterest.promoted")}</span>{/if}
          {#if looksAi(p)}<span class="tag tag-accent" title={p.ai.labeled ? $t("tools.pinterest.ai_labeled_by_pinterest") : (p.ai.keyword ?? "")}>{$t("tools.pinterest.ai")}</span>{/if}
        </span>
        {#if selectable}<span class="check" aria-hidden="true">{selected.has(p.id) ? "✓" : ""}</span>{/if}
      </button>
      <figcaption>
        <span class="title">{p.title || p.alt_text || p.id}</span>
        <span class="meta">
          {#if p.saves}<span>{fmtCount(p.saves)} {$t("tools.pinterest.saves")}</span>{/if}
          {#if p.domain && p.link}<span class="dim">· {p.domain}</span>{/if}
          <button class="btn btn-ghost btn-sm lnk" type="button" onclick={() => openUrl(p.url)} title={$t("tools.pinterest.open_pinterest")}>↗</button>
        </span>
      </figcaption>
    </figure>
  {/each}
</div>

<style>
  .pin-grid { columns: 4 180px; column-gap: var(--space-3); }
  .pin { break-inside: avoid; margin: 0 0 var(--space-3); border-radius: var(--radius-lg); overflow: hidden; background: var(--surface); box-shadow: inset 0 0 0 var(--hairline) var(--content-border); transition: box-shadow var(--duration-fast) var(--ease-out); }
  .pin.on { box-shadow: 0 0 0 2px var(--accent); }
  .pin-img { position: relative; display: block; width: 100%; padding: 0; border: 0; background: none; cursor: pointer; text-align: left; }
  .pin-img img, .pin-blank { display: block; width: 100%; height: auto; min-height: 60px; }
  .pin-blank { aspect-ratio: 3 / 4; }
  .badges { position: absolute; left: 6px; top: 6px; display: flex; flex-wrap: wrap; gap: 4px; }
  .badges .tag { background: rgba(0, 0, 0, 0.55); color: #fff; backdrop-filter: blur(6px); }
  .badges .tag-warning { background: rgba(245, 158, 11, 0.85); color: #1a1a1a; }
  .badges .tag-accent { background: rgba(147, 51, 234, 0.85); color: #fff; }
  .check { position: absolute; right: 6px; top: 6px; width: 22px; height: 22px; border-radius: 50%; background: rgba(255, 255, 255, 0.85); color: var(--accent); font-weight: 700; font-size: 13px; display: flex; align-items: center; justify-content: center; box-shadow: 0 1px 2px rgba(0, 0, 0, 0.25); }
  .pin.on .check { background: var(--accent); color: #fff; }
  figcaption { display: flex; flex-direction: column; gap: 2px; padding: 6px 8px 8px; }
  .title { font-size: var(--text-sm); line-height: 1.3; font-weight: 500; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .meta { display: flex; align-items: center; gap: 4px; font-size: var(--text-xs); color: var(--text-dim); min-width: 0; }
  .meta .dim { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .lnk { margin-left: auto; padding-inline: 4px; }
</style>
