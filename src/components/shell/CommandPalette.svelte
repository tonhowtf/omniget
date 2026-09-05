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
  let listEl = $state<HTMLDivElement | null>(null);

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

  // Group headers only when the list is unfiltered; a search result reads
  // better as one flat ranked list.
  let grouped = $derived.by(() => {
    if (query.trim()) return null;
    const map = new Map<string, number[]>();
    filtered.forEach((item, i) => {
      const list = map.get(item.group) ?? [];
      list.push(i);
      map.set(item.group, list);
    });
    return [...map.entries()];
  });

  $effect(() => {
    if (open && inputEl) {
      inputEl.focus();
      inputEl.select();
    }
  });

  $effect(() => {
    if (selectedIndex >= filtered.length) {
      setCommandPaletteSelectedIndex(0);
    }
  });

  $effect(() => {
    if (!open || !listEl) return;
    const el = listEl.querySelector<HTMLElement>(`[data-index="${selectedIndex}"]`);
    el?.scrollIntoView({ block: "nearest" });
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

  function glyphFor(id: string): string {
    if (id.startsWith("nav-home")) return "M4 10.5 12 4l8 6.5V20H4z";
    if (id.startsWith("nav-downloads")) return "M12 4v11m0 0 4-4m-4 4-4-4M5 20h14";
    if (id.startsWith("nav-settings")) return "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zm7-3 1.5-.9-1-1.7-1.7.5a6 6 0 0 0-1.4-1.4l.5-1.7-1.7-1L14.3 7a6 6 0 0 0-2-.5V4.8h-2v1.7a6 6 0 0 0-2 .5L6.9 5.8l-1.7 1 .5 1.7A6 6 0 0 0 4.3 10L2.6 9.5l-1 1.7L3 12l-1.4.9 1 1.7 1.7-.5a6 6 0 0 0 1.4 1.4l-.5 1.7 1.7 1L8.3 17a6 6 0 0 0 2 .5v1.7h2v-1.7a6 6 0 0 0 2-.5l1.4 1.2 1.7-1-.5-1.7a6 6 0 0 0 1.4-1.4l1.7.5 1-1.7L19 12z";
    if (id.startsWith("nav-marketplace")) return "M4 9h16l-1.2 10H5.2zM8 9V7a4 4 0 0 1 8 0v2";
    if (id.startsWith("nav-about")) return "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zm0-13v.01M12 11v6";
    if (id.startsWith("action-paste")) return "M9 5H7a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2h-2M9 5a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2M9 5a2 2 0 0 0 2 2h2a2 2 0 0 0 2-2";
    return "M6 12h12M12 6v12";
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="mac-command-backdrop" role="presentation" onclick={() => closeCommandPalette()}>
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="mac-command-palette" role="dialog" tabindex="-1" aria-label={$t("command_palette.open")} onclick={(e) => e.stopPropagation()}>
      <div class="mac-command-input-row">
        <svg viewBox="0 0 20 20" width="18" height="18" fill="currentColor" aria-hidden="true">
          <path d="M8.5 2.5a6 6 0 1 0 3.67 10.74l3.8 3.79a1 1 0 0 0 1.41-1.41l-3.79-3.8A6 6 0 0 0 8.5 2.5zM4.5 8.5a4 4 0 1 1 8 0 4 4 0 0 1-8 0z" />
        </svg>
        <input
          bind:this={inputEl}
          class="mac-command-input"
          type="search"
          placeholder={$t("command_palette.placeholder")}
          value={query}
          oninput={(e) => setCommandPaletteQuery((e.target as HTMLInputElement).value)}
          spellcheck="false"
          autocomplete="off"
          role="combobox"
          aria-expanded="true"
          aria-controls="command-palette-list"
          aria-autocomplete="list"
        />
        <span class="kbd">esc</span>
      </div>
      <div class="mac-command-list" id="command-palette-list" role="listbox" bind:this={listEl}>
        {#if filtered.length === 0}
          <div class="mac-command-empty">{$t("command_palette.empty")}</div>
        {:else if grouped}
          {#each grouped as [group, indexes] (group)}
            <div class="mac-command-group">{group}</div>
            {#each indexes as i (filtered[i].id)}
              {@const item = filtered[i]}
              <button
                type="button"
                class="mac-command-item"
                class:selected={i === selectedIndex}
                role="option"
                aria-selected={i === selectedIndex}
                data-index={i}
                onclick={() => runCommandPaletteSelected(filtered)}
                onmouseenter={() => setCommandPaletteSelectedIndex(i)}
              >
                <span class="mac-command-item-icon" aria-hidden="true">
                  <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d={glyphFor(item.id)} /></svg>
                </span>
                <span class="mac-command-item-label">{item.label}</span>
              </button>
            {/each}
          {/each}
        {:else}
          {#each filtered as item, i (item.id)}
            <button
              type="button"
              class="mac-command-item"
              class:selected={i === selectedIndex}
              role="option"
              aria-selected={i === selectedIndex}
              data-index={i}
              onclick={() => runCommandPaletteSelected(filtered)}
              onmouseenter={() => setCommandPaletteSelectedIndex(i)}
            >
              <span class="mac-command-item-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d={glyphFor(item.id)} /></svg>
              </span>
              <span class="mac-command-item-label">{item.label}</span>
              <span class="mac-command-item-group">{item.group}</span>
            </button>
          {/each}
        {/if}
      </div>
      <div class="mac-command-footer">
        <span><span class="kbd">↑</span><span class="kbd">↓</span> {$t("command_palette.navigate")}</span>
        <span><span class="kbd">↩</span> {$t("command_palette.select")}</span>
      </div>
    </div>
  </div>
{/if}
