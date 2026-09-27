<script lang="ts">
  import { t } from "$lib/i18n";
  import EmptyState from "./ui/EmptyState.svelte";

  /// Placeholder for a module whose screen is not built yet.
  let { title, body }: { title: string; body?: string } = $props();
</script>

<EmptyState art="searching" {title} body={body ?? ($t("washmybelly.coming_soon") as string)} />
