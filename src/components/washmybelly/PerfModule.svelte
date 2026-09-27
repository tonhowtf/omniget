<script lang="ts">
  import { t } from "$lib/i18n";
  import type { Caps } from "./registry";
  import ComingSoon from "./ComingSoon.svelte";

  // Stub created by F0; the module owner replaces it.
  let _props: { caps: Caps } = $props();
</script>

<ComingSoon title={$t("washmybelly.modules.perf") as string} />
