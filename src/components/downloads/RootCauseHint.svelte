<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";

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

  async function updateYtdlp() {
    updating = true;
    try {
      await invoke("install_dependency", { name: "yt-dlp" });
      updated = true;
    } catch {}
    updating = false;
  }
</script>

{#if diagnosis}
  <div class="root-cause" role="note">
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
{/if}

<style>
  .root-cause {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    padding: var(--space-1) 0;
  }

  .root-cause-text {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-muted);
  }

  .root-cause-done {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--success);
  }
</style>
