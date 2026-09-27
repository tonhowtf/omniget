<script lang="ts">
  /**
   * Sem visual: liga o relógio diário do monitor de releases enquanto está
   * montado (e só enquanto a janela está visível). Monte uma vez num layout
   * que fica aberto (por exemplo `src/routes/llm/+layout.svelte`). Quem quiser
   * mostrar o badge escuta `clitools://updates` (ver `onUpdates`).
   */
  import { onMount } from "svelte";
  import { startDailyUpdates, type UpdatesReport } from "./config-health";

  let { onreport }: { onreport?: (r: UpdatesReport) => void } = $props();

  onMount(() => startDailyUpdates((r) => onreport?.(r)));
</script>
