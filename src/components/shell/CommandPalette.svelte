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
    const active = document.activeElement;
    if (active && active !== document.body && !active.closest(".mac-command-palette")) return;
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
    if (id.startsWith("nav-superpowers")) return "M13 2 4 14h7l-1 8 9-12h-7z";
    if (id.startsWith("nav-washmybelly")) return "M8 14a4 4 0 1 0 8 0 4 4 0 0 0-8 0zm9-8a2 2 0 1 0 4 0 2 2 0 0 0-4 0zM4 7a3 3 0 1 0 6 0 3 3 0 0 0-6 0z";
    if (id.startsWith("nav-converter")) return "M4 8h13l-3-3M20 16H7l3 3";
    if (id.startsWith("nav-pdf")) return "M6 3h8l4 4v14H6zM14 3v4h4M9 13h6M9 17h4";
    if (id.startsWith("nav-photos")) return "M4 5h16v14H4zM4 16l5-5 4 4 2-2 5 5M15.5 9.5h.01";
    if (id.startsWith("nav-cutout")) return "M6 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM8.1 7.6 20 17M8.1 16.4 20 7";
    if (id.startsWith("nav-music-stems")) return "M16 3h5v5M8 3H3v5M12 22v-8.3a4 4 0 0 0-1.172-2.872L3 3m12 9 6-6";
    if (id.startsWith("nav-palette")) return "M12 3a9 9 0 0 0 0 18c1.1 0 1.6-.9 1.2-1.8-.5-1 .2-2.2 1.3-2.2H17a4 4 0 0 0 4-4c0-5.5-4-10-9-10z";
    if (id.startsWith("nav-fonts")) return "M4 19 9.5 5h1L16 19M6.3 14h7.4M17 19v-6";
    if (id.startsWith("nav-icons")) return "M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM17 14v6M14 17h6";
    if (id.startsWith("nav-motion")) return "M3 19C10 19 12 5 21 5";
    if (id.startsWith("nav-board")) return "M3 5h18v11H3zM8 20l4-4 4 4";
    if (id.startsWith("nav-components")) return "M12 2l4 4-4 4-4-4zM6 8l4 4-4 4-4-4zM18 8l4 4-4 4-4-4zM12 14l4 4-4 4-4-4z";
    if (id.startsWith("nav-capture")) return "M12 16a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z";
    if (id.startsWith("nav-studio")) return "M4 5h16v14H4zM8 5v14M16 5v14M4 9h4M4 15h4M16 9h4M16 15h4";
    if (id.startsWith("nav-dictation")) return "M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM5 11a7 7 0 0 0 14 0M12 18v3";
    if (id.startsWith("nav-whatsapp")) return "M12 3a9 9 0 0 0-7.8 13.5L3 21l4.6-1.2A9 9 0 1 0 12 3z";
    if (id.startsWith("nav-discord")) return "M8 4c-2 .4-3.5 1-4.5 1.8C2 9 1.5 12.5 2 16c1.3 1.3 3 2.2 4.8 2.7l1-1.7M16 4c2 .4 3.5 1 4.5 1.8 1.5 3.2 2 6.7 1.5 10.2-1.3 1.3-3 2.2-4.8 2.7l-1-1.7M7 16.5c3.3 1.3 6.7 1.3 10 0M9 12h.01M15 12h.01";
    if (id.startsWith("nav-social-x")) return "M4 4l16 16M20 4 4 20";
    if (id.startsWith("nav-social-instagram")) return "M7 3h10a4 4 0 0 1 4 4v10a4 4 0 0 1-4 4H7a4 4 0 0 1-4-4V7a4 4 0 0 1 4-4zM12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM17.5 6.5h.01";
    if (id.startsWith("nav-social-tiktok")) return "M14 3v11.5a3.5 3.5 0 1 1-3.5-3.5M14 3c.4 2.6 2.4 4.6 5 5";
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
