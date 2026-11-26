<script lang="ts">
  import { untrack } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import Modal from "#components/music/Modal.svelte";
  import type { Folder } from "$lib/photos/types";

  let {
    folders,
    onclose,
    onsave,
  }: { folders: Folder[]; onclose: () => void; onsave: (paths: string[]) => void } = $props();

  let list = $state<Folder[]>(untrack(() => folders.map((f) => ({ ...f }))));

  async function add() {
    const picked = await openDialog({ directory: true, multiple: true }).catch(() => null);
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    for (const path of paths) if (!list.some((f) => f.path === path)) list.push({ path, exists: true });
  }
</script>

<Modal title={$t("photos.folders.title") as string} width={520} {onclose}>
  <p class="dialog-body">{$t("photos.folders.body")}</p>
  {#if list.length === 0}
    <p class="fd-empty">{$t("photos.folders.none")}</p>
  {:else}
    <ul class="fd-list">
      {#each list as folder, i (folder.path)}
        <li class="fd-row">
          <span class="fd-path" title={folder.path}>{folder.path}</span>
          {#if !folder.exists}
            <span class="tag tag-warning">{$t("photos.folders.missing")}</span>
          {/if}
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => list.splice(i, 1)}>{$t("photos.folders.remove")}</button>
        </li>
      {/each}
    </ul>
  {/if}
  <p class="field-hint">{$t("photos.folders.hint")}</p>
  <div class="dialog-actions">
    <button type="button" class="btn btn-secondary" onclick={() => void add()}>{$t("photos.folders.add")}</button>
    <span class="fd-grow"></span>
    <button type="button" class="btn" onclick={onclose}>{$t("photos.editor.cancel")}</button>
    <button type="button" class="btn btn-primary" onclick={() => onsave(list.map((f) => f.path))}>{$t("photos.folders.save")}</button>
  </div>
</Modal>

<style>
  .fd-list {
    display: flex;
    flex-direction: column;
    margin: var(--space-3) 0;
    padding: 0;
    list-style: none;
    border-radius: var(--radius-md);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .fd-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
  }

  .fd-row + .fd-row {
    border-top: var(--hairline) solid var(--content-border);
  }

  .fd-path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--text);
    font-size: var(--text-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }

  .fd-empty {
    margin: var(--space-3) 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .fd-grow {
    flex: 1;
  }
</style>
