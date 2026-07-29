<script lang="ts">
  import "../app.css";
  import "$lib/style/queue-kinds.css";
  import { page } from "$app/state";
  import { isMac } from "$lib/platform";
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
  import { startClipboardMonitor, stopClipboardMonitor, onClipboardUrl } from "$lib/stores/clipboard-monitor";
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

  let leagueNavItems = $derived<NavItem[]>(
    getSettings()?.league?.enabled
      ? [{ href: "/league", labelKey: "league.nav", icon: "league", group: "app", order: 45 }]
      : []
  );

  let allNav = $derived([...CORE_NAV_ITEMS, ...leagueNavItems, ...pluginNavItems].sort((a, b) => (a.order ?? 50) - (b.order ?? 50)));
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
    page.url.pathname.startsWith("/league") ||
    page.url.pathname.startsWith("/about"),
  );

  let DebugPanel = $state<any>(null);
  let OnboardingWizard = $state<any>(null);
  let ChangelogDialog = $state<any>(null);
  let ConfirmCloseDialog = $state<any>(null);
  let ShortcutsDialog = $state<any>(null);
  let LegalDialog = $state<any>(null);
  let RecoveryDialog = $state<any>(null);

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

  function reloadPluginNav() {
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
  }

  onMount(() => {
    initDownloadListener();

    if (import.meta.env.DEV) {
      import("$components/debug/DebugPanel.svelte").then((m) => {
        DebugPanel = m.default;
      });
    }

    import("$components/onboarding/OnboardingWizard.svelte").then((m) => {
      OnboardingWizard = m.default;
    });
    import("$components/dialog/ChangelogDialog.svelte").then((m) => {
      ChangelogDialog = m.default;
    });
    import("$components/dialog/ConfirmCloseDialog.svelte").then((m) => {
      ConfirmCloseDialog = m.default;
    });
    import("$components/dialog/ShortcutsDialog.svelte").then((m) => {
      ShortcutsDialog = m.default;
    });
    import("$components/dialog/LegalDialog.svelte").then((m) => {
      LegalDialog = m.default;
    });
    import("$components/dialog/RecoveryDialog.svelte").then((m) => {
      RecoveryDialog = m.default;
    });

    refreshYtdlpStatus();
    refreshUpdateInfo();
    initChangelog();
    ensureTrackerNotifications();
    reloadPluginNav();

    let unlistenExternalUrl: (() => void) | null = null;
    let unlistenPlugins: (() => void) | null = null;

    listen<Omit<ExternalUrlEvent, "id">>("external-url-event", (event) => {
      handleExternalUrlEvent(event.payload);
    }).then((un) => {
      unlistenExternalUrl = un;
    });

    listen("plugins-changed", () => {
      reloadPluginNav();
    }).then((un) => {
      unlistenPlugins = un;
    });

    return () => {
      if (unlistenExternalUrl) unlistenExternalUrl();
      if (unlistenPlugins) unlistenPlugins();
    };
  });

  function buildCommandPaletteItems() {
    setCommandPaletteItems([
      {
        id: "nav-home",
        label: get(t)("command_palette.nav_home"),
        group: get(t)("command_palette.group_nav"),
        keywords: "index main prefill",
        action: () => goto("/"),
      },
      {
        id: "nav-downloads",
        label: get(t)("command_palette.nav_downloads"),
        group: get(t)("command_palette.group_nav"),
        keywords: "queue active history",
        action: () => goto("/downloads"),
      },
      {
        id: "nav-settings",
        label: get(t)("command_palette.nav_settings"),
        group: get(t)("command_palette.group_nav"),
        keywords: "preferences options config",
        action: () => goto("/settings"),
      },
      {
        id: "nav-marketplace",
        label: get(t)("command_palette.nav_marketplace"),
        group: get(t)("command_palette.group_nav"),
        keywords: "plugins extensions store",
        action: () => goto("/marketplace"),
      },
      {
        id: "nav-about",
        label: get(t)("command_palette.nav_about"),
        group: get(t)("command_palette.group_nav"),
        keywords: "info version changelog project",
        action: () => goto("/about"),
      },
      {
        id: "action-prefill",
        label: get(t)("command_palette.action_prefill"),
        group: get(t)("command_palette.group_action"),
        keywords: "url link add download",
        action: () => {
          goto("/");
        },
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
      onClipboardUrl((clipboardUrl) => {
        queueExternalPrefill({ action: "prefill", url: clipboardUrl, source: "clipboard" });
        showToast("info", $t("toast.clipboard_detected") as string);
        if (page.url.pathname !== "/") {
          goto("/");
        }
      });
      startClipboardMonitor();
    } else {
      onClipboardUrl(null);
      stopClipboardMonitor();
    }
    return () => {
      onClipboardUrl(null);
      stopClipboardMonitor();
    };
  });

  $effect(() => {
    document.documentElement.setAttribute("data-shell", "mac");
    // O shell é o mesmo em todas as plataformas, mas os controles de janela
    // não: o macOS põe fechar/minimizar à esquerda, Windows e Linux à direita.
    // Sem isto o app reservava 78px à esquerda em todo mundo (espaço morto fora
    // do macOS) e colocava os próprios botões exatamente onde o Windows desenha
    // o botão de fechar.
    document.documentElement.setAttribute("data-platform", isMac() ? "macos" : "other");
    void $locale;
    buildCommandPaletteItems();
  });

  let { children }: { children: Snippet } = $props();

  const VACUUM_LAST_RUN_KEY = "study.library.auto_vacuum.last_run";

  async function checkAutoVacuum() {
    try {
      const now = Date.now();
      const lastRunStr = localStorage.getItem(VACUUM_LAST_RUN_KEY);
      const lastRun = lastRunStr ? parseInt(lastRunStr, 10) : 0;

      if (now - lastRun > 7 * 24 * 60 * 60 * 1000) {
        await invoke("db_vacuum");
        localStorage.setItem(VACUUM_LAST_RUN_KEY, String(now));
      }
    } catch {}
  }

  onMount(() => {
    void checkAutoVacuum();
  });
</script>

<div class="shell" data-reduce-motion={settings?.accessibility?.reduce_motion} data-reduce-transparency={settings?.accessibility?.reduce_transparency}>
  <AppSidebar {primaryNav} {appNav} {pluginNav} {badgeLabel} />

  <div class="shell-body">
    <AppToolbar />

    {#if ytdlpMissing && !ytdlpDismissed}
      <div class="ytdlp-banner" role="alert">
        <span class="ytdlp-banner-text">
          {$t("ytdlp_missing_banner.text")}
        </span>
        <div class="ytdlp-banner-actions">
          <a href="/settings#dependencies" class="button ytdlp-banner-link">
            {$t("ytdlp_missing_banner.open_settings")}
          </a>
          <button
            type="button"
            class="ytdlp-banner-close"
            onclick={() => (ytdlpDismissed = true)}
            aria-label={$t("ytdlp_missing_banner.dismiss") as string}
          >
            ✕
          </button>
        </div>
      </div>
    {/if}

    <main id="main-content" class="content">
      {#if isStudyRoute}
        <div class="study-shell">
          {@render children()}
        </div>
      {:else if isCoreRoute}
        <div class="core-shell">
          {@render children()}
        </div>
      {:else}
        {@render children()}
      {/if}
    </main>
  </div>
</div>

<Toast />
<CommandPalette />

{#if showOnboarding && OnboardingWizard}
  <OnboardingWizard />
{/if}

{#if DebugPanel}
  <DebugPanel />
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

<style>
  .shell {
    display: flex;
    width: 100vw;
    height: 100vh;
    overflow: hidden;
    background: var(--bg);
    color: var(--text);
  }

  .shell-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    height: 100vh;
    overflow: hidden;
  }

  .content {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .core-shell {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    max-width: 1200px;
    width: 100%;
    margin: 0 auto;
    padding: var(--padding);
  }

  .study-shell {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .ytdlp-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 16px;
    background: var(--warning);
    color: var(--on-warning, black);
    font-size: 13px;
    gap: 12px;
  }

  .ytdlp-banner-text {
    flex: 1;
    font-weight: 500;
  }

  .ytdlp-banner-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .ytdlp-banner-link {
    font-size: 12px;
    padding: 4px 10px;
    border-radius: var(--radius-sm);
    background: var(--cta);
    color: var(--on-cta);
    text-decoration: none;
    font-weight: 500;
  }

  @media (hover: hover) {
    .ytdlp-banner-link:hover {
      background: var(--cta-hover);
    }
  }

  .ytdlp-banner-link:active {
    background: var(--cta-press);
  }

  .ytdlp-banner-link:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
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
