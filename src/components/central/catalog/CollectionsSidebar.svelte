<script lang="ts">
  /**
   * Collections in the catalog sidebar: select (the main area then lists the
   * collection), create, rename, delete, reorder, import and export
   * `.omnistack` files.
   */
  import { tick } from "svelte";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { collections as coll, errText, type Collection } from "$lib/central/catalog";

  let {
    list = [],
    selected = $bindable<string | null>(null),
    onchanged,
  }: {
    list?: Collection[];
    selected?: string | null;
    onchanged?: () => void | Promise<void>;
  } = $props();

  let creating = $state(false);
  let newName = $state("");
  let editing = $state<string | null>(null);
  let editName = $state("");
  let confirmDelete = $state<string | null>(null);
  let busy = $state(false);
  let createInput = $state<HTMLInputElement | null>(null);

  async function run(fn: () => Promise<unknown>) {
    busy = true;
    try {
      await fn();
      await onchanged?.();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function startCreate() {
    creating = true;
    newName = "";
    await tick();
    createInput?.focus();
  }

  function create() {
    if (!creating) return;
    const name = newName.trim();
    creating = false;
    if (!name) return;
    void run(async () => {
      const c = await coll.create(name);
      selected = c.id;
    });
  }

  function rename(id: string) {
    if (editing !== id) return;
    const name = editName.trim();
    editing = null;
    if (!name || name === list.find((c) => c.id === id)?.name) return;
    void run(() => coll.rename(id, name));
  }

  function remove(id: string) {
    confirmDelete = null;
    void run(async () => {
      await coll.delete(id);
      if (selected === id) selected = null;
    });
  }

  function reorder(id: string, to: number) {
    if (to < 0 || to >= list.length) return;
    void run(() => coll.reorder(id, to));
  }

  async function importFile() {
    try {
      const path = await openDialog({ multiple: false, filters: [{ name: "OmniGet stack", extensions: ["omnistack", "json"] }] });
      if (typeof path !== "string") return;
      await run(async () => {
        const r = await coll.import(path);
        selected = r.collection.id;
        if (r.missing.length || r.changed.length) {
          showToast(
            "info",
            $t("llm.central.catalog.collections.import_report", {
              missing: String(r.missing.length),
              changed: String(r.changed.length),
            }),
          );
        } else showToast("success", $t("llm.central.catalog.collections.imported", { name: r.collection.name }));
      });
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function exportOne(c: Collection) {
    try {
      const path = await saveDialog({
        defaultPath: `${c.name.replace(/[^\w.-]+/g, "-")}.omnistack`,
        filters: [{ name: "OmniGet stack", extensions: ["omnistack"] }],
      });
      if (!path) return;
      const out = await coll.export(c.id, path);
      showToast("success", $t("llm.central.catalog.stack.exported", { path: out }));
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  function onItemKey(e: KeyboardEvent, c: Collection, i: number) {
    if (e.altKey && e.key === "ArrowUp") {
      e.preventDefault();
      reorder(c.id, i - 1);
    } else if (e.altKey && e.key === "ArrowDown") {
      e.preventDefault();
      reorder(c.id, i + 1);
    } else if (e.key === "F2") {
      e.preventDefault();
      editing = c.id;
      editName = c.name;
    }
  }
</script>

<section class="colls" aria-labelledby="colls-title">
  <div class="title-row">
    <h3 id="colls-title">{$t("llm.central.catalog.collections.title")}</h3>
    <span class="acts">
      <button type="button" onclick={importFile} title={$t("llm.central.catalog.collections.import")} aria-label={$t("llm.central.catalog.collections.import")} disabled={busy}>⤓</button>
      <button type="button" onclick={startCreate} title={$t("llm.central.catalog.collections.new")} aria-label={$t("llm.central.catalog.collections.new")} disabled={busy}>+</button>
    </span>
  </div>
  <ul>
    {#each list as c, i (c.id)}
      <li class:on={selected === c.id}>
        {#if editing === c.id}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="edit"
            bind:value={editName}
            autofocus
            onkeydown={(e) => {
              if (e.key === "Enter") rename(c.id);
              if (e.key === "Escape") editing = null;
            }}
            onblur={() => rename(c.id)}
            aria-label={$t("llm.central.catalog.collections.rename")}
          />
        {:else if confirmDelete === c.id}
          <span class="confirm">
            {$t("llm.central.catalog.collections.delete_q")}
            <button type="button" class="danger" onclick={() => remove(c.id)}>{$t("llm.central.catalog.collections.delete")}</button>
            <button type="button" onclick={() => (confirmDelete = null)}>{$t("llm.central.catalog.cancel")}</button>
          </span>
        {:else}
          <button
            type="button"
            class="sel"
            aria-pressed={selected === c.id}
            onclick={() => (selected = selected === c.id ? null : c.id)}
            onkeydown={(e) => onItemKey(e, c, i)}
            title={c.description ?? c.name}
          >
            <span class="nm">{c.name}</span>
            <span class="n tabular">{c.items.length}</span>
          </button>
          <span class="row-acts">
            <button type="button" onclick={() => reorder(c.id, i - 1)} disabled={i === 0 || busy} aria-label={$t("llm.central.catalog.move_up")}>↑</button>
            <button type="button" onclick={() => reorder(c.id, i + 1)} disabled={i === list.length - 1 || busy} aria-label={$t("llm.central.catalog.move_down")}>↓</button>
            <button type="button" onclick={() => { editing = c.id; editName = c.name; }} aria-label={$t("llm.central.catalog.collections.rename")}>✎</button>
            <button type="button" onclick={() => exportOne(c)} aria-label={$t("llm.central.catalog.collections.export")}>⤒</button>
            <button type="button" onclick={() => (confirmDelete = c.id)} aria-label={$t("llm.central.catalog.collections.delete")}>✕</button>
          </span>
        {/if}
      </li>
    {/each}
    {#if creating}
      <li>
        <input
          class="edit"
          bind:this={createInput}
          bind:value={newName}
          placeholder={$t("llm.central.catalog.collections.name")}
          onkeydown={(e) => {
            if (e.key === "Enter") create();
            if (e.key === "Escape") creating = false;
          }}
          onblur={create}
          aria-label={$t("llm.central.catalog.collections.name")}
        />
      </li>
    {/if}
  </ul>
  {#if !list.length && !creating}
    <p class="empty">{$t("llm.central.catalog.collections.empty")}</p>
  {/if}
</section>

<style>
  .colls {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .title-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h3 {
    margin: 0;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
  }
  .acts button,
  .row-acts button {
    width: 22px;
    height: 22px;
    border: none;
    border-radius: var(--radius-xs);
    background: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: var(--text-sm);
  }
  .acts button:hover:not(:disabled),
  .row-acts button:hover:not(:disabled) {
    background: var(--fill-2);
    color: var(--text);
  }
  .row-acts button:disabled {
    opacity: 0.3;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  li {
    position: relative;
    display: flex;
    align-items: center;
    border-radius: var(--radius-sm);
  }
  li:hover,
  li.on {
    background: var(--fill-1);
  }
  li.on .sel {
    color: var(--accent-hi);
  }
  .sel {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-1);
    height: 28px;
    padding: 0 var(--space-2);
    border: none;
    background: none;
    color: var(--text);
    font-size: var(--text-sm);
    text-align: left;
    cursor: pointer;
  }
  .nm {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .n {
    color: var(--text-faint);
    font-size: var(--text-xs);
  }
  .row-acts {
    display: none;
    padding-right: 2px;
  }
  li:hover .row-acts,
  li:focus-within .row-acts {
    display: inline-flex;
  }
  li:hover .n,
  li:focus-within .n {
    display: none;
  }
  .edit {
    width: 100%;
    height: 28px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .confirm {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px var(--space-2);
    font-size: var(--text-xs);
  }
  .confirm button {
    border: none;
    background: var(--fill-1);
    border-radius: var(--radius-xs);
    color: var(--text);
    font-size: var(--text-xs);
    cursor: pointer;
    padding: 2px 6px;
  }
  .confirm .danger {
    background: var(--error);
    color: var(--on-error);
  }
  .empty {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-faint);
  }
</style>
