<script lang="ts">
  /**
   * Summarize a video with the saved AI connection (`ai_summarize_url`).
   * Opening this form only reads the saved connection (`ai_get_config`); a
   * summary runs only on an explicit click. The connection itself is edited in
   * Settings, so this form never saves or tests it.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { loadAiConnection, type AiConfigView } from "$components/settings/settings-ai-form";
  import { runSummary, summaryErrorKey, type SummaryStyle } from "./summaries";

  let { labelledBy, onsummarized }: { labelledBy?: string; onsummarized?: () => void } = $props();

  let config = $state<AiConfigView | null>(null);
  let configLoading = $state(true);
  let configError = $state(false);
  let url = $state("");
  let style = $state<SummaryStyle>("balanced");
  let lang = $state("");
  let summarizing = $state(false);
  let configured = $derived(!!config && config.provider !== "none");

  async function loadConfig() {
    configLoading = true;
    configError = false;
    try {
      config = await loadAiConnection(invoke);
    } catch {
      configError = true;
    } finally {
      configLoading = false;
    }
  }

  onMount(() => {
    void loadConfig();
  });

  async function summarize() {
    if (!url.trim() || summarizing || !configured) return;
    summarizing = true;
    try {
      await runSummary(invoke, { url, style, lang });
      url = "";
      onsummarized?.();
      showToast("success", $t("settings.ai.summary_ready") as string);
    } catch (error) {
      const key = summaryErrorKey(error);
      showToast("error", key ? ($t(key) as string) : typeof error === "string" ? error : ($t("common.error") as string));
    } finally {
      summarizing = false;
    }
  }
</script>

<div class="summary-form" aria-labelledby={labelledBy}>
  {#if configLoading}
    <p class="hint" role="status">{$t("common.loading")}</p>
  {:else if configError}
    <div class="notice" role="alert">
      <p>{$t("settings.ai.load_failed")}</p>
      <button type="button" class="button" onclick={loadConfig}>{$t("error.remedy.retry")}</button>
    </div>
  {:else if !configured}
    <p class="notice" role="status">
      {$t("settings.ai.not_configured")}
      <a href="/settings?tab=ai&section=connections">{$t("settings.ai.title")}</a>
    </p>
  {:else}
    <div class="row">
      <input
        type="text"
        class="input grow"
        placeholder={$t("settings.ai.summarize_placeholder")}
        aria-label={$t("settings.ai.summarize_placeholder")}
        aria-describedby="summary-effect"
        disabled={summarizing}
        bind:value={url}
        onkeydown={(e) => { if (e.key === "Enter") void summarize(); }}
      />
      <button type="button" class="button primary" disabled={!url.trim() || summarizing} aria-describedby="summary-effect" onclick={summarize}>
        {summarizing ? $t("settings.ai.summarizing") : $t("settings.ai.summarize")}
      </button>
    </div>
    <div class="row">
      <select class="input" bind:value={style} disabled={summarizing} aria-label={$t("settings.ai.style") as string}>
        <option value="short">{$t("settings.ai.style_short")}</option>
        <option value="balanced">{$t("settings.ai.style_balanced")}</option>
        <option value="detailed">{$t("settings.ai.style_detailed")}</option>
      </select>
      <input
        type="text"
        class="input grow"
        placeholder={$t("settings.ai.lang_placeholder")}
        aria-label={$t("settings.ai.lang_placeholder")}
        disabled={summarizing}
        bind:value={lang}
      />
    </div>
    <p class="hint" id="summary-effect"><bdi>{$t("settings.ai.summarize_effect")}</bdi></p>
    {#if config?.kind}<p class="hint">{$t("settings.ai.connection_kind", { kind: config.kind })} · <a href="/settings?tab=ai&section=connections">{$t("settings.ai.title")}</a></p>{/if}
  {/if}
</div>

<style>
  .summary-form { display: grid; gap: 8px; min-width: 0; }
  .row { display: flex; flex-wrap: wrap; gap: 8px; min-width: 0; }
  .grow { flex: 1 1 220px; min-width: 0; }
  .hint { margin: 0; color: var(--text-muted); font-size: 13px; line-height: 1.5; }
  .notice { margin: 0; font-size: 13px; line-height: 1.5; }
  .notice a, .hint a { color: var(--accent-text, var(--accent)); }
  a:focus-visible { outline: var(--focus-ring); }
</style>
