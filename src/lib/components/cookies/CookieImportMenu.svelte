<script lang="ts">
  import { t } from "$lib/i18n";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";

  type Props = {
    onImportFile: (path: string) => void;
    onImportPaste: () => void;
  };

  let { onImportFile, onImportPaste }: Props = $props();

  let pickerOpen = $state(false);

  async function chooseFile() {
    pickerOpen = false;
    try {
      const selected = await openDialog({
        title: $t("settings.cookies.import_file_dialog_title") as string,
        filters: [
          { name: "Cookies", extensions: ["txt", "json"] },
          { name: "All", extensions: ["*"] },
        ],
        multiple: false,
      });
      if (selected && typeof selected === "string") {
        onImportFile(selected);
      }
    } catch (e) {
      console.warn("[cookies] file picker failed", e);
    }
  }

  function pickPaste() {
    pickerOpen = false;
    onImportPaste();
  }
</script>

<div class="import-menu">
  <button type="button" class="trigger" onclick={() => pickerOpen = !pickerOpen}>
    {$t("settings.cookies.import_trigger")}
    <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="6 9 12 15 18 9"/></svg>
  </button>
  {#if pickerOpen}
    <div class="menu" role="menu">
      <button type="button" class="menu-item" onclick={chooseFile} role="menuitem">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
          <polyline points="14 2 14 8 20 8"/>
        </svg>
        <div class="item-text">
          <span class="item-title">{$t("settings.cookies.import_file_title")}</span>
          <span class="item-hint">{$t("settings.cookies.import_file_hint")}</span>
        </div>
      </button>
      <button type="button" class="menu-item" onclick={pickPaste} role="menuitem">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <rect x="8" y="2" width="8" height="4" rx="1"/>
          <path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/>
        </svg>
        <div class="item-text">
          <span class="item-title">{$t("settings.cookies.import_paste_title")}</span>
          <span class="item-hint">{$t("settings.cookies.import_paste_hint")}</span>
        </div>
      </button>
    </div>
  {/if}
</div>

<svelte:window onclick={(e) => {
  if (pickerOpen && !(e.target as HTMLElement)?.closest(".import-menu")) {
    pickerOpen = false;
  }
}} />

<style>
  .import-menu { position: relative; display: inline-block; }
  .trigger {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 7px 14px;
    background: var(--cta);
    color: var(--on-cta);
    border: 0;
    border-radius: 999px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: filter 120ms;
  }
  .trigger:hover { filter: brightness(1.1); }
  .menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    min-width: 240px;
    background: var(--surface, var(--bg));
    border: none;
    border-radius: 10px;
    overflow: hidden;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
    z-index: 50;
    display: flex;
    flex-direction: column;
  }
  .menu-item {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 12px 14px;
    background: transparent;
    border: 0;
    color: var(--secondary);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background 120ms;
  }
  .menu-item:hover { background: color-mix(in oklab, var(--button) 40%, transparent); color: var(--secondary); }
  .menu-item svg { margin-top: 2px; flex-shrink: 0; }
  .item-text { display: flex; flex-direction: column; gap: 2px; }
  .item-title { font-size: 13px; font-weight: 500; }
  .item-hint { font-size: 11px; color: var(--tertiary); }
</style>
