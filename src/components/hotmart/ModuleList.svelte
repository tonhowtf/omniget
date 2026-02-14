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
    padding: calc(var(--padding) / 1.5) var(--padding);
    border-radius: calc(var(--border-radius) - 4px);
  }

  @media (hover: hover) {
    .module-item:hover {
      background: var(--button-stroke);
    }
  }

  .module-header {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 1.5);
  }

  .module-number {
    color: var(--gray);
    font-size: 12.5px;
    font-weight: 500;
    min-width: 22px;
    font-variant-numeric: tabular-nums;
  }

  .module-name {
    flex: 1;
    font-size: 14.5px;
    color: var(--secondary);
  }

  .module-count {
    font-size: 11px;
    font-weight: 500;
    color: var(--gray);
    background: var(--button-stroke);
    padding: 2px calc(var(--padding) / 2);
    border-radius: calc(var(--border-radius) / 2);
    white-space: nowrap;
  }
</style>
