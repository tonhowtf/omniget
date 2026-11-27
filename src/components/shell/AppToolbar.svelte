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
    if (path.startsWith("/global")) return $t("nav.global");
    if (path.startsWith("/superpowers")) return $t("nav.superpowers");
    if (path.startsWith("/settings")) return $t("nav.settings");
    if (path.startsWith("/community")) return $t("nav.community");
    if (path.startsWith("/league")) return $t("league.nav");
    if (path.startsWith("/music")) return $t("music.title");
    if (path.startsWith("/video")) return $t("video.title");
    if (path.startsWith("/youtube")) return $t("tube.title");
    if (path.startsWith("/washmybelly")) return $t("washmybelly.nav");
    if (path.startsWith("/read")) return $t("read.title");
    if (path.startsWith("/converter")) return $t("converter.title");
    if (path.startsWith("/pdf")) return $t("pdf.title");
    if (path.startsWith("/photos")) return $t("photos.title");
    if (path.startsWith("/cutout")) return $t("cutout.title");
    if (path.startsWith("/guildchat")) return $t("guildchat.title");
    if (path === "/shortpost") return $t("social.shortpost.title");
    if (path.startsWith("/photofeed")) return $t("social.photofeed.title");
    if (path.startsWith("/clipfeed")) return $t("social.clipfeed.title");
    if (path.startsWith("/forums")) return $t("social.forums.title");
    if (path.startsWith("/pinboard")) return $t("social.pinboard.title");
    if (path.startsWith("/codehub")) return $t("social.codehub.title");
    if (path.startsWith("/phonechat")) return $t("phonechat.title");
    if (path.startsWith("/dictation")) return $t("dictation.title");
    if (path.startsWith("/studio")) return $t("studio.title");
    if (path.startsWith("/capture")) return $t("capture.title");
    if (path.startsWith("/components")) return $t("components.title");
    if (path.startsWith("/board")) return $t("board.title");
    if (path.startsWith("/motion")) return $t("motion.title");
    if (path.startsWith("/icons")) return $t("icons.title");
    if (path.startsWith("/fonts")) return $t("fonts.title");
    if (path.startsWith("/palette")) return $t("palette.title");
    if (path.startsWith("/devtools")) return $t("devtools.title");
    if (path.startsWith("/pack")) return $t("pack.title");
    if (path.startsWith("/scrape")) return $t("scrape.title");
    if (/^\/api(\/|$)/.test(path)) return $t("api.title");
    if (path.startsWith("/transfer")) return $t("transfer.title");
    if (path.startsWith("/research")) return $t("research.title");
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
