<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import DialogContainer from "./DialogContainer.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";

  type RecoveryItem = {
    id: number;
    url: string;
    title: string;
    platform: string;
    output_dir: string;
    download_mode: string | null;
    quality: string | null;
    format_id: string | null;
    referer: string | null;
  };

  let isOpen = $state(false);
  let items = $state<RecoveryItem[]>([]);
  let busy = $state(false);

  async function loadItems() {
    try {
      const fetched = await invoke<RecoveryItem[]>("get_recovery_items");
      if (fetched.length > 0) {
        items = fetched;
        isOpen = true;
      }
    } catch {}
  }

  onMount(() => {
    let unlisten: (() => void) | undefined;
    listen<{ count: number }>("recovery-available", () => {
      loadItems();
    }).then((fn) => {
      unlisten = fn;
    });
    loadItems();
    return () => {
      unlisten?.();
    };
  });

  async function discard() {
    if (busy) return;
    busy = true;
    try {
      await invoke("discard_recovery");
      items = [];
      isOpen = false;
    } finally {
      busy = false;
    }
  }

  async function restore() {
    if (busy) return;
    busy = true;
    try {
      const restored = await invoke<number>("restore_recovery");
      showToast("info", $t("recovery.restored", { values: { count: restored } }));
      items = [];
      isOpen = false;
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "";
      if (msg) showToast("error", msg);
    } finally {
      busy = false;
    }
  }
</script>

<DialogContainer bind:isOpen titleId="recovery-title" onClose={discard}>
  <h3 id="recovery-title" class="dialog-title">
    {$t("recovery.title")}
  </h3>
  <p class="dialog-body">
    {$t("recovery.message", { values: { count: items.length } })}
  </p>
  {#if items.length > 0}
    <ul class="dialog-items">
      {#each items.slice(0, 5) as item (item.id)}
        <li class="dialog-item">
          <span class="item-platform">{item.platform}</span>
          <span class="item-title">{item.title}</span>
        </li>
      {/each}
      {#if items.length > 5}
        <li class="dialog-item dialog-item-more">
          {$t("recovery.more", { values: { count: items.length - 5 } })}
        </li>
      {/if}
    </ul>
  {/if}
  <div class="dialog-actions">
    <button type="button" class="btn btn-secondary" onclick={discard} disabled={busy}>
      {$t("recovery.discard")}
    </button>
    <button type="button" class="btn btn-primary" onclick={restore} disabled={busy}>
      {$t("recovery.restore")}
    </button>
  </div>
</DialogContainer>

<style>
  .dialog-title {
    margin: 0;
    padding: calc(var(--padding) * 1.25) calc(var(--padding) * 1.5) calc(var(--padding) * 0.75);
    font-size: 15px;
    font-weight: 600;
    color: var(--secondary);
  }

  .dialog-body {
    margin: 0;
    padding: 0 calc(var(--padding) * 1.5) calc(var(--padding) * 0.75);
    font-size: 13px;
    line-height: 1.5;
    color: var(--secondary);
    opacity: 0.85;
  }

  .dialog-items {
    list-style: none;
    margin: 0;
    padding: 0 calc(var(--padding) * 1.5) calc(var(--padding) * 1);
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 200px;
    overflow-y: auto;
  }

  .dialog-item {
    display: flex;
    gap: 8px;
    align-items: baseline;
    font-size: 12px;
    color: var(--gray);
  }

  .item-platform {
    font-weight: 600;
    color: var(--secondary);
    min-width: 70px;
  }

  .item-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dialog-item-more {
    font-style: italic;
    opacity: 0.6;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: calc(var(--padding) * 0.5);
    padding: calc(var(--padding) * 0.75) calc(var(--padding) * 1.5) calc(var(--padding) * 1.25);
    border-top: none;
  }







</style>
