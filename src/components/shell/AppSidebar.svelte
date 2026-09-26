<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import Avatar from "$components/omni/Avatar.svelte";
  import { getProfile, loadProfile } from "$lib/stores/profile-store.svelte";
  import { NAV_ALIASES, type NavItem } from "$lib/nav-config";
  import { isMac } from "$lib/platform";

  interface Props {
    primaryNav?: NavItem[];
    appNav?: NavItem[];
    badgeLabel?: string;
    badgeCount?: number;
    /** Icon-only rail; the state lives in settings.appearance.sidebar_collapsed. */
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

  // Rail tooltip. Native `title` waits ~1s and cannot be styled, so the rail
  // draws its own: 400 ms before the first one, then instant while the pointer
  // moves along the column (the "warm" window), like macOS toolbars.
  let tip = $state<{ text: string; top: number; left: number } | null>(null);
  let tipTimer: ReturnType<typeof setTimeout> | undefined;
  let warmTimer: ReturnType<typeof setTimeout> | undefined;
  let warm = false;

  function showTip(event: MouseEvent | FocusEvent, text: string) {
    if (!collapsed) return;
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
    if (!collapsed) tip = null;
  });


  // Loads once per session and never throws: while `profile_*` answers ERR_STUB
  // the store stays null and the footer simply does not render.
  loadProfile();
  let profile = $derived(getProfile());

  function isActive(href: string): boolean {
    if (href === "/") return page.url.pathname === "/";
    const path = page.url.pathname;
    return path.startsWith(href) || (NAV_ALIASES[href] ?? []).some((alias) => path.startsWith(alias));
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
    title={collapsed ? undefined : title}
    aria-label={item.badge === "downloads" && badgeCount > 0 ? `${title} (${badgeLabel})` : title}
    aria-current={active ? "page" : undefined}
    onmouseenter={(e) => showTip(e, title)}
    onmouseleave={hideTip}
    onfocus={(e) => showTip(e, title)}
    onblur={hideTip}
  >
    <NavIcon icon={item.icon} size={collapsed ? 30 : 26} {active} />
    <span class="mac-nav-label">{title}{#if item.href === "/help"}<small>{$t("nav.help_subtitle")}</small>{/if}</span>
    {#if item.badge === "downloads" && badgeCount > 0}
      <span class="mac-nav-badge live">{badgeLabel}</span>
    {/if}
  </a>
{/snippet}

<aside class="mac-source-list" class:collapsed aria-label={$t("nav.section_primary")}>
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
    onclick={() => onToggleCollapsed?.()}
    aria-label={collapsed ? $t("nav.expand_sidebar") : $t("nav.collapse_sidebar")}
    aria-keyshortcuts={isMac() ? "Control+Meta+S" : "Control+Shift+S"}
    aria-expanded={!collapsed}
    onmouseenter={(e) => showTip(e, `${$t("nav.expand_sidebar")} ${shortcutLabel}`)}
    onmouseleave={hideTip}
    onfocus={(e) => showTip(e, `${$t("nav.expand_sidebar")} ${shortcutLabel}`)}
    onblur={hideTip}
  >
    <svg viewBox="0 0 20 20" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <rect x="2.5" y="3.5" width="15" height="13" rx="3" />
      <path d="M7.5 3.5v13" />
      {#if collapsed}
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
