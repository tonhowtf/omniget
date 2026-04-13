<script lang="ts">
  import "../app.css";
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { initDownloadListener } from "$lib/stores/download-listener";
  import { getCounts } from "$lib/stores/download-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { queueExternalPrefill, type ExternalUrlEvent } from "$lib/stores/external-url-store.svelte";
  import Toast from "$components/toast/Toast.svelte";
  import DebugPanel from "$components/debug/DebugPanel.svelte";
  import { open } from "@tauri-apps/plugin-shell";
  import { refreshUpdateInfo } from "$lib/stores/update-store.svelte";
  import { startClipboardMonitor, stopClipboardMonitor } from "$lib/stores/clipboard-monitor";
  import { initChangelog } from "$lib/stores/changelog-store.svelte";
  import ChangelogDialog from "$components/dialog/ChangelogDialog.svelte";
  import OnboardingWizard from "$components/onboarding/OnboardingWizard.svelte";
  import { needsOnboarding } from "$lib/stores/onboarding-store.svelte";
  import { isYtdlpAvailable, isDepsChecked, refreshYtdlpStatus } from "$lib/stores/dependency-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t, locale } from "$lib/i18n";
  import { get } from "svelte/store";
  import { CORE_NAV_ITEMS, type NavItem } from "$lib/nav-config";
  import type { Snippet } from "svelte";

  let pluginNavItems = $state<NavItem[]>([]);

  let allNav = $derived([...CORE_NAV_ITEMS, ...pluginNavItems].sort((a, b) => (a.order ?? 50) - (b.order ?? 50)));
  let primaryNav = $derived(allNav.filter((item) => item.group === "primary"));
  let secondaryNav = $derived(allNav.filter((item) => item.group === "secondary"));

  let ytdlpDismissed = $state(false);
  let ytdlpMissing = $derived(isDepsChecked() && !isYtdlpAvailable());
  let showOnboarding = $derived(needsOnboarding());

  async function openAuthorGithub(e: Event) {
    e.preventDefault();
    await open("https://github.com/tonhowtf");
  }

  let counts = $derived(getCounts());
  let badgeLabel = $derived(counts.badge > 99 ? "99+" : String(counts.badge));
  let settings = $derived(getSettings());

  function handleExternalUrlEvent(event: Omit<ExternalUrlEvent, "id">) {
    if (event.action === "prefill") {
      queueExternalPrefill(event);
      showToast("info", $t("toast.external_url_ready"));
      if (page.url.pathname !== "/") {
        goto("/");
      }
      return;
    }

    if (event.action === "queued") {
      showToast("success", $t("toast.external_url_queued"));
    }
  }

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
    let unlistenExternal: (() => void) | undefined;
    initDownloadListener().then((fn) => (cleanup = fn));

    invoke<{ id: string; enabled: boolean; nav: { route: string; label: Record<string, string>; icon_svg: string | null; group: string; order: number }[] }[]>("list_plugins")
      .then((plugins) => {
        const items: NavItem[] = [];
        for (const p of plugins) {
          if (!p.enabled) continue;
          for (const n of p.nav) {
            items.push({
              href: n.route,
              label: n.label[get(locale)] || n.label["en"] || p.id,
              icon: "plugin",
              iconSvg: n.icon_svg || undefined,
              group: n.group === "primary" ? "primary" : "secondary",
              pluginId: p.id,
              order: n.order,
            });
          }
        }
        pluginNavItems = items;
      })
      .catch(() => {});
    listen<Omit<ExternalUrlEvent, "id">>("external-url", (event) => {
      handleExternalUrlEvent(event.payload);
    }).then((fn) => {
      unlistenExternal = fn;
      invoke<Omit<ExternalUrlEvent, "id">[]>("register_external_frontend")
        .then((events) => {
          for (const event of events) {
            handleExternalUrlEvent(event);
          }
        })
        .catch(() => {});
    });
    refreshUpdateInfo();
    initChangelog();
    refreshYtdlpStatus();

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
      unlistenExternal?.();
      mediaQuery.removeEventListener("change", handleChange);
    };
  });

  function isActive(href: string): boolean {
    if (href === "/") return page.url.pathname === "/";
    return page.url.pathname.startsWith(href);
  }
</script>

<div class="layout">
  <nav class="sidebar">
    {#each primaryNav as item}
      {@const itemTitle = item.label || (item.labelKey ? $t(item.labelKey) : '')}
      <a
        href={item.href}
        class="nav-item"
        class:active={isActive(item.href)}
        title={itemTitle}
      >
        <span class="indicator"></span>
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          {#if item.iconSvg}
            {#each item.iconSvg.split(' M').map((d, i) => i === 0 ? d : 'M' + d) as pathD}
              <path d={pathD} />
            {/each}
          {:else if item.icon === "home"}
            <path d="M3 12L12 3l9 9" />
            <path d="M5 10v9a1 1 0 001 1h3v-5h6v5h3a1 1 0 001-1v-9" />
          {:else if item.icon === "downloads"}
            <path d="M12 3v12m0 0l-4-4m4 4l4-4" />
            <path d="M4 17v2a1 1 0 001 1h14a1 1 0 001-1v-2" />
          {:else if item.icon === "marketplace"}
            <path d="M3 21h18" />
            <path d="M3 7v1a3 3 0 0 0 6 0V7" />
            <path d="M9 7v1a3 3 0 0 0 6 0V7" />
            <path d="M15 7v1a3 3 0 0 0 6 0V7" />
            <path d="M3 7h18l-1.5-4H4.5z" />
            <path d="M5 21V10" />
            <path d="M19 21V10" />
          {:else if item.icon === "settings"}
            <path d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
            <circle cx="12" cy="12" r="3" />
          {:else}
            <circle cx="12" cy="12" r="10" />
            <path d="M12 8v4m0 4h.01" />
          {/if}
        </svg>
        <span class="nav-label">{itemTitle}</span>
        {#if item.badge === "downloads" && counts.badge > 0}
          <span class="badge">{badgeLabel}</span>
        {/if}
      </a>
    {/each}

    {#if secondaryNav.length > 0}
      <div class="nav-divider"></div>
    {/if}

    {#each secondaryNav as item}
      {@const itemTitle = item.label || (item.labelKey ? $t(item.labelKey) : '')}
      <a
        href={item.href}
        class="nav-item"
        class:active={isActive(item.href)}
        title={itemTitle}
      >
        <span class="indicator"></span>
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          {#if item.iconSvg}
            {#each item.iconSvg.split(' M').map((d, i) => i === 0 ? d : 'M' + d) as pathD}
              <path d={pathD} />
            {/each}
          {:else if item.icon === "about"}
            <circle cx="12" cy="12" r="10" />
            <path d="M12 16v-4m0-4h.01" />
          {:else}
            <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" />
          {/if}
        </svg>
        <span class="nav-label">{itemTitle}</span>
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
        <button class="ytdlp-banner-close" onclick={() => ytdlpDismissed = true} aria-label={$t('common.close')}>
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
<DebugPanel />
<ChangelogDialog />
{#if showOnboarding}
  <OnboardingWizard />
{/if}

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
    width: 60px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    padding: 6px 0;
    border-radius: var(--border-radius);
    color: var(--gray);
  }

  .nav-label {
    font-size: 9px;
    font-weight: 500;
    line-height: 1;
    text-align: center;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  .nav-divider {
    width: 24px;
    height: 1px;
    background: var(--content-border);
    margin: 6px 0;
    flex-shrink: 0;
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
    color: var(--on-accent);
    background: var(--blue);
    border-radius: 9999px;
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
    bottom: calc(8px + var(--safe-area-bottom));
    right: calc(12px + var(--safe-area-right));
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

  @media (max-width: 535px) {
    .layout {
      grid-template-columns: unset;
      grid-template-rows: 1fr auto;
    }

    .sidebar {
      width: 100%;
      height: auto;
      flex-direction: row;
      position: fixed;
      bottom: 0;
      left: 0;
      right: 0;
      padding: 0;
      padding-left: var(--safe-area-left);
      padding-right: var(--safe-area-right);
      padding-bottom: var(--safe-area-bottom);
      border-top: 1px solid var(--content-border);
      overflow-x: auto;
      overflow-y: hidden;
      gap: 0;
    }

    .sidebar::after {
      content: "";
      position: absolute;
      right: 0;
      top: 0;
      bottom: 0;
      width: 32px;
      pointer-events: none;
      background: linear-gradient(to right, transparent 0%, var(--sidebar-bg) 100%);
    }

    .nav-item {
      width: auto;
      height: 44px;
      padding: 0 calc(var(--padding) * 1.5);
      flex-shrink: 0;
    }

    .nav-divider {
      width: 1px;
      height: 24px;
      margin: 0 4px;
    }

    .indicator {
      position: absolute;
      bottom: -8px;
      left: 50%;
      transform: translateX(-50%);
      width: 20px;
      height: 3px;
      border-radius: 0 0 2px 2px;
    }

    .nav-item.active .indicator {
      width: 20px;
      height: 3px;
    }

    .content {
      padding: calc(var(--padding) * 2);
      padding-bottom: var(--mobile-nav-clearance);
      order: -1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .indicator {
      transition: none;
    }
  }
</style>
