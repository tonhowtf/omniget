<script lang="ts">
  import "../app.css";
  import { page } from "$app/state";
  import { onMount } from "svelte";
  import { initDownloadListener } from "$lib/stores/download-listener";
  import { getActiveCount } from "$lib/stores/download-store.svelte";
  import { loadSettings } from "$lib/stores/settings-store.svelte";
  import Toast from "$components/toast/Toast.svelte";
  import { t, locale, loadTranslations, defaultLocale } from "$lib/i18n";
  import type { Snippet } from "svelte";

  let activeCount = $derived(getActiveCount());

  let { children }: { children: Snippet } = $props();

  onMount(() => {
    let cleanup: (() => void) | undefined;
    initDownloadListener().then((fn) => (cleanup = fn));
    loadSettings().then(async (settings) => {
      const lang = settings.appearance.language;
      if (lang && lang !== defaultLocale) {
        await loadTranslations(lang, window.location.pathname);
        locale.set(lang);
      }
    });
    return () => cleanup?.();
  });

  const nav = [
    { href: "/", labelKey: "nav.home", icon: "home" },
    { href: "/downloads", labelKey: "nav.downloads", icon: "downloads" },
    { href: "/hotmart", labelKey: "nav.hotmart", icon: "hotmart" },
    { href: "/settings", labelKey: "nav.settings", icon: "settings" },
    { href: "/about", labelKey: "nav.about", icon: "about" },
  ] as const;

  function isActive(href: string): boolean {
    if (href === "/") return page.url.pathname === "/";
    return page.url.pathname.startsWith(href);
  }
</script>

<div class="layout">
  <nav class="sidebar">
    {#each nav as item}
      <a
        href={item.href}
        class="nav-item"
        class:active={isActive(item.href)}
        title={$t(item.labelKey)}
      >
        <span class="indicator"></span>
        <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          {#if item.icon === "home"}
            <path d="M3 12L12 3l9 9" />
            <path d="M5 10v9a1 1 0 001 1h3v-5h6v5h3a1 1 0 001-1v-9" />
          {:else if item.icon === "downloads"}
            <path d="M12 3v12m0 0l-4-4m4 4l4-4" />
            <path d="M4 17v2a1 1 0 001 1h14a1 1 0 001-1v-2" />
          {:else if item.icon === "hotmart"}
            <path d="M6 4v16" />
            <path d="M18 4v16" />
            <path d="M6 12h12" />
          {:else if item.icon === "settings"}
            <circle cx="12" cy="12" r="3" />
            <path d="M12 1v2m0 18v2M4.22 4.22l1.42 1.42m12.72 12.72l1.42 1.42M1 12h2m18 0h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
          {:else if item.icon === "about"}
            <circle cx="12" cy="12" r="10" />
            <path d="M12 16v-4m0-4h.01" />
          {/if}
        </svg>
        {#if item.icon === "downloads" && activeCount > 0}
          <span class="badge">{activeCount}</span>
        {/if}
      </a>
    {/each}
  </nav>

  <main class="content">
    {@render children()}
  </main>
</div>

<Toast />

<style>
  .layout {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  .sidebar {
    width: var(--sidebar-width);
    min-width: var(--sidebar-width);
    height: 100vh;
    background: var(--sidebar-bg);
    display: flex;
    flex-direction: column;
    align-items: center;
    padding-top: var(--padding);
    gap: 4px;
  }

  .nav-item {
    position: relative;
    width: 44px;
    height: 44px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--border-radius);
    color: var(--gray);
  }

  @media (hover: hover) {
    .nav-item:hover {
      color: var(--secondary);
      background-color: var(--sidebar-highlight);
    }
  }

  .nav-item:active {
    background-color: var(--sidebar-highlight);
  }

  .nav-item:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .nav-item.active {
    color: var(--blue);
  }

  .indicator {
    position: absolute;
    left: -8px;
    width: 3px;
    height: 0;
    background: var(--blue);
    border-radius: 0 2px 2px 0;
    transition: height 0.15s;
  }

  .nav-item.active .indicator {
    height: 20px;
  }

  .badge {
    position: absolute;
    top: 4px;
    right: 2px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    font-size: 10px;
    font-weight: 500;
    line-height: 16px;
    text-align: center;
    color: #fff;
    background: var(--blue);
    border-radius: 50%;
    pointer-events: none;
  }

  .content {
    flex: 1;
    overflow-y: auto;
    padding: calc(var(--padding) * 2);
    box-shadow: inset 1px 0 0 0 var(--content-border);
  }
</style>
