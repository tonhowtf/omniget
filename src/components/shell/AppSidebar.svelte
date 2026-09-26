<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import Avatar from "$components/omni/Avatar.svelte";
  import { getProfile, loadProfile } from "$lib/stores/profile-store.svelte";
  import type { NavItem } from "$lib/nav-config";

  interface Props {
    primaryNav?: NavItem[];
    appNav?: NavItem[];
    pluginNav?: NavItem[];
    badgeLabel?: string;
    badgeCount?: number;
  }

  let {
    primaryNav = [],
    appNav = [],
    pluginNav = [],
    badgeLabel = "",
    badgeCount = 0,
  }: Props = $props();


  // Loads once per session and never throws: while `profile_*` answers ERR_STUB
  // the store stays null and the footer simply does not render.
  loadProfile();
  let profile = $derived(getProfile());

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
    class:help-link={item.href === "/help"}
    class:active
    title={title}
    aria-current={active ? "page" : undefined}
  >
    <NavIcon icon={item.icon} iconSvg={item.iconSvg} size={20} {active} />
    <span class="mac-nav-label">{title}{#if item.href === "/help"}<small>{$t("nav.help_subtitle")}</small>{/if}</span>
    {#if item.badge === "downloads" && badgeCount > 0}
      <span class="mac-nav-badge live">{badgeLabel}</span>
    {/if}
  </a>
{/snippet}

<aside class="mac-source-list" aria-label={$t("nav.section_primary")}>
  <div class="mac-source-list-drag" data-tauri-drag-region></div>

  <div class="sidebar-navigation">
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

  </div>
  {#if profile}
    <a class="mac-sidebar-profile" href="/settings?tab=profile" title={$t("omni.sidebar_profile")}>
      <Avatar tint={profile.skin?.tint} size={32} compact animate="never" />
      <span class="profile-copy"><span class="mac-sidebar-profile-nick">{profile.nickname}</span><small>{$t("nav.profile_appearance")}</small></span>
    </a>
  {/if}
</aside>

<style>
  .sidebar-navigation { flex: 1; min-height: 0; overflow-y: auto; scrollbar-width: thin; }
  :global(.mac-source-list) { overflow-y: hidden; }
  @media (max-width: 860px) { .profile-copy { display: none; } .mac-sidebar-profile { padding-inline: 8px; } }
  .help-link { background: var(--accent-soft); min-height: 44px; }
  .mac-nav-label small, .profile-copy small { display: block; font-size: 11px; line-height: 1.3; color: var(--text-muted); }
  .profile-copy { display: flex; flex-direction: column; min-width: 0; }
  .mac-sidebar-profile { min-height: 44px; box-sizing: border-box; }
  @media (max-width: 900px) { .help-link .mac-nav-label small { display: none; } }

  .mac-sidebar-profile {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: auto;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    color: var(--text-dim);
    text-decoration: none;
    min-width: 0;
  }

  .mac-sidebar-profile:hover {
    background: var(--fill-quaternary, rgba(127, 127, 127, 0.12));
    color: var(--text);
  }

  .mac-sidebar-profile-nick {
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mac-sidebar-profile:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
</style>
