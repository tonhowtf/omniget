<script lang="ts">
  import { t } from "$lib/i18n";
  import type { CanvasNode } from "$lib/canvas/model/types";

  let { node }: { node: CanvasNode } = $props();
</script>

<div class="unknown">
  <div class="kind">{node.kind}</div>
  <div class="hint">{$t("canvas.engine.unknownKind", { default: "This element needs a newer version" })}</div>
</div>

<style>
  .unknown {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    color: var(--text-muted);
    font-size: 12px;
    text-align: center;
    padding: 8px;
  }
  .kind {
    font: 600 12px/1 var(--font-mono, monospace);
    color: var(--text);
  }
</style>
