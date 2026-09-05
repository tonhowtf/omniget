<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { goto, replaceState } from "$app/navigation";
  import { t } from "$lib/i18n";
  import {
    studySettingsGet,
    studySettingsSave,
    type StudySettings,
  } from "$lib/study-bridge";
  import PlayerSettingsTab from "$lib/study-components/settings/PlayerSettingsTab.svelte";
  import SubtitleSettingsTab from "$lib/study-components/settings/SubtitleSettingsTab.svelte";
  import AudioSettingsTab from "$lib/study-components/settings/AudioSettingsTab.svelte";
  import BehaviorSettingsTab from "$lib/study-components/settings/BehaviorSettingsTab.svelte";
  import LibrarySettingsTab from "$lib/study-components/settings/LibrarySettingsTab.svelte";
  import MaintenanceTab from "$lib/study-components/settings/MaintenanceTab.svelte";
  import MusicSettingsTab from "$lib/study-components/settings/MusicSettingsTab.svelte";
  import DiagnosticTab from "$lib/study-components/settings/DiagnosticTab.svelte";
  import NotesSettingsTab from "$lib/study-components/notes/NotesSettingsTab.svelte";
  import YouTubeSettingsTab from "$lib/study-components/settings/YouTubeSettingsTab.svelte";

  type TabKey =
    | "player"
    | "subtitles"
    | "audio"
    | "behavior"
    | "library"
    | "music"
    | "youtube"
    | "diagnostic"
    | "notes"
    | "maintenance";

  const TABS = $derived<{ key: TabKey; label: string }[]>([
    { key: "player", label: "Player" },
    { key: "subtitles", label: "Legendas" },
    { key: "audio", label: "Áudio" },
    { key: "behavior", label: "Comportamento" },
    { key: "library", label: "Biblioteca" },
    { key: "music", label: "Música" },
    { key: "youtube", label: "YouTube" },
    { key: "diagnostic", label: $t("study.settings.tab_diagnostic") as string },
    { key: "notes", label: "Notas" },
    { key: "maintenance", label: "Manutenção" },
  ]);

  let activeTab = $state<TabKey>("player");
  let settings = $state<StudySettings>({});
  let loading = $state(true);
  let error = $state<string | null>(null);
  let toast = $state<{ kind: "ok" | "err"; msg: string } | null>(null);
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let savingState = $state<"idle" | "saving" | "saved">("idle");
  let savedFlashTimer: ReturnType<typeof setTimeout> | null = null;

  function readUrl() {
    const params = new URLSearchParams(window.location.search);
    const t = params.get("tab") as TabKey | null;
    if (t && TABS.some((x) => x.key === t)) activeTab = t;
  }

  function writeUrl(next: TabKey) {
    const params = new URLSearchParams(window.location.search);
    if (next === "player") params.delete("tab");
    else params.set("tab", next);
    const qs = params.toString();
    const url = `${window.location.pathname}${qs ? `?${qs}` : ""}`;
    if (window.location.search !== (qs ? `?${qs}` : "")) {
      try {
        replaceState(url, {});
      } catch {
        window.history.replaceState(null, "", url);
      }
    }
  }

  function pickTab(next: TabKey) {
    activeTab = next;
    writeUrl(next);
  }

  async function loadSettings() {
    loading = true;
    error = null;
    try {
      settings = await studySettingsGet();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function showToast(kind: "ok" | "err", msg: string) {
    toast = { kind, msg };
    setTimeout(() => {
      toast = null;
    }, 2400);
  }

  function deepMerge<T>(a: T, b: Partial<T>): T {
    const out = { ...(a as Record<string, unknown>) };
    for (const k of Object.keys(b as Record<string, unknown>)) {
      const av = (a as Record<string, unknown>)[k];
      const bv = (b as Record<string, unknown>)[k];
      if (
        bv !== null &&
        typeof bv === "object" &&
        !Array.isArray(bv) &&
        av !== null &&
        typeof av === "object" &&
        !Array.isArray(av)
      ) {
        out[k] = deepMerge(av, bv as Record<string, unknown>);
      } else {
        out[k] = bv;
      }
    }
    return out as T;
  }

  function flashSaved() {
    savingState = "saved";
    if (savedFlashTimer) clearTimeout(savedFlashTimer);
    savedFlashTimer = setTimeout(() => (savingState = "idle"), 1200);
  }

  function patchSettings(patch: StudySettings) {
    settings = deepMerge(settings, patch);
    if (saveTimer) clearTimeout(saveTimer);
    savingState = "saving";
    saveTimer = setTimeout(async () => {
      saveTimer = null;
      try {
        const next = await studySettingsSave(settings);
        settings = next;
        flashSaved();
      } catch (e) {
        showToast("err", e instanceof Error ? e.message : String(e));
        savingState = "idle";
      }
    }, 500);
  }

  onMount(() => {
    readUrl();
    void loadSettings();
    return () => {
      if (saveTimer) {
        clearTimeout(saveTimer);
        void studySettingsSave(settings).catch(() => {});
      }
      if (savedFlashTimer) clearTimeout(savedFlashTimer);
    };
  });
</script>

<section class="settings-page">
  <header class="head">
    <div class="head-text">
      <h1>{$t("study.hub.settings")}</h1>
      <p class="hint">Ajustes do player, legendas, biblioteca e manutenção.</p>
    </div>
    <div class="status">
      {#if savingState === "saving"}
        <span class="dot saving" aria-hidden="true"></span>
        <span>{$t("study.settings.saving")}</span>
      {:else if savingState === "saved"}
        <span class="dot saved" aria-hidden="true"></span>
        <span>{$t("study.settings.saved")}</span>
      {/if}
    </div>
  </header>

  <nav class="tabs" role="tablist" aria-label={$t("study.settings.categories_aria") as string}>
    {#each TABS as tab (tab.key)}
      <button
        type="button"
        class="tab"
        class:active={activeTab === tab.key}
        role="tab"
        aria-selected={activeTab === tab.key}
        onclick={() => pickTab(tab.key)}
      >
        {tab.label}
      </button>
    {/each}
  </nav>

  <div class="tab-body">
    {#if loading}
      <p class="muted">Carregando…</p>
    {:else if error}
      <p class="error">{error}</p>
    {:else if activeTab === "player"}
      <PlayerSettingsTab {settings} onPatch={patchSettings} />
    {:else if activeTab === "subtitles"}
      <SubtitleSettingsTab {settings} onPatch={patchSettings} />
    {:else if activeTab === "audio"}
      <AudioSettingsTab {settings} onPatch={patchSettings} />
    {:else if activeTab === "behavior"}
      <BehaviorSettingsTab {settings} onPatch={patchSettings} />
    {:else if activeTab === "library"}
      <LibrarySettingsTab {settings} onPatch={patchSettings} />
    {:else if activeTab === "music"}
      <MusicSettingsTab {settings} onPatch={patchSettings} />
    {:else if activeTab === "youtube"}
      <YouTubeSettingsTab />
    {:else if activeTab === "diagnostic"}
      <DiagnosticTab />
    {:else if activeTab === "notes"}
      <NotesSettingsTab onToast={showToast} />
    {:else if activeTab === "maintenance"}
      <MaintenanceTab onToast={showToast} />
    {/if}
  </div>

</section>

{#if toast}
  <div class="toast" class:err={toast.kind === "err"} role="status">
    {toast.msg}
  </div>
{/if}

<style>
  .settings-page {
    max-width: 760px;
    margin: 0 auto;
    padding: 24px 32px 64px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }

  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
  }

  .head-text h1 {
    margin: 0;
    font-size: 28px;
    font-weight: 600;
  }

  .head-text .hint {
    margin: 4px 0 0;
    color: color-mix(in oklab, currentColor 60%, transparent);
    font-size: 13px;
  }

  .status {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: color-mix(in oklab, currentColor 70%, transparent);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .dot.saving {
    background: var(--warning);
    animation: pulse 1.2s ease-in-out infinite;
  }

  .dot.saved {
    background: var(--success);
  }

  .tabs {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
    border-bottom: none;
    padding-bottom: 0;
  }

  .tab {
    padding: 10px 16px;
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: color-mix(in oklab, currentColor 70%, transparent);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    margin-bottom: -1px;
    transition: color var(--tg-duration-fast, 150ms);
  }

  .tab:hover {
    color: inherit;
  }

  .tab.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }

  .tab-body {
    display: flex;
    flex-direction: column;
  }

  .muted {
    color: color-mix(in oklab, currentColor 60%, transparent);
    font-size: 13px;
  }

  .error {
    color: var(--error);
    font-size: 13px;
    padding: 12px 14px;
    background: color-mix(in oklab, var(--error) 10%, transparent);
    border-radius: 8px;
  }

  .toast {
    position: fixed;
    bottom: 24px;
    left: 50%;
    transform: translateX(-50%);
    padding: 10px 16px;
    background: color-mix(in oklab, var(--success) 90%, black);
    color: white;
    border-radius: 8px;
    font-size: 13px;
    z-index: 100;
    box-shadow: 0 8px 24px color-mix(in oklab, black 30%, transparent);
    animation: slide-up 200ms ease-out;
  }

  .toast.err {
    background: var(--error);
  }

  @keyframes slide-up {
    from {
      opacity: 0;
      transform: translate(-50%, 12px);
    }
    to {
      opacity: 1;
      transform: translate(-50%, 0);
    }
  }

  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .dot.saving,
    .toast {
      animation: none;
    }
  }
</style>
