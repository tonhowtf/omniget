<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import type { DevtoolsError } from "$lib/devtools/types";
  import StatusIcon from "#components/converter/StatusIcon.svelte";

  let { error, children }: { error: DevtoolsError; children?: Snippet } = $props();
</script>

<div class="en" role="alert">
  <StatusIcon name="failed" size={18} />
  <div class="en-copy">
    <p class="en-message">{error.message}</p>
    {#if error.nextStep}
      <p class="en-next"><b>{$t("devtools.next_step")}</b> {error.nextStep}</p>
    {/if}
    {@render children?.()}
    <span class="en-code">{error.code}</span>
  </div>
</div>

<style>
  .en {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--danger) 8%, var(--surface));
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--danger) 40%, var(--content-border));
  }

  .en-copy {
    display: flex;
    flex: 1;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    min-width: 0;
  }

  .en p {
    margin: 0;
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
    user-select: text;
  }

  .en-message {
    color: var(--text);
    font-weight: 600;
  }

  .en-next {
    color: var(--text-muted);
  }

  .en-next b {
    color: var(--text);
    font-weight: 600;
  }

  .en-code {
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }
</style>
