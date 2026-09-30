<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Status = { supported: boolean; installed: boolean; path: string | null };

  let status = $state<Status | null>(null);
  let busy = $state(false);

  onMount(async () => {
    try {
      status = await invoke<Status>("quick_action_status");
    } catch {
      status = { supported: false, installed: false, path: null };
    }
  });

  async function toggle() {
    if (!status || busy) return;
    busy = true;
    try {
      status = status.installed
        ? await invoke<Status>("quick_action_remove")
        : await invoke<Status>("quick_action_install", { menuTitle: $t("downloads.tools.quick_action.menu_title") });
      showToast(
        "success",
        status.installed ? $t("downloads.tools.quick_action.installed_toast") : $t("downloads.tools.quick_action.removed_toast"),
      );
    } catch {
      showToast("error", $t("downloads.tools.quick_action.failed"));
    } finally {
      busy = false;
    }
  }
</script>

<div class="qa" data-agent-readiness={status && !busy ? "ready" : "loading"}>
  <p class="lead">{$t("downloads.tools.quick_action.lead")}</p>
  {#if status && !status.supported}
    <p class="dim">{$t("downloads.tools.quick_action.mac_only")}</p>
  {:else if status}
    <ol class="steps">
      <li>{$t("downloads.tools.quick_action.step_select")}</li>
      <li>{$t("downloads.tools.quick_action.step_menu", { title: $t("downloads.tools.quick_action.menu_title") })}</li>
      <li>{$t("downloads.tools.quick_action.step_queue")}</li>
    </ol>
    <div class="row">
      <button class="button" onclick={toggle} disabled={busy}>
        {status.installed ? $t("downloads.tools.quick_action.remove") : $t("downloads.tools.quick_action.install")}
      </button>
      {#if status.installed}
        <span class="ok" role="status">{$t("downloads.tools.quick_action.is_installed")}</span>
      {/if}
    </div>
    <p class="dim small">{$t("downloads.tools.quick_action.shortcut_hint")}</p>
  {/if}
</div>

<style>
  .qa {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .small {
    font-size: var(--text-xs, 11px);
  }
  .steps {
    margin: 0;
    padding-left: var(--space-4, 20px);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-sm);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .ok {
    color: var(--success, var(--accent));
    font-size: var(--text-sm);
  }
</style>
