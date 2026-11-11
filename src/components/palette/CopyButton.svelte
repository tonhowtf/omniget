<script lang="ts">
  import { t } from "$lib/i18n";

  let { text, label, compact = false, named = false }: { text: string; label: string; compact?: boolean; named?: boolean } = $props();

  let state = $state<"idle" | "copied" | "failed">("idle");
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      state = "copied";
    } catch {
      state = "failed";
    }
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => (state = "idle"), 1600);
  }
</script>

<button type="button" class="btn btn-ghost btn-sm pl-copy" class:compact aria-label={label} title={label} onclick={copy}>
  {#if state === "copied"}
    <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m3.5 8.5 3 3 6-7" /></svg>
  {:else}
    <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" aria-hidden="true"><rect x="5.5" y="5.5" width="8" height="8" rx="1.5" /><path d="M10.5 5.5V3.8a1.3 1.3 0 0 0-1.3-1.3H3.8a1.3 1.3 0 0 0-1.3 1.3v5.4a1.3 1.3 0 0 0 1.3 1.3h1.7" /></svg>
  {/if}
  {#if !compact}
    <span>{state === "copied" ? $t("palette.copied") : state === "failed" ? $t("palette.copy_failed") : named ? label : $t("palette.copy")}</span>
  {/if}
</button>
<span class="pl-sr" role="status">{state === "copied" ? $t("palette.copied") : state === "failed" ? $t("palette.copy_failed") : ""}</span>

<style>
  .pl-copy {
    gap: var(--space-1);
    flex-shrink: 0;
  }

  .pl-copy.compact {
    width: var(--control-h);
    padding: 0;
    justify-content: center;
  }

  .pl-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
