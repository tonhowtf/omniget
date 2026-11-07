<script lang="ts">
  import { t } from "$lib/i18n";
  import Modal from "$components/music/Modal.svelte";
  import { formatBytes } from "$lib/photos/plan";
  import type { RemoveMode } from "$lib/photos/types";

  let {
    count,
    bytes,
    emptiesGroup = false,
    onclose,
    onconfirm,
  }: {
    count: number;
    bytes: number;
    emptiesGroup?: boolean;
    onclose: () => void;
    onconfirm: (mode: RemoveMode) => void;
  } = $props();

  let forever = $state(false);
</script>

<Modal title={(count === 1 ? $t("photos.remove.title_one") : $t("photos.remove.title_many", { count })) as string} width={440} {onclose}>
  <p class="dialog-body">
    {forever ? $t("photos.remove.body_delete", { size: formatBytes(bytes) }) : $t("photos.remove.body_trash", { size: formatBytes(bytes) })}
  </p>
  {#if emptiesGroup}
    <p class="dialog-body rm-warn">{$t("photos.remove.empties_group")}</p>
  {/if}
  <label class="rm-check">
    <input type="checkbox" class="checkbox" bind:checked={forever} />
    <span>{$t("photos.remove.forever")}</span>
  </label>
  <div class="dialog-actions">
    <button type="button" class="btn" onclick={onclose}>{$t("photos.editor.cancel")}</button>
    <button type="button" class="btn" class:btn-destructive={forever} class:btn-primary={!forever} onclick={() => onconfirm(forever ? "delete" : "trash")}>
      {forever ? $t("photos.remove.confirm_delete") : $t("photos.remove.confirm_trash")}
    </button>
  </div>
</Modal>

<style>
  .rm-warn {
    color: var(--warning);
    font-weight: 600;
  }

  .rm-check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-3) 0;
    font-size: var(--text-sm);
    color: var(--text);
  }
</style>
