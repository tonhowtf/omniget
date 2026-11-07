<script lang="ts">
  import { t } from "$lib/i18n";
  import { copyText, errorText } from "$lib/motion/api";

  let { label, code, hint = "" }: { label: string; code: string; hint?: string } = $props();

  let copyState = $state<"idle" | "copied" | "failed">("idle");
  let failure = $state("");
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function copy() {
    try {
      await copyText(code);
      copyState = "copied";
    } catch (e) {
      failure = errorText(e);
      copyState = "failed";
    }
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => (copyState = "idle"), 2200);
  }
</script>

<div class="cb">
  <div class="cb-head">
    <span class="cb-label">{label}</span>
    <span class="cb-status" role="status" aria-live="polite">
      {#if copyState === "copied"}
        <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m3.5 8.5 3 3 6-7" /></svg>
        {$t("motion.copied")}
      {:else if copyState === "failed"}
        {$t("motion.copy_failed", { error: failure })}
      {/if}
    </span>
    <button type="button" class="btn btn-secondary btn-sm" onclick={copy} aria-label={$t("motion.copy_label", { what: label })}>{$t("motion.copy")}</button>
  </div>
  <pre class="cb-code"><code>{code}</code></pre>
  {#if hint}
    <p class="cb-hint">{hint}</p>
  {/if}
</div>

<style>
  .cb {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .cb-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .cb-label {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }

  .cb-status {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    margin-left: auto;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .cb-code {
    margin: 0;
    max-height: 260px;
    overflow: auto;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: var(--leading-xs);
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .cb-hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
