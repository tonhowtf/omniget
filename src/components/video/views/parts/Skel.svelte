<script lang="ts">
  let {
    kind = "grid",
    count = 8,
    ratio = "16 / 9",
    min = 200,
  }: { kind?: "grid" | "row" | "list"; count?: number; ratio?: string; min?: number } = $props();
</script>

{#if kind === "list"}
  <div class="list" aria-busy="true">
    {#each Array(count) as _, i (i)}
      <div class="li">
        <div class="skeleton th"></div>
        <div class="tx">
          <div class="skeleton l1"></div>
          <div class="skeleton l2"></div>
        </div>
      </div>
    {/each}
  </div>
{:else}
  <div class={kind} style:--min="{min}px" aria-busy="true">
    {#each Array(count) as _, i (i)}
      <div class="card">
        <div class="skeleton art" style:aspect-ratio={ratio}></div>
        <div class="skeleton l1"></div>
        <div class="skeleton l2"></div>
      </div>
    {/each}
  </div>
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--min), 1fr));
    gap: var(--space-4);
  }
  .row {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: var(--min);
    gap: var(--space-4);
    overflow: hidden;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .art {
    width: 100%;
    border-radius: var(--radius-md);
  }
  .l1 {
    width: 70%;
    height: 12px;
  }
  .l2 {
    width: 45%;
    height: 10px;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 6px;
  }
  .li .th {
    width: 96px;
    aspect-ratio: 16 / 9;
    border-radius: var(--radius-sm);
  }
  .tx {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
</style>
