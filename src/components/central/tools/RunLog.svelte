<script lang="ts">
  // Log ao vivo de uma execução (instalar/atualizar/ACP): linhas do
  // `clitools://progress` mais o resultado final.
  import { t } from "$lib/i18n";
  import type { LogEntry, RunState } from "$lib/central/clitools";

  let { entry, onclear }: { entry: LogEntry; onclear: () => void } = $props();

  let box = $state<HTMLPreElement | null>(null);
  $effect(() => {
    void entry.lines.length;
    if (box) box.scrollTop = box.scrollHeight;
  });
  const stateTag: Record<RunState, string> = {
    running: "tag-info",
    succeeded: "tag-success",
    failed: "tag-danger",
    unchanged: "",
  };
</script>

<section class="surface-card log" aria-live="polite">
  <header>
    <h2>{$t("llm.central.tools.log_title")} · {entry.title}</h2>
    <span class={`tag ${stateTag[entry.state]}`}>{$t(`llm.central.tools.run_state.${entry.state}`)}</span>
    <button class="button" type="button" disabled={entry.state === "running"} onclick={onclear}>
      {$t("llm.central.tools.close")}
    </button>
  </header>
  {#if entry.command}<code class="cmd">$ {entry.command}</code>{/if}
  <pre bind:this={box}>{#each entry.lines as l}<span class:err={l.stream === "stderr"}>{l.text}
</span>{/each}</pre>
  {#if entry.result}
    <p class="summary">
      {$t("llm.central.tools.exit")} {entry.result.exit_code ?? "—"} ·
      {entry.result.version_before ?? "—"} → {entry.result.version_after ?? "—"} ·
      {Math.round(entry.result.duration_ms / 100) / 10}s
      {#if entry.result.truncated}· {$t("llm.central.tools.truncated")}{/if}
    </p>
    {#each entry.result.warnings as w}<p class="warn">{w}</p>{/each}
  {/if}
</section>

<style>
  .log {
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  h2 {
    margin: 0;
    font-size: var(--text-md);
    flex: 1;
    min-width: 0;
  }
  .cmd {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    color: var(--text-muted);
    word-break: break-all;
  }
  pre {
    margin: 0;
    max-height: 280px;
    overflow: auto;
    background: var(--fill-1);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-all;
    user-select: text;
  }
  .err {
    color: var(--warning);
  }
  .summary,
  .warn {
    margin: 0;
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .warn {
    color: var(--warning);
  }
</style>
