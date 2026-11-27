<script lang="ts">
  import "../app.css";
  import { startNativeAgentBridge } from "$lib/native-agent/bridge";
  onMount(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    startNativeAgentBridge().then(cleanup => {
      if (disposed) cleanup(); else stop = cleanup;
    }).catch(() => {});
    return () => { disposed = true; stop?.(); };
  });
  import "$lib/style/workspace-design.css";
  onMount(() => {
    setSettingsLocaleApplier(async (language) => {
      await loadTranslations(language, page.url.pathname);
      locale.set(language);
    });
    const stopSync = startSettingsSync();
    return () => { stopSync(); setSettingsLocaleApplier(null); };
  });

  import "$lib/style/queue-kinds.css";
  import { page } from "$app/state";
  import { isMac } from "$lib/platform";
  import { afterNavigate, goto } from "$app/navigation";
  import { holdFocusUntilNavigated, releaseFocusAfterNavigation } from "$lib/settings-navigation";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { initDownloadListener } from "$lib/stores/download-listener";
  import { getCounts } from "$lib/stores/download-store.svelte";
  import { getSettings, getSettingsLoadState, loadSettings, updateSettings, setSettingsLocaleApplier, startSettingsSync } from "$lib/stores/settings-store.svelte";
  import { queueExternalPrefill, type ExternalUrlEvent } from "$lib/stores/external-url-store.svelte";
  import Toast from "#components/toast/Toast.svelte";
  import NativeAgentActivity from "#components/agents/NativeAgentActivity.svelte";
  import ToolAskPrompt from "#components/shell/ToolAskPrompt.svelte";
  import McpAuthPrompt from "#components/agents/McpAuthPrompt.svelte";
  import AppSidebar from "#components/shell/AppSidebar.svelte";
  import AppToolbar from "#components/shell/AppToolbar.svelte";
  import CommandPalette from "#components/shell/CommandPalette.svelte";
  import DownloadStatusBar from "#components/download/DownloadStatusBar.svelte";
  import MusicHost from "#components/music/MusicHost.svelte";
  import TubeHost from "#components/tube/TubeHost.svelte";
  import ReadHost from "#components/read/ReadHost.svelte";
  import { shellLayout } from "$lib/stores/shell-layout.svelte";
  import { setCommandPaletteItems } from "$lib/stores/command-palette-store.svelte";
  import { accountPaletteItems, activateAccount, getAccounts } from "$lib/stores/agent-accounts-store.svelte";
  import { refreshUpdateInfo } from "$lib/stores/update-store.svelte";
  import { startClipboardMonitor, stopClipboardMonitor, onClipboardUrl } from "$lib/stores/clipboard-monitor";
  import { readText } from "@tauri-apps/plugin-clipboard-manager";
  import { initChangelog } from "$lib/stores/changelog-store.svelte";
  import { needsOnboarding } from "$lib/stores/onboarding-store.svelte";
  import { isYtdlpAvailable, isDepsChecked, refreshYtdlpStatus } from "$lib/stores/dependency-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { rawTranslations, t, locale, isRtlLocale, loadTranslations } from "$lib/i18n";
  import { trayStrings } from "$lib/tray-strings";
  import { usageTrayStrings } from "$lib/usage-tray-strings";
  import { agentPrompts } from "$lib/agent-prompts";
  import { llmNotifyStrings } from "$lib/llm-notify-strings";
  import { focusPayload } from "$lib/ui-focus.svelte";
  import { get } from "svelte/store";
  import { visibleCoreNav } from "$lib/nav-config";
  import { paletteNavEntries } from "$lib/command-palette-nav";
  import { SUPERPOWERS } from "$lib/superpowers/catalog";
  import { currentPins, pinnedNavItems } from "$lib/superpowers/pins";
  import { isBareWindow } from "$lib/bare-window";
  import type { Snippet } from "svelte";
  import type { Component } from "svelte";

  holdFocusUntilNavigated();
  afterNavigate(() => { setTimeout(releaseFocusAfterNavigation, 0); });


  let coreNavItems = $derived(
    visibleCoreNav({
      downloadsOnly: getSettings()?.appearance?.downloads_only ?? false,
      worldEnabled: getSettings()?.world?.enabled ?? true,
    }),
  );

  const SUPERPOWER_TOGGLES = SUPERPOWERS.filter((entry) => entry.settingsKey);

  let superpowerEnabled = $derived(
    Object.fromEntries(
      SUPERPOWER_TOGGLES.map((entry) => [entry.id, getSettings()?.[entry.settingsKey!]?.enabled ?? true]),
    ),
  );

  let pinnedNav = $derived(pinnedNavItems(SUPERPOWERS, currentPins(getSettings()?.superpowers?.pinned ?? []), superpowerEnabled));

  let allNav = $derived([...coreNavItems, ...pinnedNav].sort((a, b) => (a.order ?? 50) - (b.order ?? 50)));
  let primaryNav = $derived(allNav.filter((item) => item.group === "primary"));
  let appNav = $derived(allNav.filter((item) => item.group === "app"));

  let ytdlpDismissed = $state(false);
  let ytdlpMissing = $derived(isDepsChecked() && !isYtdlpAvailable());
  let engineInstalling = $state(false);

  async function installEngine() {
    engineInstalling = true;
    let name = "yt-dlp";
    try {
      const deps = await invoke<{ name: string; installed: boolean }[]>("check_dependencies");
      for (const dep of deps) {
        if (dep.installed || (dep.name !== "yt-dlp" && dep.name !== "ffmpeg")) continue;
        name = dep.name;
        await invoke("install_dependency", { name });
      }
    } catch (e) {
      showToast("error", get(t)("settings.dependencies.install_failed_retry", { name, msg: String(e) }) as string);
    }
    await refreshYtdlpStatus();
    engineInstalling = false;
  }
  let showOnboarding = $derived(needsOnboarding());

  let counts = $derived(getCounts());
  let badgeLabel = $derived(counts.badge > 99 ? "99+" : String(counts.badge));
  let settings = $derived(getSettings());

  $effect(() => {
    const payload = trayStrings($rawTranslations, $locale);
    invoke("sync_tray_strings", { strings: payload }).catch(() => {
    });
  });

  $effect(() => {
    const strings = usageTrayStrings($rawTranslations, $locale);
    invoke("usage_tray_sync_strings", { strings }).catch(() => {});
  });

  $effect(() => {
    const payload = agentPrompts($rawTranslations, $locale);
    invoke("sync_llm_prompts", { prompts: payload }).catch(() => {
    });
  });

  $effect(() => {
    const strings = llmNotifyStrings($rawTranslations, $locale);
    invoke("sync_llm_notify_strings", { strings }).catch(() => {});
  });

  $effect(() => {
    if (isBareWindow(page.url.pathname)) return;
    invoke("llm_ui_focus", focusPayload(page.url.pathname)).catch(() => {});
  });

  const bareWindow = isBareWindow(page.url.pathname);
  let isCoreRoute = $derived(
    page.url.pathname === "/" ||
    page.url.pathname.startsWith("/downloads") ||
    page.url.pathname.startsWith("/settings") ||
    page.url.pathname.startsWith("/superpowers") ||
    page.url.pathname.startsWith("/league") ||
    page.url.pathname.startsWith("/music") ||
    page.url.pathname.startsWith("/video") ||
    page.url.pathname.startsWith("/youtube") ||
    page.url.pathname.startsWith("/washmybelly") ||
    page.url.pathname.startsWith("/read") ||
    page.url.pathname.startsWith("/converter") ||
    page.url.pathname.startsWith("/pdf") ||
    page.url.pathname.startsWith("/photos") ||
    page.url.pathname.startsWith("/cutout") ||
    page.url.pathname.startsWith("/palette") ||
    page.url.pathname.startsWith("/fonts") ||
    page.url.pathname.startsWith("/icons") ||
    page.url.pathname.startsWith("/motion") ||
    page.url.pathname.startsWith("/board") ||
    page.url.pathname.startsWith("/components") ||
    page.url.pathname.startsWith("/capture") ||
    page.url.pathname.startsWith("/studio") ||
    page.url.pathname.startsWith("/dictation") ||
    page.url.pathname.startsWith("/phonechat") ||
    page.url.pathname.startsWith("/guildchat") ||
    page.url.pathname === "/shortpost" ||
    page.url.pathname.startsWith("/photofeed") ||
    page.url.pathname.startsWith("/clipfeed") ||
    page.url.pathname.startsWith("/forums") ||
    page.url.pathname.startsWith("/pinboard") ||
    page.url.pathname.startsWith("/codehub") ||
    page.url.pathname.startsWith("/devtools") ||
    page.url.pathname.startsWith("/pack") ||
    page.url.pathname.startsWith("/scrape") ||
    /^\/api(\/|$)/.test(page.url.pathname) ||
    page.url.pathname.startsWith("/transfer") ||
    page.url.pathname.startsWith("/research") ||
    page.url.pathname.startsWith("/community"),
  );

  let isFlushRoute = $derived(
    page.url.pathname === "/" ||
    page.url.pathname.startsWith("/music") ||
    page.url.pathname.startsWith("/video") ||
    page.url.pathname.startsWith("/youtube") ||
    page.url.pathname.startsWith("/read") ||
    page.url.pathname.startsWith("/downloads") ||
    page.url.pathname.startsWith("/washmybelly") ||
    page.url.pathname.startsWith("/settings"),
  );

  let DebugPanel = $state<any>(null);
  let OnboardingWizard = $state<any>(null);
  let ChangelogDialog = $state<any>(null);
  let ConfirmCloseDialog = $state<any>(null);
  let WmbApprovalPrompt = $state<any>(null);
  let ConnectPrompt = $state<any>(null);
  let TelegramApprovalPrompt = $state<any>(null);
  let SocialApprovalPrompt = $state<any>(null);
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

    if (event.action === "helper") {
      goto(`/global?helper=${encodeURIComponent(event.url)}`);
      return;
    }

    if (event.action === "queued") {
      showToast("success", $t("toast.external_url_queued"));
    }
  }

  onMount(() => {
    if (bareWindow) return;
    let disposed = false;
    let stopDownloads: (() => void) | undefined;
    initDownloadListener().then(stop => {
      if (disposed) stop(); else stopDownloads = stop;
    }).catch(() => {});
    if (!getSettings()) {
      let attempts = 0;
      const retry = () => {
        if (disposed || getSettings() || attempts >= 5) return;
        if (getSettingsLoadState().kind === "loading") { setTimeout(retry, 1000); return; }
        attempts += 1;
        loadSettings().catch(() => setTimeout(retry, 1000 * attempts));
      };
      setTimeout(retry, 500);
    }

    if (import.meta.env.DEV) {
      import("#components/debug/DebugPanel.svelte").then((m) => {
        DebugPanel = m.default;
      });
    }

    import("#components/onboarding/OnboardingWizard.svelte").then((m) => {
      OnboardingWizard = m.default;
    });
    import("#components/dialog/ChangelogDialog.svelte").then((m) => {
      ChangelogDialog = m.default;
    });
    import("#components/washmybelly/ApprovalPrompt.svelte").then((m) => {
      WmbApprovalPrompt = m.default;
    });
    import("#components/agents/ConnectPrompt.svelte").then((m) => {
      ConnectPrompt = m.default;
    });
    import("$lib/telegram/ApprovalPrompt.svelte").then((m) => {
      TelegramApprovalPrompt = m.default;
    });
    import("$lib/social/ApprovalPrompt.svelte").then((m) => {
      SocialApprovalPrompt = m.default;
    });
    import("#components/dialog/ConfirmCloseDialog.svelte").then((m) => {
      ConfirmCloseDialog = m.default;
    });
    import("#components/dialog/ShortcutsDialog.svelte").then((m) => {
      ShortcutsDialog = m.default;
    });
    import("#components/dialog/LegalDialog.svelte").then((m) => {
      LegalDialog = m.default;
    });
    import("#components/dialog/RecoveryDialog.svelte").then((m) => {
      RecoveryDialog = m.default;
    });

    refreshYtdlpStatus();
    refreshUpdateInfo();
    initChangelog();

    let unlistenExternalUrl: (() => void) | null = null;

    listen<Omit<ExternalUrlEvent, "id">>("external-url", (event) => {
      handleExternalUrlEvent(event.payload);
    }).then((un) => {
      if (disposed) { un(); return; }
      unlistenExternalUrl = un;
      invoke<Omit<ExternalUrlEvent, "id">[]>("register_external_frontend")
        .then((events) => {
          for (const event of events) handleExternalUrlEvent(event);
        })
        .catch(() => {});
    });

    let unlistenVideoLink: (() => void) | null = null;
    listen<{ key: string; t: number }>("video-deep-link", (event) => {
      const q = new URLSearchParams({ key: event.payload.key, t: String(event.payload.t ?? 0) });
      void goto(`/video?${q.toString()}`);
    }).then((un) => {
      if (disposed) un();
      else unlistenVideoLink = un;
    });

    let unlistenQuickGoto: (() => void) | null = null;
    listen<string>("quick-panel://goto", (event) => {
      void goto(event.payload);
    }).then((un) => {
      if (disposed) un();
      else unlistenQuickGoto = un;
    });

    return () => {
      disposed = true;
      stopDownloads?.();
      if (unlistenExternalUrl) unlistenExternalUrl();
      unlistenVideoLink?.();
      unlistenQuickGoto?.();
    };
  });

  function buildCommandPaletteItems() {
    const tt = get(t);
    const group = tt("command_palette.group_nav");
    const everyNav = visibleCoreNav({ worldEnabled: getSettings()?.world?.enabled ?? true });
    const entries = paletteNavEntries(everyNav, SUPERPOWERS, superpowerEnabled);
    const musicAt = entries.findIndex((entry) => entry.id === "nav-music");
    const navItems = entries.map((entry) => ({
      id: entry.id,
      label: entry.label ?? tt(entry.labelKey ?? ""),
      group,
      keywords: entry.keywords,
      action: () => goto(entry.href),
    }));
    if (musicAt >= 0) {
      navItems.splice(musicAt + 1, 0, {
        id: "nav-music-stems",
        label: tt("music.tools.stems"),
        group,
        keywords: "karaoke karaokê separar voz vocal instrumental acapella a capella playback stems faixas remover voz tirar voz cantar letra sincronizada vocal remover voice remover instrumental split stems sing along lyrics",
        action: () => goto("/music?tool=stems"),
      });
    }
    setCommandPaletteItems([
      ...navItems,
      ...accountPaletteItems(
        getAccounts().accounts,
        {
          group,
          switchTo: (label) => `${tt("agents.accounts.palette_switch")} ${label}`,
          openTab: tt("agents.accounts.title"),
        },
        { activate: (id) => void activateAccount(id), open: () => goto("/settings?tab=ai&section=accounts") },
      ),
      {
        id: "action-paste",
        label: get(t)("command_palette.action_paste"),
        group: get(t)("command_palette.group_action"),
        keywords: "url link add download clipboard prefill",
        action: async () => {
          const text = (await readText().catch(() => null))?.trim();
          if (!text) {
            showToast("info", get(t)("toast.clipboard_empty") as string);
            return;
          }
          queueExternalPrefill({ action: "prefill", url: text, source: "clipboard" });
          goto("/");
        },
      },
    ]);
  }

  let clipboardDetection = $derived(!!settings?.download.clipboard_detection);

  $effect(() => {
    if (bareWindow) return;
    if (!clipboardDetection) {
      stopClipboardMonitor();
      return;
    }
    const offClipboardUrl = onClipboardUrl((clipboardUrl) => {
      queueExternalPrefill({ action: "prefill", url: clipboardUrl, source: "clipboard" });
      showToast("info", $t("toast.clipboard_url_detected") as string);
      if (page.url.pathname !== "/") {
        goto("/");
      }
    });
    startClipboardMonitor();
    return () => {
      offClipboardUrl();
      stopClipboardMonitor();
    };
  });

  $effect(() => {
    document.documentElement.setAttribute("data-shell", "mac");
    document.documentElement.setAttribute("data-platform", isMac() ? "macos" : "other");
    document.documentElement.setAttribute("dir", isRtlLocale($locale) ? "rtl" : "ltr");
    document.documentElement.setAttribute("lang", $locale || "en");
    buildCommandPaletteItems();
  });

  let sidebarCollapsed = $derived(getSettings()?.appearance?.sidebar_collapsed ?? false);

  $effect(() => {
    document.documentElement.setAttribute("data-sidebar", sidebarCollapsed ? "collapsed" : "expanded");
  });

  function toggleSidebar() {
    updateSettings({ appearance: { sidebar_collapsed: !sidebarCollapsed } }).catch(() => {
    });
  }

  function onSidebarShortcut(e: KeyboardEvent) {
    if (bareWindow || e.key.toLowerCase() !== "s" || e.altKey) return;
    const combo = isMac() ? e.ctrlKey && e.metaKey && !e.shiftKey : e.ctrlKey && e.shiftKey && !e.metaKey;
    if (!combo) return;
    e.preventDefault();
    toggleSidebar();
  }

  let { children }: { children: Snippet } = $props();

</script>

<svelte:window onkeydown={onSidebarShortcut} />

{#if bareWindow}
  {@render children()}
{:else}
<div class="shell" data-reduce-motion={settings?.accessibility?.reduce_motion} data-reduce-transparency={settings?.accessibility?.reduce_transparency}>
  <AppSidebar {primaryNav} {appNav} {badgeLabel} badgeCount={counts.badge} collapsed={sidebarCollapsed} onToggleCollapsed={toggleSidebar} />

  <div class="shell-body" style:--shell-bottom-inset={`${shellLayout.bottomInset}px`}>
    <AppToolbar />

    {#if ytdlpMissing && !ytdlpDismissed}
      <div class="ytdlp-banner" role="alert">
        <span class="ytdlp-banner-text">
          {$t("ytdlp_missing_banner.text")}
        </span>
        <div class="ytdlp-banner-actions">
          <button type="button" class="ytdlp-banner-link" onclick={installEngine} disabled={engineInstalling}>
            {#if engineInstalling}<span class="spinner"></span>{/if}
            {$t(engineInstalling ? "ytdlp_missing_banner.installing" : "ytdlp_missing_banner.install")}
          </button>
          <a href="/settings?tab=advanced&section=dependencies" class="btn btn-ghost btn-sm">
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
      <div class="mac-pane" class:mac-pane--flush={isFlushRoute}>
        {#if isCoreRoute}
          <div class="core-shell" class:core-shell--flush={isFlushRoute}>
            {@render children()}
          </div>
        {:else}
          <div class="mac-pane-scroll">
            {@render children()}
          </div>
        {/if}
      </div>
    </main>

    <DownloadStatusBar />
  </div>
</div>
{/if}

{#if !bareWindow}
<Toast />
{#if settings?.music?.enabled ?? true}<MusicHost />{/if}
{#if settings?.tube?.enabled ?? true}<TubeHost />{/if}
{#if settings?.read?.enabled ?? true}<ReadHost />{/if}
<div class="approval-corner">
  <McpAuthPrompt />
  <ToolAskPrompt />
</div>
<NativeAgentActivity />
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

{#if WmbApprovalPrompt && (getSettings()?.washmybelly?.enabled ?? true)}
  <WmbApprovalPrompt />
{/if}
{#if ConnectPrompt}
  <ConnectPrompt />
{/if}
{#if TelegramApprovalPrompt && (getSettings()?.telegram?.enabled ?? true)}
  <TelegramApprovalPrompt />
{/if}
{#if SocialApprovalPrompt}
  <SocialApprovalPrompt />
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
{/if}

<style>
  .approval-corner {
    position: fixed;
    inset-inline-end: var(--space-5);
    inset-block-end: var(--space-5);
    z-index: 900;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--space-2);
    max-height: calc(100vh - 2 * var(--space-5));
    overflow-y: auto;
    pointer-events: none;
  }
  .approval-corner > :global(*) {
    pointer-events: auto;
  }

  .shell {
    display: flex;
    width: 100vw;
    height: 100vh;
    overflow: hidden;
    background: var(--bg);
    color: var(--text);
  }

  .shell-body {
    padding-block-end: var(--shell-bottom-inset, 0px);
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    height: 100vh;
    overflow: hidden;
  }

  .content {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .core-shell {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    width: 100%;
    margin: 0;
    overflow-y: auto;
  }

  .core-shell--flush {
    padding: 0;
    overflow: hidden;
  }


  .ytdlp-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    margin: 0 var(--pane-inset) var(--space-2) 0;
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-4);
    background: color-mix(in srgb, var(--warning) 16%, var(--pane-bg));
    color: var(--text);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--warning) 40%, transparent);
    font-size: var(--text-base);
  }

  .ytdlp-banner-text {
    flex: 1;
    font-weight: 500;
  }

  .ytdlp-banner-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .ytdlp-banner-link {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    height: 24px;
    border: none;
    font-family: inherit;
    cursor: pointer;
    font-size: var(--text-sm);
    padding: 0 var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--cta);
    color: var(--on-cta);
    text-decoration: none;
    font-weight: 600;
    box-shadow: none;
  }

  @media (hover: hover) {
    .ytdlp-banner-link:hover {
      background: var(--cta-hover);
    }
  }

  .ytdlp-banner-link:active {
    background: var(--cta-press);
  }

  .ytdlp-banner-link:disabled {
    opacity: 0.7;
    cursor: default;
  }

  .ytdlp-banner-link:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .ytdlp-banner-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    background: none;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    cursor: pointer;
    padding: 0;
  }

  @media (hover: hover) {
    .ytdlp-banner-close:hover {
      background: var(--fill-2);
      color: var(--text);
    }
  }
</style>
