<script lang="ts">
  import type { Snippet } from 'svelte';
  import { page } from '$app/state';
  import { t } from '$lib/i18n';
  import { LLM_PRIMARY_DESTINATIONS, LLM_SETTINGS_HREF, llmDestination } from '$lib/llm/workspace-navigation';
  import UsageMonitor from '$components/llm/UsageMonitor.svelte';
  let { children }: { children: Snippet } = $props();
  let path = $derived(page.url.pathname.replace(/\/$/, '') || '/llm');
  let destination = $derived(llmDestination(path));
</script>
<div class="llm-root">
  <header class="workspace-header">
    <nav class="llm-tabs" aria-label={$t('llm.title')}>
      {#each LLM_PRIMARY_DESTINATIONS as item (item.id)}
        <a class="llm-tab" href={item.href} aria-current={destination.id === item.id ? 'page' : undefined}>{$t(`llm.workspace.${item.id}`)}</a>
      {/each}
      <!-- Configure leaves the workspace for Settings (AI and agents); managers keep their URLs. -->
      <a class="llm-tab llm-configure" href={LLM_SETTINGS_HREF}>{$t('llm.workspace.configure')}</a>
    </nav>
    <UsageMonitor />
  </header>
  {#if destination.tools.length}
    <nav class="workspace-tools" aria-label={$t(`llm.workspace.${destination.id}`)}>
      {#each destination.tools as tool (tool.href)}<a href={tool.href} aria-current={path === tool.href ? 'page' : undefined}>{$t(tool.key)}</a>{/each}
    </nav>
  {/if}
  <div class="llm-body">{@render children()}</div>
</div>
<style>
  .llm-root { flex: 1; min-height: 0; display: flex; flex-direction: column; overflow: hidden; }
  .workspace-header { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 12px 16px; border-bottom: 1px solid var(--separator); flex-shrink: 0; flex-wrap: wrap; }
  .llm-tabs { display: flex; gap: 4px; flex-wrap: wrap; }
  .llm-tab { padding: 8px 12px; border-radius: 8px; font-size: 14px; font-weight: 500; color: var(--text-muted); text-decoration: none; }
  .llm-tab[aria-current='page'] { background: var(--accent-soft); color: var(--accent-hi); }
  .llm-tab:focus-visible, .workspace-tools a:focus-visible { outline: var(--focus-ring); }
  .llm-configure { margin-inline-start: 8px; border-inline-start: 1px solid var(--separator); border-radius: 0 8px 8px 0; }
  .workspace-tools { display: flex; gap: 4px 16px; flex-wrap: wrap; padding: 8px 20px; border-bottom: 1px solid var(--separator); }
  .workspace-tools a { padding-block: 6px; font-size: 13px; color: var(--text-muted); text-decoration: none; }
  .workspace-tools a[aria-current='page'] { color: var(--accent-hi); text-decoration: underline; text-underline-offset: 6px; }
  .llm-body { flex: 1; min-height: 0; display: flex; flex-direction: column; overflow: hidden; }
  @media (max-width: 600px) { .workspace-header { padding: 8px; gap: 8px; } .llm-tab { padding-inline: 8px; } }
</style>
