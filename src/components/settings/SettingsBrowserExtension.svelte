<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type BridgeInfo = {
    enabled: boolean;
    port: number;
    token: string;
    url: string;
  };

  let info = $state<BridgeInfo | null>(null);
  let revealed = $state(false);
  let rotating = $state(false);
  let loading = $state(true);

  async function refresh() {
    try {
      info = await invoke<BridgeInfo>("get_bridge_info");
    } catch (e: any) {
      info = null;
      showToast("error", typeof e === "string" ? e : (e.message ?? String(e)));
    } finally {
      loading = false;
    }
  }

  async function copy(value: string, kind: "url" | "token") {
    if (!value) return;
    try {
      await navigator.clipboard.writeText(value);
      showToast(
        "success",
        kind === "url"
          ? ($t("settings.bridge.copied_url") as string)
          : ($t("settings.bridge.copied_token") as string)
      );
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e.message ?? String(e)));
    }
  }

  async function rotate() {
    rotating = true;
    try {
      info = await invoke<BridgeInfo>("rotate_bridge_token");
      showToast("success", $t("settings.bridge.rotated") as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e.message ?? String(e)));
    } finally {
      rotating = false;
    }
  }

  function maskedToken(value: string): string {
    if (!value) return "";
    if (value.length <= 8) return "•".repeat(value.length);
    return value.slice(0, 4) + "…" + value.slice(-4);
  }

  onMount(refresh);
</script>

<section class="section">
  <h5 class="section-title">{$t("settings.bridge.title")}</h5>
  <div class="card">
    <p class="bridge-intro">{$t("settings.bridge.intro")}</p>

    {#if loading}
      <div class="setting-row">
        <span class="setting-label">{$t("common.loading")}</span>
      </div>
    {:else if !info || !info.enabled || info.port === 0 || !info.token}
      <div class="setting-row">
        <span class="setting-label">{$t("settings.bridge.unavailable")}</span>
      </div>
    {:else}
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t("settings.bridge.endpoint_label")}</span>
          <span class="setting-hint">{info.url}</span>
        </div>
        <button class="bridge-action" type="button" onclick={() => copy(info!.url, "url")}>
          {$t("settings.bridge.copy")}
        </button>
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t("settings.bridge.token_label")}</span>
          <span class="setting-hint token-display">
            {revealed ? info.token : maskedToken(info.token)}
          </span>
        </div>
        <div class="bridge-actions">
          <button
            class="bridge-action"
            type="button"
            onclick={() => (revealed = !revealed)}
          >
            {revealed ? $t("settings.bridge.hide") : $t("settings.bridge.reveal")}
          </button>
          <button
            class="bridge-action"
            type="button"
            onclick={() => copy(info!.token, "token")}
          >
            {$t("settings.bridge.copy")}
          </button>
        </div>
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t("settings.bridge.rotate_label")}</span>
          <span class="setting-hint">{$t("settings.bridge.rotate_hint")}</span>
        </div>
        <button
          class="bridge-action danger"
          type="button"
          disabled={rotating}
          onclick={rotate}
        >
          {rotating ? $t("settings.bridge.rotating") : $t("settings.bridge.rotate")}
        </button>
      </div>
    {/if}
  </div>
</section>

<style>
  .bridge-intro {
    margin: 0 0 12px;
    color: var(--color-text-muted, #95a0b7);
    font-size: 13px;
    line-height: 1.5;
  }
  .token-display {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    word-break: break-all;
  }
  .bridge-actions {
    display: inline-flex;
    gap: 8px;
  }
  .bridge-action {
    appearance: none;
    border: 1px solid rgba(255, 255, 255, 0.12);
    background: rgba(255, 255, 255, 0.04);
    color: inherit;
    padding: 6px 12px;
    border-radius: 8px;
    font: inherit;
    cursor: pointer;
  }
  .bridge-action:hover { background: rgba(255, 255, 255, 0.08); }
  .bridge-action:disabled { opacity: 0.5; cursor: not-allowed; }
  .bridge-action.danger { color: #ff8b6f; border-color: rgba(255, 139, 111, 0.3); }
  .bridge-action.danger:hover { background: rgba(255, 139, 111, 0.1); }
</style>
