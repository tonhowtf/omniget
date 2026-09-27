<script lang="ts">
  // Volume card with a partition bar (used · purgeable · free), after the
  // "Partition Bar" component on 21st.dev (8starlabs).
  import { t } from "$lib/i18n";
  import { formatBytes } from "$lib/washmybelly/api";
  import { usedFraction, volumeSegments, type Volume } from "$lib/washmybelly/disk";

  let { volume, selected = false, onSelect }: { volume: Volume; selected?: boolean; onSelect?: () => void } = $props();

  let segments = $derived(volumeSegments(volume));
  let full = $derived(usedFraction(volume) >= 0.9);
</script>

<button class="vol" class:selected type="button" onclick={() => onSelect?.()} aria-pressed={selected}>
  <div class="head">
    <span class="name">{volume.name}</span>
    {#if volume.removable}<span class="tag">{$t("washmybelly.disk.removable")}</span>{/if}
    {#if volume.read_only}<span class="tag">{$t("washmybelly.disk.read_only")}</span>{/if}
  </div>
  <div class="bar" class:full>
    {#each segments as s (s.key)}
      <span class="seg {s.key}" style:flex-basis="{s.fraction * 100}%"></span>
    {/each}
  </div>
  <div class="legend">
    <span><i class="dot used"></i>{formatBytes(volume.total - volume.free - (volume.purgeable ?? 0))} {$t("washmybelly.disk.used")}</span>
    {#if volume.purgeable}<span><i class="dot purgeable"></i>{formatBytes(volume.purgeable)} {$t("washmybelly.disk.purgeable")}</span>{/if}
    <span><i class="dot free"></i>{formatBytes(volume.free)} {$t("washmybelly.disk.free")}</span>
  </div>
  <div class="sub">{formatBytes(volume.total)} · {volume.file_system}{#if volume.kind !== "unknown"} · {volume.kind.toUpperCase()}{/if}</div>
</button>

<style>
  .vol { all: unset; box-sizing: border-box; display: flex; flex-direction: column; gap: 8px; padding: 14px 16px; min-width: 240px; flex: 1 1 260px; border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator)); cursor: pointer; transition: box-shadow 140ms ease-out; }
  .vol:focus-visible, .vol.selected { box-shadow: inset 0 0 0 2px var(--accent); }
  .head { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .name { font-weight: 600; font-size: 14px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tag { font-size: 10.5px; padding: 1px 6px; border-radius: var(--radius-full, 999px); background: var(--surface-mut, var(--separator)); color: var(--text-dim); }
  .bar { display: flex; gap: 2px; height: 8px; border-radius: var(--radius-full, 999px); overflow: hidden; background: var(--surface-mut, var(--separator)); }
  .seg { flex-grow: 0; flex-shrink: 0; height: 100%; }
  .seg.used, .dot.used { background: var(--accent); }
  .full .seg.used { background: var(--danger); }
  .seg.purgeable, .dot.purgeable { background: color-mix(in srgb, var(--accent) 40%, transparent); }
  .seg.free { background: transparent; }
  .dot.free { background: var(--surface-mut, var(--separator)); box-shadow: inset 0 0 0 1px var(--separator); }
  .legend { display: flex; flex-wrap: wrap; gap: 4px 12px; font-size: 11.5px; color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .legend span { display: inline-flex; align-items: center; gap: 5px; }
  .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; }
  .sub { font-size: 11px; color: var(--text-faint, var(--text-dim)); }
</style>
