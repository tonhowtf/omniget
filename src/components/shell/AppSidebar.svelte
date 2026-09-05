<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import type { NavItem } from "$lib/nav-config";

  interface Props {
    primaryNav?: NavItem[];
    appNav?: NavItem[];
    pluginNav?: NavItem[];
    badgeLabel?: string;
    badgeCount?: number;
    chatBadgeCount?: number;
  }

  let {
    primaryNav = [],
    appNav = [],
    pluginNav = [],
    badgeLabel = "",
    badgeCount = 0,
    chatBadgeCount = 0,
  }: Props = $props();

  let chatBadgeLabel = $derived(chatBadgeCount > 99 ? "99+" : String(chatBadgeCount));

  const PLUGINS_KEY = "omniget.sidebar.plugins_expanded";
  let pluginsExpanded = $state(
    typeof localStorage === "undefined" ? true : localStorage.getItem(PLUGINS_KEY) !== "0",
  );

  function togglePlugins() {
    pluginsExpanded = !pluginsExpanded;
    try {
      localStorage.setItem(PLUGINS_KEY, pluginsExpanded ? "1" : "0");
    } catch {}
  }

  function isActive(href: string): boolean {
    if (href === "/") return page.url.pathname === "/";
    return page.url.pathname.startsWith(href);
  }

  function itemTitle(item: NavItem): string {
    return item.label || (item.labelKey ? $t(item.labelKey) : "");
  }
</script>

{#snippet navLink(item: NavItem)}
  {@const title = itemTitle(item)}
  {@const active = isActive(item.href)}
  <a
    href={item.href}
    class="mac-nav-item"
    class:active
    title={title}
    aria-current={active ? "page" : undefined}
  >
    <NavIcon icon={item.icon} iconSvg={item.iconSvg} size={18} {active} />
    <span class="mac-nav-label">{title}</span>
    {#if item.badge === "downloads" && badgeCount > 0}
      <span class="mac-nav-badge live">{badgeLabel}</span>
    {:else if item.badge === "omnidisc" && chatBadgeCount > 0}
      <span class="mac-nav-badge live">{chatBadgeLabel}</span>
    {/if}
  </a>
{/snippet}

<aside class="mac-source-list" aria-label={$t("nav.section_primary")}>
  <div class="mac-source-list-drag" data-tauri-drag-region></div>

  <nav class="mac-nav-section" aria-label={$t("nav.section_primary")}>
    {#each primaryNav as item (item.href)}
      {@render navLink(item)}
    {/each}
  </nav>

  <nav class="mac-nav-section" aria-label={$t("nav.section_app")}>
    <div class="mac-nav-section-header">{$t("nav.section_app")}</div>
    {#each appNav as item (item.href)}
      {@render navLink(item)}
    {/each}
  </nav>

  {#if pluginNav.length > 0}
    <nav class="mac-nav-section" aria-label={$t("nav.section_plugins")}>
      <div class="mac-nav-section-header">
        <span>{$t("nav.section_plugins")}</span>
        <button
          type="button"
          class="mac-plugins-toggle"
          onclick={togglePlugins}
          aria-expanded={pluginsExpanded}
          aria-label={pluginsExpanded ? $t("nav.collapse_plugins") : $t("nav.expand_plugins")}
        >
          <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            {#if pluginsExpanded}
              <path d="M4 6l4 4 4-4" />
            {:else}
              <path d="M6 4l4 4-4 4" />
            {/if}
          </svg>
        </button>
      </div>
      {#if pluginsExpanded}
        {#each pluginNav as item (item.href)}
          {@render navLink(item)}
        {/each}
      {/if}
    </nav>
  {/if}
</aside>
