<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import DialogContainer from "./DialogContainer.svelte";

  let isOpen = $state(false);

  const settings = $derived(getSettings());
  const globalHotkey = $derived(settings?.download.hotkey_binding || "Ctrl+Shift+V");

  type Shortcut = { keysKey?: string; keys?: string; labelKey: string };
  type Section = { titleKey: string; items: Shortcut[] };

  const sections = $derived<Section[]>([
    {
      titleKey: "shortcuts.section_application",
      items: [
        { keys: globalHotkey, labelKey: "shortcuts.paste_url" },
        { keys: "Ctrl+?", labelKey: "shortcuts.show_shortcuts" },
        { keys: "Ctrl+,", labelKey: "shortcuts.open_settings" },
        { keys: "Ctrl+F", labelKey: "shortcuts.search_settings" },
      ],
    },
    {
      titleKey: "shortcuts.section_downloads",
      items: [
        { keys: "Enter", labelKey: "shortcuts.start_download" },
        { keys: "Esc", labelKey: "shortcuts.cancel_dialog" },
      ],
    },
    {
      titleKey: "shortcuts.section_navigation",
      items: [
        { keys: "Ctrl+1", labelKey: "shortcuts.nav_home" },
        { keys: "Ctrl+2", labelKey: "shortcuts.nav_downloads" },
      ],
    },
  ]);

  function open() {
    isOpen = true;
  }

  function close() {
    isOpen = false;
  }

  function onKey(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    const tag = target?.tagName?.toLowerCase();
    const isEditable =
      tag === "input" ||
      tag === "textarea" ||
      tag === "select" ||
      target?.isContentEditable;
    if (isEditable) return;

    if (e.ctrlKey && !e.altKey && !e.metaKey && (e.key === "?" || e.key === "/")) {
      e.preventDefault();
      isOpen ? close() : open();
      return;
    }
    if (!e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey && e.key === "?") {
      e.preventDefault();
      isOpen ? close() : open();
    }
  }

  onMount(() => {
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  function keyTokens(combo: string): string[] {
    return combo.split("+").map((k) => k.trim()).filter(Boolean);
  }
</script>

<DialogContainer bind:isOpen onClose={close} titleId="shortcuts-dialog-title">
  <div class="shortcuts-header">
    <h2 id="shortcuts-dialog-title">{$t("shortcuts.title")}</h2>
    <button class="dialog-close" onclick={close} aria-label={$t("common.close")}>
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M18 6L6 18M6 6l12 12" />
      </svg>
    </button>
  </div>

  <div class="shortcuts-body">
    {#each sections as section}
      <section class="shortcut-section">
        <h3>{$t(section.titleKey)}</h3>
        <ul>
          {#each section.items as s}
            <li>
              <span class="shortcut-label">{$t(s.labelKey)}</span>
              <span class="shortcut-keys">
                {#each keyTokens(s.keys ?? "") as token, i}
                  {#if i > 0}<span class="plus">+</span>{/if}
                  <kbd>{token}</kbd>
                {/each}
              </span>
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  </div>
</DialogContainer>

<style>
  .shortcuts-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 16px 20px 8px;
  }

  .shortcuts-header h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    color: var(--secondary);
  }

  .dialog-close {
    background: transparent;
    border: none;
    color: var(--tertiary);
    padding: 4px;
    cursor: pointer;
    border-radius: var(--border-radius);
  }

  .dialog-close:hover {
    background: var(--button-hover);
    color: var(--secondary);
  }

  .shortcuts-body {
    padding: 8px 20px 20px;
    overflow-y: auto;
    max-height: 60vh;
  }

  .shortcut-section + .shortcut-section {
    margin-top: 16px;
  }

  .shortcut-section h3 {
    margin: 0 0 8px 0;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--tertiary);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  li {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 8px;
    border-radius: var(--border-radius);
    background: var(--button);
  }

  .shortcut-label {
    font-size: 13px;
    color: var(--secondary);
  }

  .shortcut-keys {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }

  kbd {
    display: inline-block;
    min-width: 20px;
    padding: 2px 6px;
    border: 1px solid var(--content-border);
    border-bottom-width: 2px;
    border-radius: 4px;
    background: var(--input-bg);
    color: var(--secondary);
    font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
    font-size: 11px;
    line-height: 1.2;
    text-align: center;
  }

  .plus {
    color: var(--tertiary);
    font-size: 11px;
  }
</style>
