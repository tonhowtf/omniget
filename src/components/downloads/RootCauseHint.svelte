<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";

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

  let { error, onRetry }: { error: string; onRetry?: () => void } = $props();

  let diagnosis = $state<Diagnosis | null>(null);
  let updating = $state(false);
  let updated = $state(false);

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

  let translated = $derived(translateBackendError(error, $t));
  let showRaw = $derived(translated !== error);

  async function updateYtdlp() {
    updating = true;
    try {
      await invoke("install_dependency", { name: "yt-dlp" });
      updated = true;
    } catch {}
    updating = false;
  }
</script>

<div class="root-cause" role="note">
  {#if diagnosis}
    <div class="root-cause-row">
      <span class="root-cause-text">{$t(diagnosis.cause_key)}</span>
      {#if diagnosis.remedy === "import_cookies"}
        <button class="btn btn-secondary btn-sm" onclick={() => goto("/settings?tab=cookies")}>
          {$t("error.remedy.import_cookies")}
        </button>
      {:else if diagnosis.remedy === "set_up_pot_provider"}
        <button class="btn btn-secondary btn-sm" onclick={() => openExternal(POT_GUIDE_URL)}>
          {$t("error.remedy.pot_guide")}
        </button>
      {:else if diagnosis.remedy === "update_ytdlp"}
        {#if updated}
          <span class="root-cause-done">{$t("error.remedy.updated")}</span>
          {#if onRetry}
            <button class="btn btn-secondary btn-sm" onclick={onRetry}>{$t("error.remedy.retry")}</button>
          {/if}
        {:else}
          <button class="btn btn-secondary btn-sm" class:loading={updating} onclick={updateYtdlp} disabled={updating}>
            {#if updating}<span class="spinner"></span>{/if}
            {$t("error.remedy.update_ytdlp")}
          </button>
        {/if}
      {:else if RETRY_REMEDIES.includes(diagnosis.remedy) && onRetry}
        <button class="btn btn-secondary btn-sm" onclick={onRetry}>{$t("error.remedy.retry")}</button>
      {/if}
    </div>
    <details class="root-cause-details">
      <summary>{$t("downloads.error_details")}</summary>
      <code>{error}</code>
    </details>
  {:else}
    <span class="root-cause-text root-cause-text--plain">{translated}</span>
    {#if showRaw}
      <details class="root-cause-details">
        <summary>{$t("downloads.error_details")}</summary>
        <code>{error}</code>
      </details>
    {/if}
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

  .root-cause-done {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--success);
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
