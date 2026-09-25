<script lang="ts">
  import "../app.css";
  import "$lib/style/workspace-design.css";
  import { getWorkspaceDesign, initWorkspaceDesign } from "$lib/stores/workspace-design.svelte";
  let workspaceDesign = $derived(getWorkspaceDesign());
  let designScope = $derived(page.url.pathname === "/" || page.url.pathname === "/downloads" || page.url.pathname === "/llm" || page.url.pathname.startsWith("/llm/") || page.url.pathname === "/help" || page.url.pathname.startsWith("/help/"));
  onMount(initWorkspaceDesign);
  // Document-level scope includes native chrome and portals; cleanup restores
  // the legacy module contract when leaving the workspace.
  $effect(() => {
    if (!designScope) return;
    const root = document.documentElement;
    const previous = { preset: root.getAttribute("data-ds-preset"), mode: root.getAttribute("data-ds-mode"), scoped: root.classList.contains("ds-scope") };
    root.classList.add("ds-scope");
    root.setAttribute("data-ds-preset", workspaceDesign.preset);
    root.setAttribute("data-ds-mode", workspaceDesign.resolvedMode);
    return () => {
      if (!previous.scoped) root.classList.remove("ds-scope");
      for (const [key, value] of [["data-ds-preset", previous.preset], ["data-ds-mode", previous.mode]]) {
        if (value === null) root.removeAttribute(key!); else root.setAttribute(key!, value!);
      }
    };
  });

  import "$lib/style/queue-kinds.css";
  import { page } from "$app/state";
  import { isMac } from "$lib/platform";
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { initDownloadListener } from "$lib/stores/download-listener";
  import { getCounts } from "$lib/stores/download-store.svelte";
  import { getSettings, loadSettings } from "$lib/stores/settings-store.svelte";
  import { queueExternalPrefill, type ExternalUrlEvent } from "$lib/stores/external-url-store.svelte";
  import Toast from "$components/toast/Toast.svelte";
  import AppSidebar from "$components/shell/AppSidebar.svelte";
  import AppToolbar from "$components/shell/AppToolbar.svelte";
  import CommandPalette from "$components/shell/CommandPalette.svelte";
  import DownloadStatusBar from "$components/download/DownloadStatusBar.svelte";
  import { shellLayout } from "$lib/stores/shell-layout.svelte";
  import { setCommandPaletteItems } from "$lib/stores/command-palette-store.svelte";
  import { accountPaletteItems, activateAccount, getAccounts } from "$lib/stores/llm-accounts-store.svelte";
  import { refreshUpdateInfo } from "$lib/stores/update-store.svelte";
  import { startClipboardMonitor, stopClipboardMonitor, onClipboardUrl } from "$lib/stores/clipboard-monitor";
  import { readText } from "@tauri-apps/plugin-clipboard-manager";
  import { initChangelog } from "$lib/stores/changelog-store.svelte";
  import { needsOnboarding } from "$lib/stores/onboarding-store.svelte";
  import { isYtdlpAvailable, isDepsChecked, refreshYtdlpStatus } from "$lib/stores/dependency-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { rawTranslations, t, locale, isRtlLocale } from "$lib/i18n";
  import { trayStrings } from "$lib/tray-strings";
  import { agentPrompts } from "$lib/agent-prompts";
  import { get } from "svelte/store";
  import { CORE_NAV_ITEMS, type NavItem } from "$lib/nav-config";
  import type { Snippet } from "svelte";
  import type { Component } from "svelte";

  let leagueNavItems = $derived<NavItem[]>(
    (getSettings()?.league?.enabled ?? true)
      ? [{ href: "/league", labelKey: "league.nav", icon: "league", group: "app", order: 45 }]
      : []
  );

  let coreNavItems = $derived(
    CORE_NAV_ITEMS.filter(
      (item) =>
        item.href !== "/world" || (getSettings()?.world?.enabled ?? true),
    )
  );

  let allNav = $derived([...coreNavItems, ...leagueNavItems].sort((a, b) => (a.order ?? 50) - (b.order ?? 50)));
  let primaryNav = $derived(allNav.filter((item) => item.group === "primary"));
  let appNav = $derived(allNav.filter((item) => item.group === "app"));

  let ytdlpDismissed = $state(false);
  let ytdlpMissing = $derived(isDepsChecked() && !isYtdlpAvailable());
  let showOnboarding = $derived(needsOnboarding());

  let counts = $derived(getCounts());
  let badgeLabel = $derived(counts.badge > 99 ? "99+" : String(counts.badge));
  let settings = $derived(getSettings());

  // The tray menu is native, so the frontend owns the translations and pushes
  // them whenever the locale changes (see sync_tray_strings in channels.rs).
  // The values come from `rawTranslations`, not `$t`: the default parser
  // substitutes `{{placeholders}}` and would strip the `{{count}}` / `{{speed}}`
  // tokens the Rust side fills in, leaving the tray without the number and the
  // speed in every language (see $lib/tray-strings).
  $effect(() => {
    const payload = trayStrings($rawTranslations, $locale);
    invoke("sync_tray_strings", payload).catch(() => {
      // tray sync is best-effort (no backend in browser/dev)
    });
  });

  // Same push-based pattern for the prompts of the agents the backend seeds:
  // they are shown to the user, so they follow the interface language, while
  // the compiled-in English set in roster_store.rs stays the fallback
  // (see $lib/agent-prompts).
  $effect(() => {
    const payload = agentPrompts($rawTranslations, $locale);
    invoke("sync_llm_prompts", { prompts: payload }).catch(() => {
      // prompt sync is best-effort (no backend in browser/dev)
    });
  });

  // The pet window is a bare 200x200 transparent canvas: no shell around it.
  let isPetWindow = $derived(page.url.pathname === "/pet");
  // Same for the limits strip: the window is exactly as big as what it draws.
  let isLimitsStrip = $derived(page.url.pathname === "/limits-strip");
  let isCoreRoute = $derived(
    page.url.pathname === "/" ||
    page.url.pathname.startsWith("/downloads") ||
    page.url.pathname.startsWith("/settings") ||
    page.url.pathname.startsWith("/league") ||
    page.url.pathname.startsWith("/about"),
  );

  let isFlushRoute = $derived(
    page.url.pathname === "/" ||
    page.url.pathname.startsWith("/downloads") ||
    page.url.pathname.startsWith("/settings"),
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

  onMount(() => {
    initDownloadListener();
    // If `get_settings` failed while the shell was booting, the sidebar has no
    // League entry and Settings spins forever. Retry a few times instead of
    // leaving the app half-configured until the next restart.
    if (!getSettings()) {
      let attempts = 0;
      const retry = () => {
        if (getSettings() || attempts >= 5) return;
        attempts += 1;
        loadSettings().catch(() => setTimeout(retry, 1000 * attempts));
      };
      setTimeout(retry, 500);
    }

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

    let unlistenExternalUrl: (() => void) | null = null;

    listen<Omit<ExternalUrlEvent, "id">>("external-url-event", (event) => {
      handleExternalUrlEvent(event.payload);
    }).then((un) => {
      unlistenExternalUrl = un;
    });

    return () => {
      if (unlistenExternalUrl) unlistenExternalUrl();
    };
  });

  function buildCommandPaletteItems() {
    setCommandPaletteItems([
      {
        id: "nav-home",
        label: get(t)("nav.home"),
        group: get(t)("command_palette.group_nav"),
        keywords: "index main prefill",
        action: () => goto("/"),
      },
      {
        id: "nav-downloads",
        label: get(t)("nav.downloads"),
        group: get(t)("command_palette.group_nav"),
        keywords: "queue active history",
        action: () => goto("/downloads"),
      },
      {
        id: "nav-settings",
        label: get(t)("nav.settings"),
        group: get(t)("command_palette.group_nav"),
        keywords: "preferences options config",
        action: () => goto("/settings"),
      },
      // Contas & cota: ⌘K troca a assinatura do CLI sem abrir a aba. Lê só o
      // estado já carregado, então não há IPC no boot.
      ...accountPaletteItems(
        getAccounts().accounts,
        {
          group: get(t)("command_palette.group_nav"),
          switchTo: (label) => `${get(t)("llm.accounts.palette_switch")} ${label}`,
          openTab: get(t)("llm.accounts.title"),
        },
        { activate: (id) => void activateAccount(id), open: () => goto("/llm/accounts") },
      ),
      { id: "nav-help", label: get(t)("nav.help"), group: get(t)("command_palette.group_nav"), keywords: "help ajuda guias docs assinatura agente monitor", action: () => goto("/help") },
      {
        id: "nav-about",
        label: get(t)("nav.about"),
        group: get(t)("command_palette.group_nav"),
        keywords: "info version changelog project",
        action: () => goto("/about"),
      },
      // The three former "action" entries ran the same goto() as the
      // navigation entries above them, so the palette listed Downloads and
      // Settings twice and Home three times. Only the paste action does
      // something navigation cannot — `command_palette.action_paste` was
      // already translated in every locale, just never wired up.
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
    // não: o macOS põe fechar/minimizar à esquerda (sobre a sidebar, titlebar
    // overlay), Windows e Linux à direita. Sem data-platform o app reservava
    // espaço morto e encostava a busca no botão de fechar.
    document.documentElement.setAttribute("data-platform", isMac() ? "macos" : "other");
    document.documentElement.setAttribute("dir", isRtlLocale($locale) ? "rtl" : "ltr");
    document.documentElement.setAttribute("lang", $locale || "en");
    buildCommandPaletteItems();
  });

  let { children }: { children: Snippet } = $props();

</script>

{#if isPetWindow || isLimitsStrip}
  {@render children()}
{:else}
<div class="shell" data-reduce-motion={settings?.accessibility?.reduce_motion} data-reduce-transparency={settings?.accessibility?.reduce_transparency}>
  <AppSidebar {primaryNav} {appNav} {badgeLabel} />

  <div class="shell-body" style:--shell-bottom-inset={`${shellLayout.bottomInset}px`}>
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

    <main id="main-content" class="content" class:ds-scope={designScope} data-ds-preset={designScope ? workspaceDesign.preset : undefined} data-ds-mode={designScope ? workspaceDesign.resolvedMode : undefined}>
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
    height: 24px;
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
