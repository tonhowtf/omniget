<script lang="ts">
  import "../app.css";
  import "$lib/style/queue-kinds.css";
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
  import AppSidebar from "$components/shell/AppSidebar.svelte";
  import AppToolbar from "$components/shell/AppToolbar.svelte";
  import CommandPalette from "$components/shell/CommandPalette.svelte";
  import { setCommandPaletteItems } from "$lib/stores/command-palette-store.svelte";
  import { refreshUpdateInfo } from "$lib/stores/update-store.svelte";
  import { startClipboardMonitor, stopClipboardMonitor } from "$lib/stores/clipboard-monitor";
  import { initChangelog } from "$lib/stores/changelog-store.svelte";
  import { needsOnboarding } from "$lib/stores/onboarding-store.svelte";
  import { isYtdlpAvailable, isDepsChecked, refreshYtdlpStatus } from "$lib/stores/dependency-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { ensureTrackerNotifications } from "$lib/tracker-notifications.svelte";
  import { t, locale } from "$lib/i18n";
  import { get } from "svelte/store";
  import { CORE_NAV_ITEMS, type NavItem } from "$lib/nav-config";
  import {
    STUDY_FOCUS_ENABLED,
    STUDY_PROGRESS_ENABLED,
    STUDY_ACHIEVEMENTS_ENABLED,
    STUDY_NOTES_ENABLED,
  } from "$lib/study-feature-flags";
  import type { Snippet } from "svelte";
  import type { Component } from "svelte";

  let pluginNavItems = $state<NavItem[]>([]);

  let allNav = $derived([...CORE_NAV_ITEMS, ...pluginNavItems].sort((a, b) => (a.order ?? 50) - (b.order ?? 50)));
  let primaryNav = $derived(allNav.filter((item) => item.group === "primary"));
  let appNav = $derived(allNav.filter((item) => item.group === "app"));
  let pluginNav = $derived(allNav.filter((item) => item.group === "plugins"));

  let ytdlpDismissed = $state(false);
  let ytdlpMissing = $derived(isDepsChecked() && !isYtdlpAvailable());
  let showOnboarding = $derived(needsOnboarding());

  let counts = $derived(getCounts());
  let badgeLabel = $derived(counts.badge > 99 ? "99+" : String(counts.badge));
  let settings = $derived(getSettings());

  let isStudyRoute = $derived(page.url.pathname.startsWith("/study"));
  let isCoreRoute = $derived(
    page.url.pathname === "/" ||
    page.url.pathname.startsWith("/downloads") ||
    page.url.pathname.startsWith("/settings") ||
    page.url.pathname.startsWith("/marketplace") ||
    page.url.pathname.startsWith("/about"),
  );

  let DebugPanel = $state<Component | null>(null);
  let OnboardingWizard = $state<Component | null>(null);
  let BandwidthPill = $state<Component | null>(null);
  let NotificationBell = $state<Component | null>(null);
  let ChangelogDialog = $state<Component | null>(null);
  let ConfirmCloseDialog = $state<Component | null>(null);
  let ShortcutsDialog = $state<Component | null>(null);
  let LegalDialog = $state<Component | null>(null);
  let RecoveryDialog = $state<Component | null>(null);
  let BilibiliSessionExpiredBanner = $state<Component | null>(null);

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

  function buildCommandPaletteItems() {
    const navItems = [...CORE_NAV_ITEMS, ...pluginNavItems].map((item) => ({
      id: `nav-${item.href}`,
      label: item.label || (item.labelKey ? get(t)(item.labelKey) : item.href),
      group: get(t)("command_palette.group_nav"),
      keywords: item.href,
      action: () => goto(item.href),
    }));

    setCommandPaletteItems([
      ...navItems,
      {
        id: "action-paste",
        label: get(t)("command_palette.action_paste"),
        group: get(t)("command_palette.group_action"),
        keywords: "clipboard url paste",
        action: () => goto("/"),
      },
      {
        id: "action-downloads",
        label: get(t)("command_palette.action_downloads"),
        group: get(t)("command_palette.group_action"),
        keywords: "queue",
        action: () => goto("/downloads"),
      },
      {
        id: "action-settings",
        label: get(t)("command_palette.action_settings"),
        group: get(t)("command_palette.group_action"),
        keywords: "preferences",
        action: () => goto("/settings"),
      },
    ]);
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

  $effect(() => {
    document.documentElement.setAttribute("data-shell", "mac");
    void $locale;
    buildCommandPaletteItems();
  });

  let { children }: { children: Snippet } = $props();

  const VACUUM_LAST_RUN_KEY = "study.library.auto_vacuum.last_run";

  async function checkAutoVacuum() {
    try {
      const { studySettingsGet, studyLibraryVacuum } = await import("$lib/study-bridge");
      const studySettings = await studySettingsGet();
      const enabled = studySettings.library?.auto_vacuum ?? true;
      if (!enabled) return;
      const intervalDays = studySettings.library?.auto_vacuum_interval_days ?? 30;
      const intervalMs = intervalDays * 86400 * 1000;
      const last = Number(localStorage.getItem(VACUUM_LAST_RUN_KEY) ?? "0");
      const now = Date.now();
      if (now - last < intervalMs) return;
      const result = await studyLibraryVacuum();
      localStorage.setItem(VACUUM_LAST_RUN_KEY, String(now));
      const total =
        (result.seek_logs_deleted ?? 0)
        + (result.notifications_deleted ?? 0)
        + (result.recents_deleted ?? 0);
      if (total > 0) {
        console.info(`[study] auto-vacuum: ${total} items cleaned`, result);
      }
    } catch (e) {
      console.warn("auto-vacuum failed", e);
    }
  }

  onMount(() => {
    let cleanup: (() => void) | undefined;
    let unlistenExternal: (() => void) | undefined;
    let unlistenChannels: (() => void) | undefined;
    initDownloadListener().then((fn) => (cleanup = fn));
    setTimeout(() => void checkAutoVacuum(), 5000);
    void ensureTrackerNotifications();
    import("$lib/rpc").then(({ rpcSyncIdleStats }) => rpcSyncIdleStats());

    void import("$components/dialog/ChangelogDialog.svelte").then((m) => { ChangelogDialog = m.default; });
    void import("$components/dialog/ConfirmCloseDialog.svelte").then((m) => { ConfirmCloseDialog = m.default; });
    void import("$components/dialog/ShortcutsDialog.svelte").then((m) => { ShortcutsDialog = m.default; });
    void import("$components/dialog/LegalDialog.svelte").then((m) => { LegalDialog = m.default; });
    void import("$components/dialog/RecoveryDialog.svelte").then((m) => { RecoveryDialog = m.default; });
    void import("$lib/components/BilibiliSessionExpiredBanner.svelte").then((m) => { BilibiliSessionExpiredBanner = m.default; });
    if (showOnboarding) {
      void import("$components/onboarding/OnboardingWizard.svelte").then((m) => { OnboardingWizard = m.default; });
    }
    void import("$components/debug/DebugPanel.svelte").then((m) => { DebugPanel = m.default; });

    invoke<{ id: string; enabled: boolean; nav: { route: string; label: Record<string, string>; icon_svg: string | null; group: string; order: number }[] }[]>("list_plugins")
      .then((plugins) => {
        const items: NavItem[] = [];
        for (const p of plugins) {
          if (!p.enabled) continue;
          for (const n of p.nav) {
            if (n.route === "/study/focus" && !STUDY_FOCUS_ENABLED) continue;
            if (n.route === "/study/progress" && !STUDY_PROGRESS_ENABLED) continue;
            if (n.route === "/study/achievements" && !STUDY_ACHIEVEMENTS_ENABLED) continue;
            if (n.route === "/study/notes" && !STUDY_NOTES_ENABLED) continue;
            items.push({
              href: n.route,
              label: n.label[get(locale)] || n.label["en"] || p.id,
              icon: "plugin",
              iconSvg: n.icon_svg || undefined,
              group: "plugins",
              pluginId: p.id,
              order: n.order,
            });
          }
        }
        pluginNavItems = items;
        buildCommandPaletteItems();
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
    listen<{ channel_title: string; auto_download: boolean; videos: unknown[] }>(
      "channel-new-videos",
      (event) => {
        const p = event.payload;
        const count = p.videos?.length ?? 0;
        if (count <= 0) return;
        showToast(
          "info",
          $t(
            p.auto_download
              ? "toast.channel_new_auto"
              : "toast.channel_new",
            { channel: p.channel_title, count },
          ) as string,
        );
      },
    ).then((fn) => {
      unlistenChannels = fn;
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
      unlistenChannels?.();
      mediaQuery.removeEventListener("change", handleChange);
    };
  });

  $effect(() => {
    if (isStudyRoute) {
      void import("$lib/study-components/BandwidthPill.svelte").then((m) => { BandwidthPill = m.default; });
      void import("$lib/study-components/shelves/NotificationBell.svelte").then((m) => { NotificationBell = m.default; });
    } else {
      BandwidthPill = null;
      NotificationBell = null;
    }
  });
</script>

<div class="mac-shell">
  <AppToolbar />
  <div class="mac-shell-body">
    <AppSidebar
      {primaryNav}
      {appNav}
      {pluginNav}
      {badgeLabel}
      badgeCount={counts.badge}
    />
    <main
      class="mac-content"
      class:mac-content--home={page.url.pathname === "/"}
      class:mac-content--pane={
        page.url.pathname.startsWith("/settings") ||
        page.url.pathname.startsWith("/downloads")
      }
    >
      {#if ytdlpMissing && !ytdlpDismissed && isCoreRoute}
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
    </main>
  </div>
</div>

<CommandPalette />

{#if isStudyRoute && BandwidthPill}
  <div class="bandwidth-pill-mount">
    <BandwidthPill />
  </div>
{/if}

{#if isStudyRoute && NotificationBell}
  <div class="notification-bell-mount">
    <NotificationBell />
  </div>
{/if}

<Toast />
{#if DebugPanel}
  <DebugPanel />
{/if}
{#if BilibiliSessionExpiredBanner}
  <BilibiliSessionExpiredBanner />
{/if}
{#if ChangelogDialog}
  <ChangelogDialog />
{/if}
{#if ConfirmCloseDialog}
  <ConfirmCloseDialog />
{/if}
{#if ShortcutsDialog}
  <ShortcutsDialog />
{/if}
{#if LegalDialog}
  <LegalDialog />
{/if}
{#if RecoveryDialog}
  <RecoveryDialog />
{/if}
{#if showOnboarding && OnboardingWizard}
  <OnboardingWizard />
{/if}

<style>
  .bandwidth-pill-mount {
    position: fixed;
    bottom: 12px;
    right: 12px;
    z-index: 50;
    pointer-events: none;
  }

  .notification-bell-mount {
    position: fixed;
    top: calc(var(--titlebar-height) + 8px);
    right: 16px;
    z-index: 60;
  }

  .ytdlp-banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    margin-bottom: var(--padding);
    background: color-mix(in srgb, var(--warning) 18%, var(--surface));
    color: color-mix(in srgb, var(--warning) 82%, var(--text));
    border: 1px solid color-mix(in srgb, var(--warning) 34%, var(--border));
    border-radius: var(--border-radius);
    font-size: 12.5px;
    font-weight: 500;
  }

  :global([data-theme="light"]) .ytdlp-banner,
  :global([data-theme="catppuccin-latte"]) .ytdlp-banner,
  :global([data-theme="eink-day"]) .ytdlp-banner,
  :global([data-theme="eink-sepia"]) .ytdlp-banner,
  :global([data-theme="nyxvamp-radiance"]) .ytdlp-banner {
    color: var(--on-warning);
  }

  .ytdlp-banner-text {
    flex: 1;
  }

  .ytdlp-banner-link {
    background: var(--warning);
    color: #fff;
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
      background: color-mix(in srgb, var(--warning) 65%, black);
    }
  }

  .ytdlp-banner-close {
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    padding: 2px;
    opacity: 0.7;
    display: flex;
    align-items: center;
  }

  @media (hover: hover) {
    .ytdlp-banner-close:hover {
      opacity: 1;
    }
  }
</style>
