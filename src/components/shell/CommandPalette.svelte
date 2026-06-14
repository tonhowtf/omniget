<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import {
    closeCommandPalette,
    getCommandPaletteItems,
    getCommandPaletteQuery,
    getCommandPaletteSelectedIndex,
    isCommandPaletteOpen,
    moveCommandPaletteSelection,
    runCommandPaletteSelected,
    setCommandPaletteQuery,
    setCommandPaletteSelectedIndex,
  } from "$lib/stores/command-palette-store.svelte";

  let inputEl = $state<HTMLInputElement | null>(null);

  let open = $derived(isCommandPaletteOpen());
  let query = $derived(getCommandPaletteQuery());
  let selectedIndex = $derived(getCommandPaletteSelectedIndex());
  let allItems = $derived(getCommandPaletteItems());

  let filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return allItems;
    return allItems.filter((item) => {
      const hay = `${item.label} ${item.group} ${item.keywords ?? ""}`.toLowerCase();
      return hay.includes(q);
    });
  });

  $effect(() => {
    if (open && inputEl) {
      inputEl.focus();
    }
  });

  $effect(() => {
    if (selectedIndex >= filtered.length) {
      setCommandPaletteSelectedIndex(0);
    }
  });

  function onKeydown(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      closeCommandPalette();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      moveCommandPaletteSelection(1, filtered.length);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      moveCommandPaletteSelection(-1, filtered.length);
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      runCommandPaletteSelected(filtered);
    }
  }

  onMount(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        import("$lib/stores/command-palette-store.svelte").then((m) => m.openCommandPalette());
      }
      onKeydown(e);
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  });
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="mac-command-backdrop" onclick={() => closeCommandPalette()}>
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="mac-command-palette" onclick={(e) => e.stopPropagation()}>
      <div class="mac-command-input-row">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
          <circle cx="11" cy="11" r="8" />
          <path d="M21 21l-4.35-4.35" />
        </svg>
        <input
          bind:this={inputEl}
          class="mac-command-input"
          type="search"
          placeholder={$t("command_palette.placeholder")}
          value={query}
          oninput={(e) => setCommandPaletteQuery((e.target as HTMLInputElement).value)}
          spellcheck="false"
        />
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => closeCommandPalette()} aria-label={$t("common.close")}>
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </div>
      <div class="mac-command-list" role="listbox">
        {#each filtered as item, i (item.id)}
          <button
            type="button"
            class="mac-command-item"
            class:selected={i === selectedIndex}
            onclick={() => runCommandPaletteSelected(filtered)}
            onmouseenter={() => setCommandPaletteSelectedIndex(i)}
          >
            <span>{item.label}</span>
            <span style="margin-left: auto; font-size: 11px; color: var(--text-dim);">{item.group}</span>
          </button>
        {:else}
          <div style="padding: 16px; text-align: center; color: var(--text-dim); font-size: 13px;">
            {$t("command_palette.empty")}
          </div>
        {/each}
      </div>
      <div class="mac-command-footer">
        <span><span class="mac-kbd">↑</span> <span class="mac-kbd">↓</span> {$t("command_palette.navigate")}</span>
        <span><span class="mac-kbd">↵</span> {$t("command_palette.select")}</span>
        <span><span class="mac-kbd">esc</span> {$t("command_palette.close")}</span>
      </div>
    </div>
  </div>
{/if}
