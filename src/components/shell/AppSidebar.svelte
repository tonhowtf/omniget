<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import NavIcon from "#components/shell/NavIcon.svelte";
  import Avatar from "#components/omni/Avatar.svelte";
  import { getProfile, loadProfile } from "$lib/stores/profile-store.svelte";
  import { activeNavHref, type NavItem } from "$lib/nav-config";
  import { isMac } from "$lib/platform";

  interface Props {
    primaryNav?: NavItem[];
    appNav?: NavItem[];
    badgeLabel?: string;
    badgeCount?: number;
    collapsed?: boolean;
    onToggleCollapsed?: () => void;
  }

  let {
    primaryNav = [],
    appNav = [],
    badgeLabel = "",
    badgeCount = 0,
    collapsed = false,
    onToggleCollapsed,
  }: Props = $props();

  const shortcutLabel = isMac() ? "⌃⌘S" : "Ctrl+Shift+S";

  const AUTO_COLLAPSE_BELOW = 1180;
  let viewportWidth = $state(typeof window === "undefined" ? AUTO_COLLAPSE_BELOW : window.innerWidth);
  let denseRoute = $derived(/^\/settings(\/|$)/.test(page.url.pathname));
  let autoCollapsed = $derived(!collapsed && denseRoute && viewportWidth < AUTO_COLLAPSE_BELOW);
  let drawerOpen = $state(false);
  let drawer = $derived(autoCollapsed && drawerOpen);
  let railCollapsed = $derived(collapsed || (autoCollapsed && !drawerOpen));
  let collapseButton = $state<HTMLButtonElement | null>(null);
  let aside = $state<HTMLElement | null>(null);

  $effect(() => {
    if (!autoCollapsed) drawerOpen = false;
  });
  $effect(() => {
    void page.url.pathname;
    drawerOpen = false;
  });

  function closeDrawer(returnFocus: boolean) {
    if (!drawerOpen) return;
    drawerOpen = false;
    if (returnFocus) collapseButton?.focus();
  }

  function onToggle() {
    if (autoCollapsed) {
      drawerOpen = !drawerOpen;
      return;
    }
    onToggleCollapsed?.();
  }

  function onWindowKeydown(event: KeyboardEvent) {
    if (drawer && event.key === "Escape" && !event.defaultPrevented) {
      event.preventDefault();
      closeDrawer(true);
    }
  }

  function onWindowPointerdown(event: PointerEvent) {
    if (drawer && aside && !aside.contains(event.target as Node)) closeDrawer(false);
  }

  let tip = $state<{ text: string; top: number; left: number } | null>(null);
  let tipTimer: ReturnType<typeof setTimeout> | undefined;
  let warmTimer: ReturnType<typeof setTimeout> | undefined;
  let warm = false;

  function showTip(event: MouseEvent | FocusEvent, text: string) {
    if (!railCollapsed) return;
    const el = event.currentTarget as HTMLElement;
    const place = () => {
      const r = el.getBoundingClientRect();
      tip = { text, top: r.top + r.height / 2, left: r.right + 8 };
      warm = true;
    };
    clearTimeout(tipTimer);
    clearTimeout(warmTimer);
    if (warm || event.type === "focus") place();
    else tipTimer = setTimeout(place, 400);
  }

  function hideTip() {
    clearTimeout(tipTimer);
    tip = null;
    warmTimer = setTimeout(() => (warm = false), 300);
  }

  $effect(() => {
    if (!railCollapsed) tip = null;
  });


  loadProfile();
  let profile = $derived(getProfile());

  let activeHref = $derived(activeNavHref([...primaryNav, ...appNav], page.url.pathname));

  function itemTitle(item: NavItem): string {
    return item.label || (item.labelKey ? $t(item.labelKey) : "");
  }
</script>

{#snippet navLink(item: NavItem)}
  {@const title = itemTitle(item)}
  {@const active = item.href === activeHref}
  <a
    href={item.href}
    class="mac-nav-item"
    class:help-link={item.href === "/help"}
    class:pinned-link={item.pinned}
    class:active
    title={railCollapsed ? undefined : title}
    aria-label={item.badge === "downloads" && badgeCount > 0 ? `${title} (${badgeLabel})` : title}
    aria-current={active ? "page" : undefined}
    onmouseenter={(e) => showTip(e, title)}
    onmouseleave={hideTip}
    onfocus={(e) => showTip(e, title)}
    onblur={hideTip}
  >
    <NavIcon icon={item.icon} size={railCollapsed ? 30 : 26} {active} />
    <span class="mac-nav-label">{title}</span>
    {#if item.badge === "downloads" && badgeCount > 0}
      <span class="mac-nav-badge live">{badgeLabel}</span>
    {/if}
  </a>
{/snippet}

<svelte:window bind:innerWidth={viewportWidth} onkeydown={onWindowKeydown} onpointerdown={onWindowPointerdown} />

{#if drawer}<div class="sidebar-drawer-spacer" aria-hidden="true"></div>{/if}
<aside
  bind:this={aside}
  class="mac-source-list"
  class:collapsed={railCollapsed}
  class:auto-collapsed={autoCollapsed && !drawerOpen}
  class:sidebar-drawer={drawer}
  aria-label={$t("nav.section_primary")}
>
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

  </div>
  <button
    type="button"
    class="mac-sidebar-collapse"
    bind:this={collapseButton}
    onclick={onToggle}
    aria-label={railCollapsed ? $t("nav.expand_sidebar") : $t("nav.collapse_sidebar")}
    aria-keyshortcuts={autoCollapsed ? undefined : isMac() ? "Control+Meta+S" : "Control+Shift+S"}
    aria-expanded={!railCollapsed}
    onmouseenter={(e) => showTip(e, `${$t("nav.expand_sidebar")} ${shortcutLabel}`)}
    onmouseleave={hideTip}
    onfocus={(e) => showTip(e, `${$t("nav.expand_sidebar")} ${shortcutLabel}`)}
    onblur={hideTip}
  >
    <svg viewBox="0 0 20 20" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <rect x="2.5" y="3.5" width="15" height="13" rx="3" />
      <path d="M7.5 3.5v13" />
      {#if railCollapsed}
        <path d="M11 8l2 2-2 2" />
      {:else}
        <path d="M13 8l-2 2 2 2" />
      {/if}
    </svg>
    <span class="mac-sidebar-collapse-label">{$t("nav.collapse_sidebar")}</span>
    <kbd class="mac-sidebar-collapse-kbd">{shortcutLabel}</kbd>
  </button>
  {#if profile}
    <a class="mac-sidebar-profile" href="/settings?tab=profile" title={$t("omni.sidebar_profile")}>
      <Avatar tint={profile.skin?.tint} size={32} compact animate="never" />
      <span class="profile-copy"><span class="mac-sidebar-profile-nick">{profile.nickname}</span><small>{$t("nav.profile_appearance")}</small></span>
    </a>
  {/if}
</aside>

{#if tip}
  <div class="mac-rail-tip" role="tooltip" style:top="{tip.top}px" style:left="{tip.left}px">{tip.text}</div>
{/if}

<style>
  .mac-sidebar-collapse {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 32px;
    margin-top: var(--space-2);
    padding: 0 var(--space-2) 0 12px;
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
    flex-shrink: 0;
  }
  .mac-sidebar-collapse:hover { background: var(--fill-1); color: var(--text); }
  .mac-sidebar-collapse:focus-visible { outline: var(--focus-ring); outline-offset: -2px; }
  .mac-sidebar-collapse-label { flex: 1; text-align: left; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .mac-sidebar-collapse-kbd {
    font: inherit;
    font-size: var(--text-xs);
    color: var(--text-muted);
    padding: 1px 5px;
    border-radius: var(--radius-xs);
    background: var(--fill-2);
  }
  .auto-collapsed { --sidebar-width: 64px; }
  .auto-collapsed .mac-nav-label,
  .auto-collapsed .mac-nav-badge { display: none; }
  .auto-collapsed .mac-nav-section-header { font-size: 0; min-height: 0; height: 1px; width: 24px; margin: 0 auto var(--space-2); padding: 0; background: var(--separator, var(--content-border)); }
  .auto-collapsed .mac-nav-item { justify-content: center; padding: 0; height: 44px; }
  .sidebar-drawer-spacer { width: 64px; min-width: 64px; flex-shrink: 0; }
  .sidebar-drawer {
    --sidebar-width: 224px;
    position: fixed;
    inset-block: 0;
    inset-inline-start: 0;
    z-index: 900;
    background: var(--surface);
    box-shadow: 0 8px 28px rgba(var(--shadow-ink), 0.28);
  }
  .collapsed .mac-sidebar-collapse { justify-content: center; padding: 0; }
  .collapsed .mac-sidebar-collapse-label,
  .collapsed .mac-sidebar-collapse-kbd { display: none; }
  .collapsed .mac-sidebar-profile { justify-content: center; padding-inline: 0; }
  .collapsed .profile-copy { display: none; }

  .mac-rail-tip {
    position: fixed;
    z-index: 1000;
    transform: translateY(-50%);
    padding: 5px 9px;
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
    pointer-events: none;
    box-shadow: 0 4px 14px rgba(var(--shadow-ink), 0.28), inset 0 0 0 0.5px var(--content-border);
    animation: rail-tip-in 90ms var(--ease-out, ease-out);
  }
  @keyframes rail-tip-in { from { opacity: 0; transform: translate(-3px, -50%); } }
  @media (prefers-reduced-motion: reduce) { .mac-rail-tip { animation: none; } }

  .sidebar-navigation { flex: 1; min-height: 0; overflow-y: auto; scrollbar-width: thin; }
  :global(.mac-source-list) { overflow-y: hidden; }
  @media (max-width: 860px) { .profile-copy { display: none; } .mac-sidebar-profile { padding-inline: 8px; } }
  .help-link { min-height: 44px; }
  .pinned-link { margin-inline-start: var(--space-3); }
  .collapsed .pinned-link,
  .auto-collapsed .pinned-link { margin-inline-start: 0; }
  @media (max-width: 860px) { .pinned-link { margin-inline-start: 0; } }
  .profile-copy small { display: block; font-size: 11px; line-height: 1.3; color: var(--text-muted); }
  .profile-copy { display: flex; flex-direction: column; min-width: 0; }
  .mac-sidebar-profile { min-height: 44px; box-sizing: border-box; }

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
