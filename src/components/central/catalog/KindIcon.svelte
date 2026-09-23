<script lang="ts">
  /** Chunky filled tile for a component kind (same tile as the Tools section). */
  import ToolIcon from "$components/tools/ToolIcon.svelte";
  import { kindStyle } from "$lib/central/catalog";

  let { kind, size = 36 }: { kind: string; size?: number } = $props();
  let s = $derived(kindStyle(kind));
</script>

<ToolIcon icon={`glyph:${s.glyph}`} from={s.from} to={s.to} {size} />
