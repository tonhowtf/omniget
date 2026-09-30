<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { library, refreshShelves } from "$lib/read/library-store.svelte";
  import Modal from "$components/music/Modal.svelte";
  import ReadIcon from "./ReadIcon.svelte";

  let { ids, onclose }: { ids: number[]; onclose: () => void } = $props();
  let lib = $derived(library());
  let name = $state("");

  async function add(shelfId: number) {
    try {
      await invoke("read_shelf_add", { shelfId, bookIds: ids });
      await refreshShelves();
      showToast("success", $t("read.shelf.added", { count: ids.length }) as string);
      onclose();
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function create() {
    if (!name.trim()) return;
    try {
      const id = await invoke<number>("read_shelf_save", { id: null, name: name.trim(), icon: null, smart: null });
      await add(id);
    } catch (e) {
      showToast("error", String(e));
    }
  }
</script>

<Modal title={$t("read.shelf.add_to") as string} width={380} {onclose}>
  <ul class="r-list sp">
    {#each lib.shelves.filter((s) => !s.smart) as s (s.id)}
      <li>
        <button class="row" onclick={() => add(s.id)}><ReadIcon name="shelf" size={15} /> <span>{s.name}</span> <em>{s.book_ids.length}</em></button>
      </li>
    {/each}
  </ul>
  <form
    class="nf"
    onsubmit={(e) => {
      e.preventDefault();
      void create();
    }}
  >
    <input class="input" bind:value={name} placeholder={$t("read.shelf.new_name") as string} />
    <button type="submit" class="btn btn-primary" disabled={!name.trim()}>{$t("read.shelf.create")}</button>
  </form>
</Modal>

<style>
  .sp {
    max-height: 280px;
    overflow-y: auto;
    margin-bottom: var(--space-3);
  }
  .row {
    all: unset;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .row:hover,
  .row:focus-visible {
    background: var(--fill-1);
  }
  .row span {
    flex: 1;
  }
  .row em {
    font-style: normal;
    color: var(--text-muted);
    font-size: var(--text-sm);
  }
  .nf {
    display: flex;
    gap: var(--space-2);
  }
  .nf .input {
    flex: 1;
  }
</style>
