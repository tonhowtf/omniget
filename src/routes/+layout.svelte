<script lang="ts">
  import "../app.css";
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { initDownloadListener } from "$lib/stores/download-listener";
  import { getActiveCount } from "$lib/stores/download-store.svelte";
  import { loadSettings, getSettings } from "$lib/stores/settings-store.svelte";
  import Toast from "$components/toast/Toast.svelte";
  import { open } from "@tauri-apps/plugin-shell";
  import { refreshUpdateInfo } from "$lib/stores/update-store.svelte";
  import { startClipboardMonitor, stopClipboardMonitor } from "$lib/stores/clipboard-monitor";
  import { initChangelog } from "$lib/stores/changelog-store.svelte";
  import ChangelogDialog from "$components/dialog/ChangelogDialog.svelte";
  import { t } from "$lib/i18n";
  import type { Snippet } from "svelte";

  let ytdlpMissing = $state(false);
  let ytdlpDismissed = $state(false);

  async function openAuthorGithub(e: Event) {
    e.preventDefault();
    await open("https://github.com/tonhowtf");
  }

  let activeCount = $derived(getActiveCount());
  let settings = $derived(getSettings());

  $effect(() => {
    if (settings?.download.clipboard_detection) {
      startClipboardMonitor();
    } else {
      stopClipboardMonitor();
    }
    return () => {
      stopClipboardMonitor();
    };
  });

  let { children }: { children: Snippet } = $props();

  onMount(() => {
    let cleanup: (() => void) | undefined;
    initDownloadListener().then((fn) => (cleanup = fn));
    loadSettings();
    refreshUpdateInfo();
    initChangelog();
    invoke<boolean>("check_ytdlp_available").then((ok) => { ytdlpMissing = !ok; }).catch(() => {});

    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const handleChange = () => {
      const s = getSettings();
      if (s?.appearance.theme === "system") {
        document.documentElement.setAttribute("data-theme", mediaQuery.matches ? "dark" : "light");
      }
    };
    mediaQuery.addEventListener("change", handleChange);

    return () => {
      cleanup?.();
      mediaQuery.removeEventListener("change", handleChange);
    };
  });

  const nav = [
    { href: "/", labelKey: "nav.home", icon: "home" },
    { href: "/downloads", labelKey: "nav.downloads", icon: "downloads" },
    { href: "/convert", labelKey: "nav.convert", icon: "convert" },
    { href: "/hotmart", labelKey: "nav.hotmart", icon: "hotmart" },
    { href: "/udemy", labelKey: "nav.udemy", icon: "udemy" },
    { href: "/telegram", labelKey: "nav.telegram", icon: "telegram" },
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
          {:else if item.icon === "convert"}
            <path d="M20 10H4l4-4" />
            <path d="M4 14h16l-4 4" />
          {:else if item.icon === "hotmart"}
            <path d="M6 4v16" />
            <path d="M18 4v16" />
            <path d="M6 12h12" />
          {:else if item.icon === "udemy"}
            <path d="M22 9l-10 -4l-10 4l10 4l10 -4v6" />
            <path d="M6 10.6v5.4a6 3 0 0 0 12 0v-5.4" />
          {:else if item.icon === "telegram"}
            <path d="M21 5L2 12.5l7 1M21 5l-5.5 15-4.5-7.5M21 5L9 13.5" />
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
    {#if ytdlpMissing && !ytdlpDismissed}
      <div class="ytdlp-banner">
        <span class="ytdlp-banner-text">{$t('common.ytdlp_missing')}</span>
        <button class="button ytdlp-banner-link" onclick={() => goto('/settings#dependencies')}>
          {$t('common.go_to_settings')}
        </button>
        <button class="ytdlp-banner-close" onclick={() => ytdlpDismissed = true}>
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </div>
    {/if}
    {@render children()}
    <a
      href="https://github.com/tonhowtf"
      class="watermark"
      onclick={openAuthorGithub}
      title="@tonhowtf"
    >
      @tonhowtf
    </a>
  </main>
</div>

<Toast />
<ChangelogDialog />

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

  .watermark {
    position: fixed;
    bottom: 8px;
    right: 12px;
    font-size: 10px;
    font-weight: 400;
    color: var(--gray);
    opacity: 0.3;
    pointer-events: auto;
    cursor: pointer;
    z-index: 1;
    user-select: none;
    transition: opacity 0.15s;
  }

  @media (hover: hover) {
    .watermark:hover {
      opacity: 0.7;
    }
  }

  .ytdlp-banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    margin-bottom: var(--padding);
    background: var(--orange);
    color: #000;
    border-radius: var(--border-radius);
    font-size: 12.5px;
    font-weight: 500;
  }

  .ytdlp-banner-text {
    flex: 1;
  }

  .ytdlp-banner-link {
    background: rgba(0, 0, 0, 0.15);
    color: #000;
    border: none;
    font-size: 12px;
    padding: 4px 10px;
    border-radius: calc(var(--border-radius) - 4px);
    cursor: pointer;
    white-space: nowrap;
    box-shadow: none;
  }

  @media (hover: hover) {
    .ytdlp-banner-link:hover {
      background: rgba(0, 0, 0, 0.25);
    }
  }

  .ytdlp-banner-close {
    background: none;
    border: none;
    color: #000;
    cursor: pointer;
    padding: 2px;
    opacity: 0.6;
    display: flex;
    align-items: center;
  }

  @media (hover: hover) {
    .ytdlp-banner-close:hover {
      opacity: 1;
    }
  }
</style>
