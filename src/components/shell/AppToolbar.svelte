<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { openCommandPalette } from "$lib/stores/command-palette-store.svelte";
  import { getToolbar } from "$lib/stores/toolbar-store.svelte";
  import { shortcut } from "$lib/platform";

  let toolbar = $derived(getToolbar());

  let pageTitle = $derived.by(() => {
    const path = page.url.pathname;
    if (path === "/") return "";
    if (path.startsWith("/downloads")) return $t("nav.downloads");
    if (path.startsWith("/tools")) return $t("tools.hub.title");
    if (path.startsWith("/marketplace")) return $t("nav.marketplace");
    if (path.startsWith("/settings")) return $t("nav.settings");
    if (path.startsWith("/about")) return $t("nav.about");
    if (path.startsWith("/omnidisc")) return $t("nav.omnidisc");
    if (path.startsWith("/league")) return $t("league.nav");
    if (path.startsWith("/courses")) return $t("courses.title");
    if (path.startsWith("/convert")) return $t("convert.title");
    if (path.startsWith("/telegram")) return $t("telegram.title");
    if (path.startsWith("/study/music")) return $t("study.hub.music");
    if (path.startsWith("/study")) return $t("study.hub.title");
    if (path.startsWith("/misc/file-clip")) return $t("tools.catalog.video-clip.name");
    if (path.includes("/library")) return $t("study.hub.library");
    return "";
  });
</script>

<header class="mac-titlebar" data-tauri-drag-region aria-label={pageTitle || "OmniGet"}>
  <div class="mac-titlebar-leading" data-tauri-drag-region>
    {#if pageTitle}
      <h1 class="mac-titlebar-title" data-tauri-drag-region>{pageTitle}</h1>
    {:else}
      <span class="mac-titlebar-title mac-titlebar-title--quiet" data-tauri-drag-region></span>
    {/if}
  </div>

  <div class="mac-titlebar-center" data-tauri-drag-region>
    {#if toolbar.segments && toolbar.segments.length > 0}
      <div class="segmented" role="tablist">
        {#each toolbar.segments as seg (seg.id)}
          <button
            type="button"
            class="segmented-btn"
            class:active={toolbar.activeSegment === seg.id}
            role="tab"
            aria-selected={toolbar.activeSegment === seg.id}
            onclick={() => toolbar.onSegment?.(seg.id)}
          >
            {seg.label}
            {#if seg.count !== undefined && seg.count > 0}
              <span class="mac-nav-badge">{seg.count > 99 ? "99+" : seg.count}</span>
            {/if}
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <div class="mac-titlebar-actions">
    {#if toolbar.actions && toolbar.actions.length > 0}
      <div class="mac-toolbar-group">
        {#each toolbar.actions as action (action.id)}
          <button
            type="button"
            class="mac-toolbar-btn"
            class:active={action.active}
            class:prominent={action.prominent}
            disabled={action.disabled}
            onclick={action.onClick}
            title={action.label}
            aria-label={action.label}
            aria-pressed={action.active !== undefined ? action.active : undefined}
          >
            {#if action.icon}
              <svg
                viewBox="0 0 24 24"
                width="16"
                height="16"
                fill={action.iconFilled ? "currentColor" : "none"}
                stroke={action.iconFilled ? "none" : "currentColor"}
                stroke-width="1.8"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d={action.icon} />
              </svg>
            {/if}
            {#if action.showLabel || !action.icon}
              <span>{action.label}</span>
            {/if}
          </button>
        {/each}
      </div>
      <span class="mac-toolbar-sep" aria-hidden="true"></span>
    {/if}
    <button
      type="button"
      class="mac-search-field"
      onclick={() => openCommandPalette()}
      aria-label={$t("command_palette.open")}
      aria-keyshortcuts="Meta+K Control+K"
    >
      <svg class="mac-search-glyph" viewBox="0 0 20 20" width="13" height="13" fill="currentColor" aria-hidden="true">
        <path d="M8.5 2.5a6 6 0 1 0 3.67 10.74l3.8 3.79a1 1 0 0 0 1.41-1.41l-3.79-3.8A6 6 0 0 0 8.5 2.5zM4.5 8.5a4 4 0 1 1 8 0 4 4 0 0 1-8 0z" />
      </svg>
      <span class="mac-search-label">{$t("command_palette.open")}</span>
      <span class="kbd">{shortcut("K")}</span>
    </button>
  </div>
</header>
