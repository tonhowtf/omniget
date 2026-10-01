<script lang="ts">
  import { onMount } from "svelte";
  import { startDailyUpdates, type UpdatesReport } from "./config-health";

  let { onreport }: { onreport?: (r: UpdatesReport) => void } = $props();

  onMount(() => startDailyUpdates((r) => onreport?.(r)));
</script>
