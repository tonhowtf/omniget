<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { translateBackendError } from "$lib/error-translate";
  import { classifyError } from "$lib/home/friendly-error";

  type Remedy =
    | "import_cookies"
    | "use_impersonation"
    | "set_up_pot_provider"
    | "switch_player_client"
    | "wait_and_retry"
    | "free_disk_space"
    | "update_ytdlp"
    | "none";

  type Diagnosis = { cause_key: string; remedy: Remedy; detail: string | null };

  let { error, onRetry, url, downloadId }: { error: string; onRetry?: () => void; url?: string; downloadId?: number } = $props();

  let diagnosis = $state<Diagnosis | null>(null);
  let updating = $state(false);

  const RETRY_REMEDIES: Remedy[] = ["use_impersonation", "switch_player_client", "wait_and_retry"];
  const POT_GUIDE_URL = "https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide";

  $effect(() => {
    const stderr = error;
    if (!stderr) {
      diagnosis = null;
      return;
    }
    invoke<Diagnosis | null>("diagnose_download_error", { stderr })
      .then((d) => (diagnosis = d))
      .catch(() => (diagnosis = null));
  });

  let kind = $derived(classifyError(error));
  let translated = $derived.by(() => {
    const known = translateBackendError(error, $t, $t);
    return known !== error || kind === "generic" ? known : ($t(`omnibox.err.${kind}.title`) as string);
  });
  const NEEDS_UPDATE = /extraction failed|extractor is broken|update yt-dlp/i;
  let wantsUpdate = $derived(diagnosis ? diagnosis.remedy === "update_ytdlp" : NEEDS_UPDATE.test(error));
  const BROWSER_HELPS = /unsupported url|login|log in|sign in|cookies|members|private|403|401|forbidden|unable to extract|no video formats/i;
  let browserUrl = $derived(
    url && /^https?:\/\//i.test(url) && (BROWSER_HELPS.test(error) || diagnosis?.remedy === "import_cookies" || diagnosis?.remedy === "use_impersonation") ? url : null,
  );
  let showRaw = $derived(translated !== error);
  let wantsLogin = $derived(
    diagnosis ? diagnosis.remedy === "import_cookies" || diagnosis.remedy === "set_up_pot_provider" : kind === "login" || kind === "age",
  );

  async function openInBuiltinBrowser() {
    try {
      const source = downloadId === undefined ? (browserUrl ?? "") : await invoke<string>("get_download_source_url", { id: downloadId });
      await goto(`/global?url=${encodeURIComponent(source)}`);
    } catch {
      showToast("error", $t("downloads.history_link_expired") as string);
    }
  }

  async function updateAndRetry() {
    updating = true;
    try {
      await invoke("install_dependency", { name: "yt-dlp" });
      onRetry?.();
    } catch (e) {
      showToast("error", $t("settings.dependencies.update_failed", { name: "yt-dlp", msg: String(e) }) as string);
    }
    updating = false;
  }
</script>

<div class="root-cause" role="note">
  <div class="root-cause-row">
    <span class="root-cause-text" class:root-cause-text--plain={!diagnosis}>{diagnosis ? $t(diagnosis.cause_key) : translated}</span>
    {#if wantsLogin}
      <button class="btn btn-secondary btn-sm" onclick={() => goto("/settings?tab=cookies")}>
        {$t("omnibox.err.login.action")}
      </button>
    {/if}
    {#if wantsUpdate}
      <button class="btn btn-secondary btn-sm" class:loading={updating} onclick={updateAndRetry} disabled={updating}>
        {#if updating}<span class="spinner"></span>{/if}
        {$t("error.remedy.update_ytdlp")}
      </button>
    {:else if diagnosis && (RETRY_REMEDIES.includes(diagnosis.remedy) || diagnosis.remedy === "set_up_pot_provider") && onRetry}
      <button class="btn btn-secondary btn-sm" onclick={onRetry}>{$t("error.remedy.retry")}</button>
    {/if}
  </div>
  {#if diagnosis || showRaw}
    <details class="root-cause-details">
      <summary>{$t("downloads.error_details")}</summary>
      <code>{error}</code>
      {#if diagnosis?.remedy === "set_up_pot_provider"}
        <button class="btn btn-ghost btn-sm" onclick={() => openExternal(POT_GUIDE_URL)}>
          {$t("error.remedy.pot_guide")}
        </button>
      {/if}
    </details>
  {/if}
  {#if browserUrl}
    <div class="root-cause-row">
      <span class="root-cause-text">{$t("downloads.try_builtin_browser_hint")}</span>
      <button class="btn btn-secondary btn-sm" onclick={openInBuiltinBrowser}>
        {$t("omnibox.try_builtin_browser")}
      </button>
    </div>
  {/if}
</div>

<style>
  .root-cause {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1) 0;
  }

  .root-cause-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .root-cause-text {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text);
  }

  .root-cause-text--plain {
    color: var(--danger);
  }

  .root-cause-details summary {
    font-size: var(--text-xs);
    color: var(--text-dim);
    cursor: pointer;
    list-style: none;
    user-select: none;
  }

  .root-cause-details summary::-webkit-details-marker {
    display: none;
  }

  .root-cause-details summary::marker {
    content: "";
  }

  @media (hover: hover) {
    .root-cause-details summary:hover {
      color: var(--text-muted);
    }
  }

  .root-cause-details code {
    display: block;
    margin-top: var(--space-1);
    padding: var(--space-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.45;
    color: var(--text-muted);
    background: var(--fill-1);
    border-radius: var(--radius-sm);
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
  }
</style>
