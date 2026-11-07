<script lang="ts">
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import { t } from "$lib/i18n";

  let { text, label, small = true }: { text: string; label: string; small?: boolean } = $props();

  let state = $state<"idle" | "copied" | "failed">("idle");
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function copy() {
    try {
      await writeText(text);
      state = "copied";
    } catch {
      state = "failed";
    }
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => (state = "idle"), 1800);
  }
</script>

<button type="button" class="btn btn-ghost" class:btn-sm={small} onclick={copy} aria-live="polite">
  {state === "copied" ? $t("components.copied") : state === "failed" ? $t("components.copy_failed") : label}
</button>
