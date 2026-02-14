<script lang="ts">
  type Module = {
    id: string;
    name: string;
    pages: { hash: string; name: string; page_type: string }[];
  };

  let { modules }: { modules: Module[] } = $props();
</script>

<ul class="module-list">
  {#each modules as mod, i}
    <li class="module-item">
      <div class="module-header">
        <span class="module-number">{i + 1}.</span>
        <span class="module-name">{mod.name}</span>
        <span class="module-count">{mod.pages.length} {mod.pages.length === 1 ? 'aula' : 'aulas'}</span>
      </div>
    </li>
  {/each}
</ul>

<style>
  .module-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .module-item {
    padding: 10px 12px;
    border-radius: 6px;
    transition: background-color 0.15s;
  }

  .module-item:hover {
    background: rgba(255, 255, 255, 0.03);
  }

  .module-header {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .module-number {
    color: var(--text-muted);
    font-size: 0.82rem;
    min-width: 22px;
    font-variant-numeric: tabular-nums;
  }

  .module-name {
    flex: 1;
    font-size: 0.88rem;
    color: var(--text);
  }

  .module-count {
    font-size: 0.75rem;
    color: var(--text-muted);
    background: rgba(255, 255, 255, 0.05);
    padding: 2px 8px;
    border-radius: 10px;
    white-space: nowrap;
  }
</style>
