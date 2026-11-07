<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/capture/api";
  import type { HistoryEntry, Platform } from "$lib/capture/types";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import RecordPanel from "$components/capture/RecordPanel.svelte";
  import CutPanel from "$components/capture/CutPanel.svelte";
  import HistoryPanel from "$components/capture/HistoryPanel.svelte";

  type Tab = "record" | "cut";

  let tab = $state<Tab>("record");
  let recording = $state(false);
  let cutPath = $state<string | null>(null);
  let entries = $state<HistoryEntry[]>([]);
  let historyLoading = $state(true);
  let historyError = $state<string | null>(null);
  let platform = $state<Platform>("other");

  let lastRecording = $derived(entries.find((e) => e.kind === "recording" && e.exists)?.path ?? null);

  async function refreshHistory() {
    historyLoading = true;
    try {
      entries = await api.history();
      historyError = null;
    } catch (e) {
      historyError = api.errorText(e);
    } finally {
      historyLoading = false;
    }
  }

  function openInCutter(path: string) {
    cutPath = path;
    tab = "cut";
  }

  async function removeEntry(entry: HistoryEntry) {
    try {
      entries = await api.historyRemove(entry.id);
    } catch (e) {
      historyError = api.errorText(e);
    }
  }

  function tabKey(event: KeyboardEvent) {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    tab = tab === "record" ? "cut" : "record";
    document.getElementById(`cap-tab-${tab}`)?.focus();
  }

  onMount(() => {
    void refreshHistory();
  });
</script>

<section class="cap">
  <header class="cap-head">
    <NavIcon icon="capture" size={56} />
    <div class="cap-head-copy">
      <h1>{$t("capture.title")}</h1>
      <p class="cap-subtitle">{$t("capture.subtitle")}</p>
    </div>
  </header>

  <div class="mac-segmented cap-tabs" role="tablist" aria-label={$t("capture.title")} tabindex="-1" onkeydown={tabKey}>
    <button
      type="button"
      role="tab"
      id="cap-tab-record"
      aria-selected={tab === "record"}
      aria-controls="cap-panel-record"
      tabindex={tab === "record" ? 0 : -1}
      class="mac-segmented-btn"
      class:active={tab === "record"}
      onclick={() => (tab = "record")}
    >
      {#if recording}<span class="cap-live-dot" aria-hidden="true"></span>{/if}
      {recording ? $t("capture.tab_record_live") : $t("capture.tab_record")}
    </button>
    <button
      type="button"
      role="tab"
      id="cap-tab-cut"
      aria-selected={tab === "cut"}
      aria-controls="cap-panel-cut"
      tabindex={tab === "cut" ? 0 : -1}
      class="mac-segmented-btn"
      class:active={tab === "cut"}
      onclick={() => (tab = "cut")}
    >
      {$t("capture.tab_cut")}
    </button>
  </div>

  <div id="cap-panel-record" role="tabpanel" aria-labelledby="cap-tab-record" hidden={tab !== "record"}>
    <RecordPanel bind:recording oncut={openInCutter} onhistory={refreshHistory} onplatform={(p) => (platform = p)} />
  </div>
  <div id="cap-panel-cut" role="tabpanel" aria-labelledby="cap-tab-cut" hidden={tab !== "cut"}>
    <CutPanel bind:path={cutPath} {lastRecording} {platform} active={tab === "cut"} onhistory={refreshHistory} />
  </div>

  <HistoryPanel
    {entries}
    loading={historyLoading}
    error={historyError}
    onopen={(e) => openInCutter(e.path)}
    onreveal={(e) => void api.reveal(e.path).catch(() => {})}
    onremove={removeEntry}
    onretry={refreshHistory}
  />
</section>

<style>
  .cap {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .cap-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .cap-head-copy {
    min-width: 0;
  }

  .cap-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .cap-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .cap-tabs {
    align-self: flex-start;
  }

  .cap-live-dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
    background: var(--error);
  }
</style>
