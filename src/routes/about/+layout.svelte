<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import type { Snippet } from "svelte";

  let { children }: { children: Snippet } = $props();

  const tabs = [
    { href: "/about/terms", labelKey: "about.tab.terms" },
    { href: "/about/project", labelKey: "about.tab.project" },
    { href: "/about/roadmap", labelKey: "about.tab.roadmap" },
  ] as const;

  function isActive(href: string): boolean {
    return page.url.pathname === href;
  }
</script>

<div class="about-layout">
  <nav class="about-tabs">
    {#each tabs as tab}
      <a
        href={tab.href}
        class="about-tab"
        class:active={isActive(tab.href)}
      >
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
    flex-direction: column;
    max-width: 520px;
    margin: 0 auto;
    padding-top: calc(var(--padding) * 1.5);
    gap: calc(var(--padding) * 1.5);
  }

  .about-tabs {
    display: flex;
    gap: 4px;
    background: var(--button);
    border-radius: var(--border-radius);
    padding: 4px;
    box-shadow: var(--button-box-shadow);
  }

  .about-tab {
    flex: 1;
    text-align: center;
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 13px;
    font-weight: 500;
    color: var(--gray);
    border-radius: calc(var(--border-radius) - 2px);
    cursor: pointer;
    user-select: none;
    text-decoration: none;
  }

  @media (hover: hover) {
    .about-tab:hover:not(.active) {
      color: var(--secondary);
      background: var(--button-elevated);
    }
  }

  .about-tab.active {
    background: var(--secondary);
    color: var(--primary);
    cursor: default;
  }

  .about-tab:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .about-content {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
    padding-bottom: calc(var(--padding) * 3);
  }
</style>
