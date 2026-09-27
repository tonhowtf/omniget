<script lang="ts">
  // Filled Phosphor glyph (MIT) drawn inline; colour follows `currentColor`.
  import { ICONS, type IconName } from "$lib/central/threads-ui/icons";

  let { name, size = 16, label = null }: { name: IconName; size?: number; label?: string | null } = $props();
</script>

<svg
  class="ti"
  viewBox="0 0 256 256"
  width={size}
  height={size}
  fill="currentColor"
  role={label ? "img" : undefined}
  aria-label={label ?? undefined}
  aria-hidden={label ? undefined : "true"}
><path d={ICONS[name]} /></svg>

<style>
  .ti { flex-shrink: 0; display: inline-block; vertical-align: middle; }
</style>
