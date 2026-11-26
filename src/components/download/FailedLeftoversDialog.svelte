<script lang="ts">
  import { t } from "$lib/i18n";
  import { formatBytes } from "$lib/stores/download-store.svelte";
  import Sheet from "#components/washmybelly/ui/Sheet.svelte";

  export type LeftoverFile = { path: string; name: string; size_bytes: number };

  let {
    open = $bindable(false),
    files,
    onRemove,
  }: {
    open?: boolean;
    files: LeftoverFile[];
    onRemove: (deletePartials: boolean) => void;
  } = $props();

  let total = $derived(files.reduce((sum, f) => sum + f.size_bytes, 0));

  function finish(deletePartials: boolean) {
    open = false;
    onRemove(deletePartials);
  }
</script>

<Sheet bind:open label={$t("downloads.leftovers_title") as string}>
  <div class="body">
    <h2>{$t("downloads.leftovers_title")}</h2>
    <p>{$t("downloads.leftovers_body", { count: files.length, size: formatBytes(total) })}</p>
    <ul>
      {#each files as file (file.path)}
        <li title={file.path}><span class="name" dir="ltr">{file.name}</span><span class="size">{formatBytes(file.size_bytes)}</span></li>
      {/each}
    </ul>
  </div>
  {#snippet footer()}
    <div class="actions">
      <button type="button" class="secondary" onclick={() => (open = false)}>{$t("common.cancel")}</button>
      <button type="button" class="secondary" onclick={() => finish(false)}>{$t("downloads.leftovers_keep")}</button>
      <button type="button" class="danger" onclick={() => finish(true)}>{$t("downloads.leftovers_delete")}</button>
    </div>
  {/snippet}
</Sheet>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  p {
    margin: 0;
    font-size: 13px;
    color: var(--text-secondary, var(--text));
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 180px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: 12px;
  }

  li {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .size {
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  button {
    min-height: 32px;
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-md);
    font: inherit;
    font-size: 13px;
    cursor: pointer;
  }

  .secondary {
    background: transparent;
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
  }

  .danger {
    background: var(--danger, #c0392b);
    color: #fff;
  }
</style>
