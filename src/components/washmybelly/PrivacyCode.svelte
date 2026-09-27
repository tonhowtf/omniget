<script lang="ts">
  import { t } from "$lib/i18n";
  import Icon from "./ui/Icon.svelte";

  /// Read-only code with a Copy button, for items OmniGet shows but does
  /// not run and for the terminal form of an administrator step. Ported
  /// from the "Code Block" pattern (kvnkld, 21st.dev): a small header with
  /// a label and the copy action over a monospaced body; here without line
  /// numbers or highlighting, wrapping long lines, in the app's tokens.
  let { code, label = "", maxHeight = 220 }: { code: string; label?: string; maxHeight?: number } = $props();

  let copied = $state(false);
  let failed = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function copy() {
    failed = false;
    try {
      await navigator.clipboard.writeText(code);
      copied = true;
      clearTimeout(timer);
      timer = setTimeout(() => (copied = false), 1600);
    } catch {
      failed = true;
    }
  }

  $effect(() => () => clearTimeout(timer));
</script>

<div class="pc">
  <div class="pc-head">
    <span class="pc-label">{label || $t("washmybelly.privacy.code.label")}</span>
    <button type="button" class="btn btn-sm btn-ghost pc-copy" onclick={() => void copy()} aria-live="polite">
      <Icon name={copied ? "check" : "copy"} size={12} />
      {copied ? $t("washmybelly.privacy.code.copied") : $t("washmybelly.privacy.code.copy")}
    </button>
  </div>
  <pre class="pc-body" style:max-height="{maxHeight}px"><code>{code}</code></pre>
  {#if failed}<p class="pc-err">{$t("washmybelly.privacy.code.copy_failed")}</p>{/if}
</div>

<style>
  .pc {
    border-radius: var(--radius-md);
    background: var(--fill-1);
    overflow: hidden;
    min-width: 0;
  }
  .pc-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: 4px 4px 4px var(--space-3);
    border-bottom: 1px solid var(--separator, var(--border));
  }
  .pc-label {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-weight: 500;
  }
  .pc-copy {
    gap: 4px;
  }
  .pc-body {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    overflow: auto;
    font-family: var(--font-mono);
    font-size: 11.5px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-all;
    color: var(--text);
    user-select: text;
  }
  .pc-err {
    margin: 0;
    padding: 0 var(--space-3) var(--space-2);
    font-size: var(--text-xs);
    color: var(--danger);
  }
</style>
