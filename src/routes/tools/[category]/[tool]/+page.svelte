<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { currentOs } from "$lib/platform";
  import ToolIcon from "$components/tools/ToolIcon.svelte";
  import ToolsPanel, { type ToolSection } from "$components/downloads/ToolsPanel.svelte";
  import SpicetifyTool from "$components/tools/spotify/SpicetifyTool.svelte";
  import WhisperTool from "$components/tools/speech/WhisperTool.svelte";
  import TtsTool from "$components/tools/speech/TtsTool.svelte";
  import SrtTranslateTool from "$components/tools/speech/SrtTranslateTool.svelte";
  import DubTool from "$components/tools/speech/DubTool.svelte";
  import OllamaTool from "$components/tools/ai/OllamaTool.svelte";
  import PricingTool from "$components/tools/ai/PricingTool.svelte";
  import UsageTool from "$components/tools/ai/UsageTool.svelte";
  import HumanizeTool from "$components/tools/ai/HumanizeTool.svelte";
  import KeysTool from "$components/tools/ai/KeysTool.svelte";
  import McpTool from "$components/tools/ai/McpTool.svelte";
  import SponsorBlockTool from "$components/tools/youtube/SponsorBlockTool.svelte";
  import DislikesTool from "$components/tools/youtube/DislikesTool.svelte";
  import FramesTool from "$components/tools/youtube/FramesTool.svelte";
  import CodecTool from "$components/tools/youtube/CodecTool.svelte";
  import RecordTool from "$components/tools/video/RecordTool.svelte";
  import VoiceStudioTool from "$components/tools/speech/VoiceStudioTool.svelte";
  import DictationTool from "$components/tools/speech/DictationTool.svelte";
  import AutoclickTool from "$components/tools/automation/AutoclickTool.svelte";
  import SlideshareTool from "$components/tools/documents/SlideshareTool.svelte";
  import GdocsTool from "$components/tools/documents/GdocsTool.svelte";
  import CalameoTool from "$components/tools/documents/CalameoTool.svelte";
  import GalleryTool from "$components/tools/documents/GalleryTool.svelte";
  import PdfTool from "$components/tools/documents/PdfTool.svelte";
  import UpscaleTool from "$components/tools/images/UpscaleTool.svelte";
  import ResizeTool from "$components/tools/images/ResizeTool.svelte";
  import OcrTool from "$components/tools/images/OcrTool.svelte";
  import DupesTool from "$components/tools/files/DupesTool.svelte";
  import RenameTool from "$components/tools/files/RenameTool.svelte";
  import FileSearchTool from "$components/tools/files/FileSearchTool.svelte";
  import AwakeTool from "$components/tools/files/AwakeTool.svelte";
  import Aria2Tool from "$components/tools/downloads/Aria2Tool.svelte";
  import ManifestTool from "$components/tools/downloads/ManifestTool.svelte";
  import WinTweaksTool from "$components/tools/system/WinTweaksTool.svelte";
  import SysCleanTool from "$components/tools/system/SysCleanTool.svelte";
  import DiskTool from "$components/tools/system/DiskTool.svelte";
  import StartupTool from "$components/tools/system/StartupTool.svelte";
  import UninstallTool from "$components/tools/system/UninstallTool.svelte";
  import DebloatTool from "$components/tools/system/DebloatTool.svelte";
  import RegistryTool from "$components/tools/system/RegistryTool.svelte";
  import UpdaterTool from "$components/tools/system/UpdaterTool.svelte";
  import KdeConnectTool from "$components/tools/phone/KdeConnectTool.svelte";
  import ThreadTool from "$components/tools/x/ThreadTool.svelte";
  import CardTool from "$components/tools/x/CardTool.svelte";
  import ProfileTool from "$components/tools/x/ProfileTool.svelte";
  import XMediaTool from "$components/tools/x/MediaTool.svelte";
  import XSearchTool from "$components/tools/x/SearchTool.svelte";
  import BookmarksTool from "$components/tools/x/BookmarksTool.svelte";
  import UnfollowTool from "$components/tools/x/UnfollowTool.svelte";
  import ArchiveTool from "$components/tools/x/ArchiveTool.svelte";
  import GrokTool from "$components/tools/x/GrokTool.svelte";
  import IgDownloadTool from "$components/tools/instagram/IgDownloadTool.svelte";
  import IgStoriesTool from "$components/tools/instagram/IgStoriesTool.svelte";
  import IgProfileTool from "$components/tools/instagram/IgProfileTool.svelte";
  import IgFollowTool from "$components/tools/instagram/IgFollowTool.svelte";
  import IgSnapshotsTool from "$components/tools/instagram/IgSnapshotsTool.svelte";
  import IgGhostsTool from "$components/tools/instagram/IgGhostsTool.svelte";
  import IgExportTool from "$components/tools/instagram/IgExportTool.svelte";
  import IgAnalyticsTool from "$components/tools/instagram/IgAnalyticsTool.svelte";
  import IgHashtagTool from "$components/tools/instagram/IgHashtagTool.svelte";
  import IgCommentsTool from "$components/tools/instagram/IgCommentsTool.svelte";
  import IgPublishTool from "$components/tools/instagram/IgPublishTool.svelte";
  import PinDownloadTool from "$components/tools/pinterest/PinDownloadTool.svelte";
  import BoardBackupTool from "$components/tools/pinterest/BoardBackupTool.svelte";
  import PinSearchTool from "$components/tools/pinterest/PinSearchTool.svelte";
  import PinSourceTool from "$components/tools/pinterest/PinSourceTool.svelte";
  import PinDupesTool from "$components/tools/pinterest/PinDupesTool.svelte";
  import PinPaletteTool from "$components/tools/pinterest/PinPaletteTool.svelte";
  import PinExportTool from "$components/tools/pinterest/PinExportTool.svelte";
  import PinKeywordsTool from "$components/tools/pinterest/PinKeywordsTool.svelte";
  import type { Component } from "svelte";

  // Runners que rodam dentro do Tools (chave do catálogo → componente).
  const RUNNERS: Record<string, Component<any>> = {
    spicetify: SpicetifyTool,
    whisper: WhisperTool,
    tts: TtsTool,
    "srt-translate": SrtTranslateTool,
    dub: DubTool,
    ollama: OllamaTool,
    pricing: PricingTool,
    usage: UsageTool,
    humanize: HumanizeTool,
    keys: KeysTool,
    mcp: McpTool,
    sponsorblock: SponsorBlockTool,
    ryd: DislikesTool,
    "yt-frames": FramesTool,
    codec: CodecTool,
    record: RecordTool,
    voicestudio: VoiceStudioTool,
    dictation: DictationTool,
    autoclick: AutoclickTool,
    slideshare: SlideshareTool,
    gdocs: GdocsTool,
    calameo: CalameoTool,
    gallery: GalleryTool,
    pdf: PdfTool,
    upscale: UpscaleTool,
    resize: ResizeTool,
    ocr: OcrTool,
    dupes: DupesTool,
    rename: RenameTool,
    "file-search": FileSearchTool,
    awake: AwakeTool,
    aria2: Aria2Tool,
    manifest: ManifestTool,
    "win-tweaks": WinTweaksTool,
    "win-harden": WinTweaksTool,
    sysclean: SysCleanTool,
    disk: DiskTool,
    startup: StartupTool,
    uninstall: UninstallTool,
    debloat: DebloatTool,
    winreg: RegistryTool,
    updater: UpdaterTool,
    kdeconnect: KdeConnectTool,
    "x-thread": ThreadTool,
    "x-card": CardTool,
    "x-profile": ProfileTool,
    "x-media": XMediaTool,
    "x-search": XSearchTool,
    "x-bookmarks": BookmarksTool,
    "x-unfollow": UnfollowTool,
    "x-archive": ArchiveTool,
    "x-grok": GrokTool,
    "ig-download": IgDownloadTool,
    "ig-bulk": IgDownloadTool,
    "ig-audio": IgDownloadTool,
    "ig-stories": IgStoriesTool,
    "ig-highlights": IgStoriesTool,
    "ig-story-viewers": IgStoriesTool,
    "ig-viewer": IgProfileTool,
    "ig-avatar": IgProfileTool,
    "ig-profile-media": IgProfileTool,
    "ig-unfollowers": IgFollowTool,
    "ig-fans": IgFollowTool,
    "ig-mutuals": IgFollowTool,
    "ig-whitelist": IgFollowTool,
    "ig-unfollowed": IgSnapshotsTool,
    "ig-ghosts": IgGhostsTool,
    "ig-export": IgExportTool,
    "ig-analytics": IgAnalyticsTool,
    "ig-benchmark": IgAnalyticsTool,
    "ig-hashtag": IgHashtagTool,
    "ig-comments": IgCommentsTool,
    "ig-likers": IgCommentsTool,
    "ig-giveaway": IgCommentsTool,
    "ig-publish": IgPublishTool,
    "ig-schedule": IgPublishTool,
    "pin-download": PinDownloadTool,
    "pin-board": BoardBackupTool,
    "pin-profile": BoardBackupTool,
    "pin-search": PinSearchTool,
    "pin-related": PinSearchTool,
    "pin-source": PinSourceTool,
    "pin-dupes": PinDupesTool,
    "pin-palette": PinPaletteTool,
    "pin-export": PinExportTool,
    "pin-keywords": PinKeywordsTool,
  };

  // Props extras por runner (o mesmo componente serve várias tools).
  const RUNNER_PROPS: Record<string, Record<string, unknown>> = {
    "win-harden": { group: "harden" },
    "pdf-merge": { mode: "merge" },
    "speech-clone": { mode: "clone" },
    "speech-design": { mode: "design" },
    "speech-isolate": { mode: "isolate" },
    "pdf-split": { mode: "split" },
    "pdf-compress": { mode: "compress" },
    "pdf-convert": { mode: "convert" },
    "pdf-ocr": { mode: "ocr" },
    "pdf-sanitize": { mode: "sanitize" },
    "win-tweaks": { group: "privacy" },
    "pin-board": { mode: "board" },
    "pin-profile": { mode: "profile" },
    "pin-search": { mode: "search" },
    "pin-related": { mode: "related" },
    "ig-download": { mode: "post" },
    "ig-bulk": { mode: "bulk" },
    "ig-audio": { mode: "audio" },
    "ig-stories": { mode: "stories" },
    "ig-highlights": { mode: "highlights" },
    "ig-story-viewers": { mode: "viewers" },
    "ig-viewer": { mode: "viewer" },
    "ig-avatar": { mode: "avatar" },
    "ig-profile-media": { mode: "media" },
    "ig-unfollowers": { mode: "unfollowers" },
    "ig-fans": { mode: "fans" },
    "ig-mutuals": { mode: "mutuals" },
    "ig-whitelist": { mode: "whitelist" },
    "ig-analytics": { mode: "analytics" },
    "ig-benchmark": { mode: "compare" },
    "ig-comments": { mode: "comments" },
    "ig-likers": { mode: "likers" },
    "ig-giveaway": { mode: "giveaway" },
    "ig-publish": { mode: "publish" },
    "ig-schedule": { mode: "schedule" },
  };
  import { categoryById, isCrossPlatform, toolById, type OsName } from "$lib/tools/catalog";

  let tool = $derived(toolById(page.params.tool ?? ""));
  let category = $derived(categoryById(page.params.category ?? ""));

  const RUNNER_SECTION: Record<string, ToolSection> = {
    "yt-metadata": "metadata",
    "yt-thumbnails": "thumbnails",
    "yt-subtitles": "subs",
    "yt-comments": "cc",
    "yt-livechat": "lc",
    "yt-workshop": "workshop",
  };

  let section = $derived(tool?.runner ? RUNNER_SECTION[tool.runner] : undefined);
  let Runner = $derived(tool?.runner && !section ? RUNNERS[tool.runner] : undefined);
  let os = currentOs();
  let runsHere = $derived(tool ? tool.platforms.includes(os) : true);

  const OS_NAME: Record<OsName, string> = { windows: "Windows", macos: "macOS", linux: "Linux" };
</script>

<section class="tool-page">
  {#if category}
    <a class="tools-back" href="/tools/{category.id}">
      <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M10 3 5 8l5 5" />
      </svg>
      {$t(`tools.categories.${category.id}.name`)}
    </a>
  {/if}

  {#if !tool}
    <div class="tools-empty">
      <img class="empty-state-art" src="/emoji/warning.png" alt="" width="96" height="96" />
      <h2>{$t("tools.hub.empty_title")}</h2>
    </div>
  {:else}
    <header class="tool-head">
      <ToolIcon icon={tool.icon} from={tool.from} to={tool.to} via={tool.via} size={64} muted={tool.status === "soon"} />
      <div class="tool-meta">
        <h1>{$t(`tools.catalog.${tool.id}.name`)}</h1>
        <p>{$t(`tools.catalog.${tool.id}.desc`)}</p>
        <div class="tool-tags">
          {#if tool.status === "soon"}
            <span class="tag">{$t("tools.hub.soon")}</span>
          {:else if tool.status === "beta"}
            <span class="tag tag-accent">{$t("tools.hub.beta")}</span>
          {/if}
          {#if isCrossPlatform(tool)}
            <span class="tag tag-success">{$t("tools.hub.cross")}</span>
          {:else}
            {#each tool.platforms as p (p)}
              <span class="tag tag-warning">{$t("tools.hub.runs_on")} {OS_NAME[p]}</span>
            {/each}
          {/if}
        </div>
      </div>
    </header>

    {#if !runsHere}
      <div class="tool-notice" role="status">
        {$t("tools.hub.not_on_this_os", { os: OS_NAME[os] })}
      </div>
    {/if}

    {#if section}
      <ToolsPanel only={[section]} />
    {:else if Runner}
      <Runner {...(RUNNER_PROPS[tool.runner ?? ""] ?? {})} />
    {:else if tool.status === "soon"}
      <div class="tools-empty tool-soon">
        <img class="empty-state-art" src="/emoji/hourglass_not_done.png" alt="" width="96" height="96" />
        <h2>{$t("tools.hub.soon_title")}</h2>
        <p>{$t("tools.hub.soon_desc")}</p>
      </div>
    {/if}
  {/if}
</section>

<style>
  .tool-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    width: 100%;
    max-width: 920px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .tools-back {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    align-self: flex-start;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--accent-hi);
    text-decoration: none;
  }

  .tools-back:hover {
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .tool-head {
    display: flex;
    align-items: flex-start;
    gap: var(--space-4);
  }

  .tool-meta {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .tool-meta h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .tool-meta p {
    margin: 0;
    font-size: var(--text-base);
    color: var(--text-muted);
    line-height: 1.45;
  }

  .tool-tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin-top: var(--space-1);
  }

  .tool-notice {
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--warning) 12%, transparent);
    color: var(--warning);
    font-size: var(--text-sm);
    font-weight: 500;
  }

  .tools-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8) var(--space-4);
    text-align: center;
  }

  .tool-soon {
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .tools-empty h2 {
    margin: var(--space-2) 0 0;
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--text);
  }

  .tools-empty p {
    margin: 0;
    max-width: 44ch;
    font-size: var(--text-base);
    color: var(--text-muted);
  }
</style>
