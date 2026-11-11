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
  import Toast from "$components/toast/Toast.svelte";
  import NativeAgentActivity from "$components/agents/NativeAgentActivity.svelte";
  import ToolAskPrompt from "$components/shell/ToolAskPrompt.svelte";
  import McpAuthPrompt from "$components/agents/McpAuthPrompt.svelte";
  import AppSidebar from "$components/shell/AppSidebar.svelte";
  import AppToolbar from "$components/shell/AppToolbar.svelte";
  import CommandPalette from "$components/shell/CommandPalette.svelte";
  import DownloadStatusBar from "$components/download/DownloadStatusBar.svelte";
  import MusicHost from "$components/music/MusicHost.svelte";
  import TubeHost from "$components/tube/TubeHost.svelte";
  import ReadHost from "$components/read/ReadHost.svelte";
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
  import { CORE_NAV_ITEMS } from "$lib/nav-config";
  import { SUPERPOWERS } from "$lib/superpowers/catalog";
  import { pinnedNavItems } from "$lib/superpowers/pins";
  import { isBareWindow } from "$lib/bare-window";
  import type { Snippet } from "svelte";
  import type { Component } from "svelte";

  holdFocusUntilNavigated();
  afterNavigate(() => { setTimeout(releaseFocusAfterNavigation, 0); });


  let coreNavItems = $derived(
    CORE_NAV_ITEMS.filter(
      (item) =>
        item.href !== "/world" || (getSettings()?.world?.enabled ?? true),
    )
  );

  const SUPERPOWER_TOGGLES = SUPERPOWERS.filter((entry) => entry.settingsKey);

  let pinnedNav = $derived.by(() => {
    const current = getSettings();
    const enabled = Object.fromEntries(
      SUPERPOWER_TOGGLES.map((entry) => [entry.id, current?.[entry.settingsKey!]?.enabled ?? true]),
    );
    return pinnedNavItems(SUPERPOWERS, current?.superpowers?.pinned ?? [], enabled);
  });

  let allNav = $derived([...coreNavItems, ...pinnedNav].sort((a, b) => (a.order ?? 50) - (b.order ?? 50)));
  let primaryNav = $derived(allNav.filter((item) => item.group === "primary"));
  let appNav = $derived(allNav.filter((item) => item.group === "app"));

  let ytdlpDismissed = $state(false);
  let ytdlpMissing = $derived(isDepsChecked() && !isYtdlpAvailable());
  let showOnboarding = $derived(needsOnboarding());

  let counts = $derived(getCounts());
  let badgeLabel = $derived(counts.badge > 99 ? "99+" : String(counts.badge));
  let settings = $derived(getSettings());

  $effect(() => {
    const payload = trayStrings($rawTranslations, $locale);
    invoke("sync_tray_strings", payload).catch(() => {
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
    page.url.pathname.startsWith("/whatsapp") ||
    page.url.pathname.startsWith("/discord") ||
    page.url.pathname === "/x" ||
    page.url.pathname.startsWith("/instagram") ||
    page.url.pathname.startsWith("/tiktok") ||
    page.url.pathname.startsWith("/reddit") ||
    page.url.pathname.startsWith("/pinterest") ||
    page.url.pathname.startsWith("/github") ||
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
    import("$components/washmybelly/ApprovalPrompt.svelte").then((m) => {
      WmbApprovalPrompt = m.default;
    });
    import("$components/agents/ConnectPrompt.svelte").then((m) => {
      ConnectPrompt = m.default;
    });
    import("$lib/telegram/ApprovalPrompt.svelte").then((m) => {
      TelegramApprovalPrompt = m.default;
    });
    import("$lib/social/ApprovalPrompt.svelte").then((m) => {
      SocialApprovalPrompt = m.default;
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

    return () => {
      disposed = true;
      stopDownloads?.();
      if (unlistenExternalUrl) unlistenExternalUrl();
      unlistenVideoLink?.();
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
      ...accountPaletteItems(
        getAccounts().accounts,
        {
          group: get(t)("command_palette.group_nav"),
          switchTo: (label) => `${get(t)("agents.accounts.palette_switch")} ${label}`,
          openTab: get(t)("agents.accounts.title"),
        },
        { activate: (id) => void activateAccount(id), open: () => goto("/settings?tab=ai&section=accounts") },
      ),
      {
        id: "nav-superpowers",
        label: get(t)("nav.superpowers"),
        group: get(t)("command_palette.group_nav"),
        keywords: "superpowers superpoderes league of legends lol extras",
        action: () => goto("/superpowers"),
      },
      {
        id: "nav-music",
        label: get(t)("music.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "music musica player library biblioteca radio lyrics letras playlist",
        action: () => goto("/music"),
      },
      {
        id: "nav-video",
        label: get(t)("video.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "video watch assistir player filmes movies series séries cursos courses aulas lessons iptv jellyfin anime legendas subtitles",
        action: () => goto("/video"),
      },
      {
        id: "nav-tube",
        label: get(t)("tube.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "youtube omnitube yt music ytmusic videos vídeos inscrições subscriptions shorts sponsorblock dearrow playlist músicas songs",
        action: () => goto("/youtube"),
      },
      {
        id: "nav-washmybelly",
        label: get(t)("washmybelly.nav"),
        group: get(t)("command_palette.group_nav"),
        keywords: "washmybelly clean limpar cache junk lixo uninstall desinstalar startup inicialização privacy privacidade debloat fps frametime battery bateria disk disco",
        action: () => goto("/washmybelly"),
      },
      {
        id: "nav-read",
        label: get(t)("read.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "read ler leitura livros books ebook epub pdf manga mangá comics quadrinhos reader leitor biblioteca opds kindle kobo",
        action: () => goto("/read"),
      },
      {
        id: "nav-converter",
        label: get(t)("converter.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "converter conversor convert converta formato format json yaml csv xml markdown html pdf docx png jpg jpeg webp mp3 mp4 gif",
        action: () => goto("/converter"),
      },
      {
        id: "nav-pdf",
        label: get(t)("pdf.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "pdf juntar merge combinar unir separar split dividir extrair extract páginas pages girar rotate reordenar reorder apagar delete página em branco blank comprimir compress reduzir tamanho shrink imagens images fotos jpg png scan escanear para pdf to pdf",
        action: () => goto("/pdf"),
      },
      {
        id: "nav-photos",
        label: get(t)("photos.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "fotos photos galeria gallery imagens pictures duplicatas duplicates limpar cleanup editar edit recortar crop exportar export heic raw",
        action: () => goto("/photos"),
      },
      {
        id: "nav-cutout",
        label: get(t)("cutout.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "recortar recorte cutout remover fundo remove background sem fundo transparente png figurinha sticker",
        action: () => goto("/cutout"),
      },
      {
        id: "nav-music-stems",
        label: get(t)("music.tools.stems"),
        group: get(t)("command_palette.group_nav"),
        keywords: "karaoke karaokê separar voz vocal instrumental acapella a capella playback stems faixas remover voz tirar voz cantar letra sincronizada vocal remover voice remover instrumental split stems sing along lyrics",
        action: () => goto("/music?tool=stems"),
      },
      {
        id: "nav-palette",
        label: get(t)("palette.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "palette paleta cores cor color colors oklch oklab contraste contrast wcag apca gradiente gradient degrade escala scale tokens tailwind css",
        action: () => goto("/palette"),
      },
      {
        id: "nav-fonts",
        label: get(t)("fonts.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "type tipografia typography fonte fontes font fonts inter geist jetbrains mono escala scale instalar install fontsource ofl",
        action: () => goto("/fonts"),
      },
      {
        id: "nav-icons",
        label: get(t)("icons.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "icons icones ícones icon svg lucide phosphor tabler heroicons iconify material sprite png",
        action: () => goto("/icons"),
      },
      {
        id: "nav-motion",
        label: get(t)("motion.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "motion animação animacao animation easing curva curve bezier spring mola timeline keyframes lottie dotlottie css gsap",
        action: () => goto("/motion"),
      },
      {
        id: "nav-board",
        label: get(t)("board.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "board quadro quadro-branco lousa whiteboard desenho drawing diagrama diagram rascunho sketch fluxograma flowchart mapa mental mind map excalidraw wireframe",
        action: () => goto("/board"),
      },
      {
        id: "nav-components",
        label: get(t)("components.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "components componentes shadcn registry registro ui blocks blocos magic ui react tailwind design tokens dtcg tokens studio figma penpot",
        action: () => goto("/components"),
      },
      {
        id: "nav-capture",
        label: get(t)("capture.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "capture captura gravar tela gravação de tela screen recorder screen recording record gravador cortar vídeo cortador cut video trim lossless cut sem perda keyframe segmentos segments snapshot microfone microphone ffmpeg obs losslesscut",
        action: () => goto("/capture"),
      },
      {
        id: "nav-studio",
        label: get(t)("studio.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "studio estúdio legendas subtitles captions legenda automática auto captions whisper srt vtt karaoke vertical 9:16 reels shorts tiktok reframe cartela title card vídeo video",
        action: () => goto("/studio"),
      },
      {
        id: "nav-dictation",
        label: get(t)("dictation.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "dictation ditado ditar voz voice typing digitar por voz speech to text fala transcrever whisper atalho push to talk microfone microphone handy superwhisper",
        action: () => goto("/dictation"),
      },
      {
        id: "nav-whatsapp",
        label: get(t)("whatsapp.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "whatsapp zap zapzap whats mensagens conversas grupos baixar mídias fotos vídeos áudios documentos figurinhas exportar conversa backup qr code messages chats groups download media photos videos audio voice notes documents stickers export chat backup",
        action: () => goto("/whatsapp"),
      },
      {
        id: "nav-discord",
        label: get(t)("discord.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "discord servidores servers canais channels mensagens messages dm conversas chats responder reply exportar export historico histórico history baixar download midia mídia media anexos attachments",
        action: () => goto("/discord"),
      },
      {
        id: "nav-social-x",
        label: get(t)("social.x.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "x twitter tweets tweet posts timeline feed perfil profile seguir follow curtir like repost retweet responder reply menções mentions bookmarks salvos busca search agente agents mcp",
        action: () => goto("/x"),
      },
      {
        id: "nav-social-instagram",
        label: get(t)("social.instagram.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "instagram insta ig feed reels stories posts perfil profile seguir follow curtir like comentar comment direct dm mensagens messages salvos saved busca search hashtags agente agents mcp",
        action: () => goto("/instagram"),
      },
      {
        id: "nav-social-tiktok",
        label: get(t)("social.tiktok.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "tiktok tik tok videos vídeos for you fyp perfil profile seguir follow curtir like comentar comment busca search agente agents mcp",
        action: () => goto("/tiktok"),
      },
      {
        id: "nav-social-reddit",
        label: get(t)("social.reddit.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "reddit subreddit subreddits comunidades communities posts upvote votar vote karma comentários comments salvos saved busca search front page popular agente agents mcp",
        action: () => goto("/reddit"),
      },
      {
        id: "nav-social-pinterest",
        label: get(t)("social.pinterest.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "pinterest pins pin boards pastas quadros ideias ideas inspiração inspiration moodboard salvar save seguir follow busca search agente agents mcp",
        action: () => goto("/pinterest"),
      },
      {
        id: "nav-social-github",
        label: get(t)("social.github.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "github git repos repositórios repositories stars estrelas stargazers forks issues pull requests prs releases trending notificações notifications seguidores followers token agente agents mcp",
        action: () => goto("/github"),
      },
      {
        id: "nav-devtools",
        label: get(t)("devtools.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "devtools dev tools ferramentas utilities utilitários json format formatar jwt decode hash md5 sha256 base64 regex diff comparar uuid timestamp port porta ports portas inspector localhost",
        action: () => goto("/devtools"),
      },
      {
        id: "nav-pack",
        label: get(t)("pack.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "pack empacotar repo repository repositório folder pasta prompt context contexto llm tokens budget secrets segredos scan repomix gitingest codebase",
        action: () => goto("/pack"),
      },
      {
        id: "nav-scrape",
        label: get(t)("scrape.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "scrape scraper raspar extrair extract page página site web markdown html selector seletor css elements elementos article artigo",
        action: () => goto("/scrape"),
      },
      {
        id: "nav-api",
        label: get(t)("api.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "api http request requisição rest get post put delete headers status timing tempo redirects redirecionamentos curl postman insomnia endpoint",
        action: () => goto("/api"),
      },
      {
        id: "nav-transfer",
        label: get(t)("transfer.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "transfer ftp ftps sftp ssh servidor server nas hospedagem hosting console switch arquivos files upload download enviar baixar remoto remote",
        action: () => goto("/transfer"),
      },
      {
        id: "nav-research",
        label: get(t)("research.title"),
        group: get(t)("command_palette.group_nav"),
        keywords: "research pesquisa scrapecreators api social tiktok instagram reels youtube reddit linkedin facebook meta ads anúncios ad library creators criadores influencers influenciadores transcript transcrição brief briefing concorrentes competitors",
        action: () => goto("/research"),
      },
      { id: "nav-help", label: get(t)("nav.help"), group: get(t)("command_palette.group_nav"), keywords: "help ajuda guias docs assinatura agente monitor", action: () => goto("/help") },
      {
        id: "nav-community",
        label: get(t)("nav.community"),
        group: get(t)("command_palette.group_nav"),
        keywords: "community comunidade chat discord info about sobre version changelog project",
        action: () => goto("/community"),
      },
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
<McpAuthPrompt />
<ToolAskPrompt />
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
