<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { isDebugEnabled } from "$lib/stores/debug-store.svelte";
  import type { Snippet } from "svelte";

  let { children }: { children: Snippet } = $props();

  const ALL_TABS = [
    { href: "/about", labelKey: "about.tab.overview", icon: "overview" },
    { href: "/about/changelog", labelKey: "about.tab.changelog", icon: "changelog" },
    { href: "/about/project", labelKey: "about.tab.project", icon: "project" },
    { href: "/about/terms", labelKey: "about.tab.terms", icon: "terms" },
    { href: "/about/debug", labelKey: "about.tab.debug", icon: "debug" },
  ] as const;

  let visibleTabs = $derived(
    isDebugEnabled() ? ALL_TABS : ALL_TABS.filter((tab) => tab.href !== "/about/debug")
  );

  function isActive(href: string): boolean {
    if (href === "/about") {
      return page.url.pathname === "/about";
    }
    return page.url.pathname === href || page.url.pathname.startsWith(`${href}/`);
  }
</script>

<div class="about-layout">
  <nav class="about-sidebar" aria-label={$t("nav.about")}>
    {#each visibleTabs as tab}
      <a
        href={tab.href}
        class="about-nav-item"
        class:active={isActive(tab.href)}
      >
        <span class="about-nav-icon" aria-hidden="true">
          {#if tab.icon === "overview"}
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>
          {:else if tab.icon === "changelog"}
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/></svg>
          {:else if tab.icon === "project"}
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/></svg>
          {:else if tab.icon === "terms"}
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>
          {:else}
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>
          {/if}
        </span>
        {$t(tab.labelKey)}
      </a>
    {/each}
  </nav>

  <div class="about-content">
    {@render children()}
  </div>
</div>

<style>
  .about-layout {
    display: flex;
    gap: var(--space-6);
    max-width: 880px;
    margin: 0 auto;
    padding: var(--space-5) var(--space-6) var(--space-8);
    align-items: flex-start;
  }

  .about-sidebar {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 176px;
    flex-shrink: 0;
    position: sticky;
    top: var(--space-5);
  }

  .about-nav-item {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 30px;
    padding: 0 var(--space-2);
    font-size: var(--text-base);
    font-weight: 400;
    color: var(--text);
    border-radius: var(--radius-sm);
    text-decoration: none;
    transition: background var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .about-nav-item:hover:not(.active) {
      background: var(--fill-1);
    }
  }

  .about-nav-item.active {
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 500;
    cursor: default;
  }

  .about-nav-item:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }

  .about-nav-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    background: var(--fill-2);
    color: var(--text-muted);
    flex-shrink: 0;
  }

  .about-nav-icon svg {
    width: 13px;
    height: 13px;
  }

  .about-nav-item.active .about-nav-icon {
    background: color-mix(in srgb, var(--on-accent) 14%, transparent);
    color: inherit;
  }

  .about-content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  @media (max-width: 640px) {
    .about-layout {
      flex-direction: column;
    }

    .about-sidebar {
      width: 100%;
      position: static;
      flex-direction: row;
      flex-wrap: wrap;
    }

    .about-nav-item {
      flex: 1 1 auto;
    }
  }
</style>
