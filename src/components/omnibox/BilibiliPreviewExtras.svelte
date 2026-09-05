<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";

  type CodecOption = { codec_id: number; label_key: string };
  type QualityOption = { qn: number; label_key: string; codecs: CodecOption[] };
  type AudioOption = { qn: number; label_key: string };
  type PreviewSummary = {
    kind_label_key: string;
    item_title: string;
    item_count: number;
    qualities: QualityOption[];
    audios: AudioOption[];
    premium_required: boolean;
  };

  type Props = { url: string; accountSlug: string | null };
  let { url, accountSlug }: Props = $props();

  let summary = $state<PreviewSummary | null>(null);
  let loading = $state(false);
  let errorKey = $state<string | null>(null);
  let lastKey = "";

  const CACHE = new Map<string, PreviewSummary>();
  const MAX_CACHE_SIZE = 32;

  function cacheKey(u: string, slug: string | null): string {
    return `${slug ?? "_anon"}::${u}`;
  }

  $effect(() => {
    if (!url) return;
    const key = cacheKey(url, accountSlug);
    if (key === lastKey) return;
    lastKey = key;
    const cached = CACHE.get(key);
    if (cached) {
      summary = cached;
      loading = false;
      errorKey = null;
      return;
    }
    loadPreview(key);
  });

  async function loadPreview(key: string) {
    if (!url) return;
    loading = true;
    errorKey = null;
    summary = null;
    try {
      const res = await invoke<PreviewSummary>("bilibili_preview_info", {
        url,
        slug: accountSlug,
      });
      summary = res;
      if (CACHE.size >= MAX_CACHE_SIZE) {
        const oldest = CACHE.keys().next().value;
        if (oldest !== undefined) CACHE.delete(oldest);
      }
      CACHE.set(key, res);
    } catch (e) {
      errorKey = typeof e === "string" ? e : "errors.bilibili.content_unavailable";
    } finally {
      loading = false;
    }
  }

  let settings = $derived(getSettings());
  let selectedQn = $derived(settings?.download?.bilibili_preferred_qn ?? 200);
  let selectedCodec = $derived(settings?.download?.bilibili_preferred_codec ?? 20);
  let selectedAudio = $derived(settings?.download?.bilibili_preferred_audio_qn ?? 30300);

  let codecsForSelectedQn = $derived(() => {
    if (!summary) return [] as CodecOption[];
    const q = summary.qualities.find((q) => q.qn === selectedQn);
    return q?.codecs ?? [];
  });

  async function onQnChange(e: Event) {
    const value = parseInt((e.target as HTMLSelectElement).value, 10);
    await updateSettings({ download: { bilibili_preferred_qn: value } });
  }

  async function onCodecChange(e: Event) {
    const value = parseInt((e.target as HTMLSelectElement).value, 10);
    await updateSettings({ download: { bilibili_preferred_codec: value } });
  }

  async function onAudioChange(e: Event) {
    const value = parseInt((e.target as HTMLSelectElement).value, 10);
    await updateSettings({ download: { bilibili_preferred_audio_qn: value } });
  }
</script>

<div class="bili-extras" data-state={loading ? "loading" : summary ? "ready" : errorKey ? "error" : "idle"}>
  {#if loading}
    <span class="bili-loading">{$t("platforms.bilibili.preview.loading")}</span>
  {:else if errorKey}
    <span class="bili-error">{$t(errorKey)}</span>
  {:else if summary}
    {#if !accountSlug}
      <div class="bili-warn">{$t("platforms.bilibili.preview.no_account_hint")}</div>
    {/if}
    {#if summary.premium_required}
      <div class="bili-warn">{$t("platforms.bilibili.preview.premium_hint")}</div>
    {/if}

    <div class="bili-kind-badge">{$t(summary.kind_label_key)}{#if summary.item_count > 1} · {summary.item_count}{/if}</div>

    <div class="bili-pickers">
      <label class="bili-picker">
        <span class="bili-picker-label">{$t("platforms.bilibili.preview.quality")}</span>
        <select class="bili-select" value={selectedQn} onchange={onQnChange}>
          <option value={200}>{$t("platforms.bilibili.preview.auto")}</option>
          {#each summary.qualities as q (q.qn)}
            <option value={q.qn}>{$t(q.label_key)}</option>
          {/each}
        </select>
      </label>

      {#if codecsForSelectedQn().length > 0}
        <label class="bili-picker">
          <span class="bili-picker-label">{$t("platforms.bilibili.preview.codec")}</span>
          <select class="bili-select" value={selectedCodec} onchange={onCodecChange}>
            <option value={20}>{$t("platforms.bilibili.preview.auto")}</option>
            {#each codecsForSelectedQn() as c (c.codec_id)}
              <option value={c.codec_id}>{$t(c.label_key)}</option>
            {/each}
          </select>
        </label>
      {/if}

      {#if summary.audios.length > 0}
        <label class="bili-picker">
          <span class="bili-picker-label">{$t("platforms.bilibili.preview.audio")}</span>
          <select class="bili-select" value={selectedAudio} onchange={onAudioChange}>
            <option value={30300}>{$t("platforms.bilibili.preview.auto")}</option>
            {#each summary.audios as a (a.qn)}
              <option value={a.qn}>{$t(a.label_key)}</option>
            {/each}
          </select>
        </label>
      {/if}
    </div>
  {/if}
</div>

<style>
  .bili-extras {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 10px;
    padding: 10px 12px;
    border-radius: 10px;
    background: color-mix(in oklab, var(--surface) 80%, transparent);
    border: none;
    font-size: 12px;
  }
  .bili-loading,
  .bili-error {
    color: var(--text-muted);
    font-size: 12px;
  }
  .bili-error {
    color: var(--danger);
  }
  .bili-warn {
    color: var(--text-muted);
    font-size: 11px;
    padding: 4px 8px;
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border-radius: 6px;
  }
  .bili-kind-badge {
    display: inline-block;
    align-self: flex-start;
    padding: 2px 8px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--accent);
    font-size: 11px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .bili-pickers {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
  }
  .bili-picker {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 130px;
  }
  .bili-picker-label {
    font-size: 10.5px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .bili-select {
    padding: 5px 8px;
    border-radius: 6px;
    border: none;
    background: var(--surface);
    color: var(--text);
    font-size: 12px;
  }
  .bili-select:focus {
    outline: none;
    border-color: var(--accent);
  }
</style>
