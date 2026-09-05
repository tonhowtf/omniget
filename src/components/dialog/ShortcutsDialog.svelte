<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import DialogContainer from "./DialogContainer.svelte";

  let isOpen = $state(false);

  const settings = $derived(getSettings());
  const globalHotkey = $derived(settings?.download.hotkey_binding || "Ctrl+Shift+D");

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
    padding: var(--space-4) var(--space-5) var(--space-2);
  }

  .shortcuts-header h2 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 600;
    letter-spacing: var(--track-snug);
    color: var(--text);
  }

  .dialog-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    background: transparent;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    border-radius: var(--radius-sm);
  }

  .dialog-close:hover {
    background: var(--fill-2);
    color: var(--text);
  }

  .shortcuts-body {
    padding: var(--space-2) var(--space-5) var(--space-5);
    overflow-y: auto;
    max-height: 60vh;
  }

  .shortcut-section + .shortcut-section {
    margin-top: var(--space-5);
  }

  .shortcut-section h3 {
    margin: 0 0 var(--space-2) var(--space-2);
    font-size: var(--text-sm);
    font-weight: 600;
    text-transform: none;
    letter-spacing: 0;
    color: var(--text-dim);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0;
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
  }

  li {
    display: flex;
    justify-content: space-between;
    align-items: center;
    min-height: 34px;
    padding: var(--space-1) var(--space-3);
    position: relative;
  }

  li + li::before {
    content: "";
    position: absolute;
    top: 0;
    left: var(--space-3);
    right: 0;
    height: var(--hairline);
    background: var(--separator);
  }

  .shortcut-label {
    font-size: var(--text-base);
    color: var(--text);
  }

  .shortcut-keys {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }

  kbd {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 20px;
    height: 20px;
    padding: 0 6px;
    border: none;
    border-radius: 5px;
    background: var(--fill-2);
    color: var(--text-muted);
    font-family: var(--font-body);
    font-size: var(--text-xs);
    font-weight: 500;
    line-height: 1;
    text-align: center;
  }

  .plus {
    color: var(--text-faint);
    font-size: var(--text-xs);
  }
</style>
