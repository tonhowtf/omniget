<script lang="ts">
  import { t } from "$lib/i18n";
  import { iconifyId } from "$lib/icons/sets";
  import type { Library, SavedIcon } from "$lib/icons/types";
  import IconView from "./IconView.svelte";
  import Glyph from "./Glyph.svelte";

  type Target = "favorites" | string;

  let {
    library,
    selected = null,
    exporting = null,
    isPalette,
    onopen,
    onremove,
    oncreate,
    onrename,
    ondelete,
    onexport,
    onsearch,
  }: {
    library: Library;
    selected?: string | null;
    exporting?: { target: Target; mode: "folder" | "sprite" } | null;
    isPalette: (prefix: string) => boolean;
    onopen: (saved: SavedIcon) => void;
    onremove: (target: Target, saved: SavedIcon) => void;
    oncreate: (name: string) => void;
    onrename: (id: string, name: string) => void;
    ondelete: (id: string) => void;
    onexport: (target: Target, mode: "folder" | "sprite") => void;
    onsearch: () => void;
  } = $props();

  let newName = $state("");
  let renaming = $state<string | null>(null);
  let renameValue = $state("");
  let confirmDelete = $state<string | null>(null);

  let groups = $derived([
    { id: "favorites" as Target, name: $t("icons.favorites") as string, icons: library.favorites, editable: false },
    ...library.collections.map((c) => ({ id: c.id as Target, name: c.name, icons: c.icons, editable: true })),
  ]);

  function create(event: SubmitEvent) {
    event.preventDefault();
    const name = newName.trim();
    if (!name) return;
    oncreate(name);
    newName = "";
  }

  function startRename(id: string, name: string) {
    renaming = id;
    renameValue = name;
  }

  function cancelOnEscape(event: KeyboardEvent) {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    renaming = null;
  }

  function finishRename(event: SubmitEvent) {
    event.preventDefault();
    if (renaming && renameValue.trim()) onrename(renaming, renameValue.trim());
    renaming = null;
  }
</script>

<div class="ic-lib">
  <form class="ic-lib-new" onsubmit={create}>
    <label for="ic-lib-new-name" class="ic-lib-new-label">{$t("icons.new_collection")}</label>
    <input id="ic-lib-new-name" class="ic-input" bind:value={newName} maxlength="80" placeholder={$t("icons.collection_name_placeholder") as string} />
    <button type="submit" class="btn btn-secondary btn-sm" disabled={!newName.trim()} title={!newName.trim() ? ($t("icons.collection_name_needed") as string) : undefined}>
      <Glyph name="plus" size={13} />
      {$t("icons.create_collection")}
    </button>
  </form>

  {#each groups as group (group.id)}
    {@const busy = exporting?.target === group.id}
    <section class="ic-lib-card" aria-labelledby="ic-lib-{group.id}">
      <header class="ic-lib-head">
        {#if renaming === group.id}
          <form class="ic-lib-rename" onsubmit={finishRename}>
            <label class="ic-sr" for="ic-rename-{group.id}">{$t("icons.collection_name")}</label>
            <!-- svelte-ignore a11y_autofocus -->
            <input id="ic-rename-{group.id}" class="ic-input" bind:value={renameValue} maxlength="80" autofocus onkeydown={cancelOnEscape} />
            <button type="submit" class="btn btn-secondary btn-sm" disabled={!renameValue.trim()}>{$t("icons.rename_save")}</button>
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => (renaming = null)}>{$t("icons.cancel")}</button>
          </form>
        {:else}
          <h2 id="ic-lib-{group.id}">
            {#if group.id === "favorites"}<Glyph name="star-fill" size={14} />{/if}
            {group.name}
            <span class="ic-lib-count">{$t("icons.icon_count", { count: group.icons.length })}</span>
          </h2>
          <div class="ic-lib-tools">
            <button type="button" class="btn btn-secondary btn-sm" disabled={group.icons.length === 0 || exporting !== null} title={group.icons.length === 0 ? ($t("icons.export_empty") as string) : undefined} onclick={() => onexport(group.id, "folder")}>
              {#if busy && exporting?.mode === "folder"}<span class="spinner ic-spin" aria-hidden="true"></span>{:else}<Glyph name="folder" size={13} />{/if}
              {$t("icons.export_folder")}
            </button>
            <button type="button" class="btn btn-secondary btn-sm" disabled={group.icons.length === 0 || exporting !== null} title={group.icons.length === 0 ? ($t("icons.export_empty") as string) : undefined} onclick={() => onexport(group.id, "sprite")}>
              {#if busy && exporting?.mode === "sprite"}<span class="spinner ic-spin" aria-hidden="true"></span>{:else}<Glyph name="download" size={13} />{/if}
              {$t("icons.export_sprite")}
            </button>
            {#if group.editable}
              <button type="button" class="btn btn-icon btn-ghost btn-sm" aria-label={$t("icons.rename_named", { name: group.name }) as string} onclick={() => startRename(group.id, group.name)}><Glyph name="pencil" size={13} /></button>
              <button type="button" class="btn btn-icon btn-ghost btn-sm" aria-label={$t("icons.delete_named", { name: group.name }) as string} onclick={() => (confirmDelete = group.id)}><Glyph name="trash" size={13} /></button>
            {/if}
          </div>
        {/if}
      </header>

      {#if confirmDelete === group.id}
        <div class="ic-lib-confirm" role="alertdialog" aria-labelledby="ic-del-{group.id}">
          <p id="ic-del-{group.id}">{$t("icons.delete_confirm", { name: group.name, count: group.icons.length })}</p>
          <button type="button" class="btn btn-destructive btn-sm" onclick={() => { ondelete(group.id); confirmDelete = null; }}>{$t("icons.delete_collection")}</button>
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => (confirmDelete = null)}>{$t("icons.keep_collection")}</button>
        </div>
      {/if}

      {#if group.icons.length === 0}
        <p class="ic-lib-empty">
          {group.id === "favorites" ? $t("icons.favorites_empty") : $t("icons.collection_empty")}
          <button type="button" class="btn btn-ghost btn-sm" onclick={onsearch}>{$t("icons.go_search")}</button>
        </p>
      {:else}
        <ul class="ic-lib-icons">
          {#each group.icons as saved (iconifyId(saved.prefix, saved.icon.name))}
            {@const sid = iconifyId(saved.prefix, saved.icon.name)}
            <li class="ic-lib-item" class:on={selected === sid}>
              <button type="button" class="ic-lib-open" title={sid} aria-label={$t("icons.cell_label", { name: saved.icon.name, set: saved.set_name ?? saved.prefix }) as string} onclick={() => onopen(saved)}>
                <IconView icon={saved.icon} size={24} palette={isPalette(saved.prefix)} />
              </button>
              <button type="button" class="ic-lib-remove" aria-label={$t("icons.remove_from", { name: saved.icon.name, from: group.name }) as string} onclick={() => onremove(group.id, saved)}>
                <Glyph name="close" size={10} />
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {/each}
</div>

<style>
  .ic-lib {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .ic-lib-new,
  .ic-lib-rename {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .ic-lib-new-label {
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  .ic-input {
    flex: 1;
    min-width: 160px;
    max-width: 320px;
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
  }

  .ic-input:focus-visible,
  .ic-lib-open:focus-visible,
  .ic-lib-remove:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .ic-lib-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ic-lib-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .ic-lib-head h2 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    color: var(--text);
    font-size: var(--text-md);
    font-weight: 650;
  }

  .ic-lib-head h2 :global(.ic-glyph) {
    color: var(--accent-text);
  }

  .ic-lib-count {
    color: var(--text-dim);
    font-size: var(--text-sm);
    font-weight: 500;
  }

  .ic-lib-tools {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .ic-lib-tools .btn {
    gap: var(--space-1);
  }

  .ic-lib-confirm {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--error) 10%, transparent);
  }

  .ic-lib-confirm p {
    flex: 1;
    margin: 0;
    color: var(--text);
    font-size: var(--text-sm);
  }

  .ic-lib-empty {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .ic-lib-icons {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .ic-lib-item {
    position: relative;
  }

  .ic-lib-open {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 52px;
    height: 52px;
    border: none;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    color: var(--text);
    cursor: pointer;
  }

  @media (hover: hover) {
    .ic-lib-open:hover {
      background: var(--fill-2);
    }
  }

  .ic-lib-item.on .ic-lib-open {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }

  .ic-lib-remove {
    position: absolute;
    top: -4px;
    right: -4px;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: none;
    border-radius: var(--radius-full);
    background: var(--surface-hi);
    box-shadow: var(--elev-1);
    color: var(--text-muted);
    cursor: pointer;
    opacity: 0;
  }

  .ic-lib-item:hover .ic-lib-remove,
  .ic-lib-item:focus-within .ic-lib-remove {
    opacity: 1;
  }

  .ic-spin {
    width: 12px;
    height: 12px;
  }

  .ic-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
