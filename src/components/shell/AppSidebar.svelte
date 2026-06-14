<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import type { NavItem } from "$lib/nav-config";

  let {
    primaryNav,
    appNav,
    pluginNav,
    badgeLabel,
    badgeCount,
  }: {
    primaryNav: NavItem[];
    appNav: NavItem[];
    pluginNav: NavItem[];
    badgeLabel: string;
    badgeCount: number;
  } = $props();

  let pluginsExpanded = $state(true);

  function isActive(href: string): boolean {
    if (href === "/") return page.url.pathname === "/";
    return page.url.pathname.startsWith(href);
  }

  function itemTitle(item: NavItem): string {
    return item.label || (item.labelKey ? $t(item.labelKey) : "");
  }
</script>

<aside class="mac-source-list">
  <div class="mac-nav-section">
    <div class="mac-nav-section-header">{$t("nav.section_primary")}</div>
    {#each primaryNav as item}
      {@const title = itemTitle(item)}
      <a href={item.href} class="mac-nav-item" class:active={isActive(item.href)} title={title}>
        <NavIcon icon={item.icon} iconSvg={item.iconSvg} />
        <span class="mac-nav-label">{title}</span>
        {#if item.badge === "downloads" && badgeCount > 0}
          <span class="mac-nav-badge">{badgeLabel}</span>
        {/if}
      </a>
    {/each}
  </div>

  <div class="mac-nav-section">
    <div class="mac-nav-section-header">{$t("nav.section_app")}</div>
    {#each appNav as item}
      {@const title = itemTitle(item)}
      <a href={item.href} class="mac-nav-item" class:active={isActive(item.href)} title={title}>
        <NavIcon icon={item.icon} iconSvg={item.iconSvg} />
        <span class="mac-nav-label">{title}</span>
      </a>
    {/each}
  </div>

  {#if pluginNav.length > 0}
    <div class="mac-nav-section">
      <div class="mac-nav-section-header">
        <span>{$t("nav.section_plugins")}</span>
        <button
          type="button"
          class="mac-plugins-toggle"
          onclick={() => { pluginsExpanded = !pluginsExpanded; }}
          aria-expanded={pluginsExpanded}
          aria-label={pluginsExpanded ? $t("nav.collapse_plugins") : $t("nav.expand_plugins")}
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
            {#if pluginsExpanded}
              <path d="M6 9l6 6 6-6" />
            {:else}
              <path d="M9 6l6 6-6 6" />
            {/if}
          </svg>
        </button>
      </div>
      {#if pluginsExpanded}
        {#each pluginNav as item}
          {@const title = itemTitle(item)}
          <a href={item.href} class="mac-nav-item" class:active={isActive(item.href)} title={title}>
            <NavIcon icon={item.icon} iconSvg={item.iconSvg} />
            <span class="mac-nav-label">{title}</span>
          </a>
        {/each}
      {/if}
    </div>
  {/if}
</aside>
