<script lang="ts">
  import { t } from "$lib/i18n";
  import type { McpRegistryEntry } from "$lib/llm/mcp";

  let {
    entry,
    installed = false,
    oninstall,
  }: {
    entry: McpRegistryEntry;
    installed?: boolean;
    oninstall: (entry: McpRegistryEntry) => void;
  } = $props();
</script>

<article class="card">
  <header>
    <h3 class="title">{entry.name}</h3>
    <span class="tag">{$t(`llm.mcp.transport.${entry.template.transport.kind}`)}</span>
  </header>
  <p class="desc">{$t(entry.descKey)}</p>
  {#if entry.needs.length > 0}
    <p class="needs mono">{$t("llm.mcp.reg_needs", { fields: entry.needs.join(", ") })}</p>
  {/if}
  <footer>
    <button type="button" class="button primary" onclick={() => oninstall(entry)}>
      {installed ? $t("llm.mcp.reg_reinstall") : $t("llm.mcp.reg_install")}
    </button>
    <a class="link" href={entry.homepage} target="_blank" rel="noreferrer noopener">
      {$t("llm.mcp.reg_homepage")}
    </a>
  </footer>
</article>

<style>
  .card {
    border: var(--hairline) solid var(--separator);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    background: var(--surface);
  }

  header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .title {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
  }

  .desc {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    flex: 1;
  }

  .needs {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .mono {
    font-family: var(--font-mono);
  }

  footer {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
</style>
