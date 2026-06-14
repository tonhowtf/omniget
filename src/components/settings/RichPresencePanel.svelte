<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { rpcTestConnection, rpcSyncIdleStats } from "$lib/rpc";
  import { showToast } from "$lib/stores/toast-store.svelte";

  let settings = $derived(getSettings());
  let busy = $state(false);
  let testResult = $state<"ok" | "fail" | null>(null);
  let appIdInput = $state("");
  let imageKeyInput = $state("");

  $effect(() => {
    if (settings) {
      appIdInput = settings.rpc?.app_id ?? "";
      imageKeyInput = settings.rpc?.large_image_key ?? "";
    }
  });

  async function toggleEnabled() {
    if (!settings || busy) return;
    busy = true;
    try {
      await updateSettings({ rpc: { enabled: !settings.rpc.enabled } });
      if (!settings.rpc.enabled) {
        testResult = null;
      } else {
        void rpcSyncIdleStats();
      }
    } finally {
      busy = false;
    }
  }

  async function saveAdvanced() {
    if (!settings || busy) return;
    busy = true;
    try {
      await updateSettings({
        rpc: {
          app_id: appIdInput.trim(),
          large_image_key: imageKeyInput.trim(),
        },
      });
      void rpcSyncIdleStats();
      showToast("success", $t("settings.rpc.saved") as string);
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  async function testConnection() {
    if (busy) return;
    busy = true;
    testResult = null;
    try {
      const res = await rpcTestConnection();
      testResult = res.ok ? "ok" : "fail";
      if (!res.ok) {
        showToast("error", $t("settings.rpc.test_fail") as string);
      } else {
        showToast("success", $t("settings.rpc.test_ok") as string);
      }
    } catch (e) {
      testResult = "fail";
      showToast("error", e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
</script>

{#if settings}
  <div class="card">
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t("settings.rpc.enable")}</span>
        <span class="setting-path">{$t("settings.rpc.privacy_note")}</span>
      </div>
      <button
        class="toggle"
        class:on={settings.rpc.enabled}
        onclick={toggleEnabled}
        disabled={busy}
        role="switch"
        aria-checked={settings.rpc.enabled}
        aria-label={$t("settings.rpc.enable") as string}
      ><span class="toggle-knob"></span></button>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t("settings.rpc.test")}</span>
        <span class="setting-path">{$t("settings.rpc.test_desc")}</span>
      </div>
      <div class="rpc-test-row">
        <button
          class="button"
          onclick={testConnection}
          disabled={busy || !settings.rpc.enabled}
          type="button"
        >{$t("settings.rpc.test_button")}</button>
        {#if testResult === "ok"}
          <span class="badge ok">✓</span>
        {:else if testResult === "fail"}
          <span class="badge fail">✗</span>
        {/if}
      </div>
    </div>
  </div>

  <p class="settings-subsection-head">{$t("settings.rpc.advanced")}</p>
  <div class="card">
      <div class="setting-row stack">
        <div class="setting-col">
          <span class="setting-label">{$t("settings.rpc.app_id")}</span>
          <span class="setting-path">{$t("settings.rpc.app_id_hint")}</span>
        </div>
        <input type="text" class="rpc-input" bind:value={appIdInput} spellcheck="false" />
      </div>
      <div class="divider"></div>
      <div class="setting-row stack">
        <div class="setting-col">
          <span class="setting-label">{$t("settings.rpc.large_image")}</span>
          <span class="setting-path">{$t("settings.rpc.large_image_hint")}</span>
        </div>
        <input type="text" class="rpc-input" bind:value={imageKeyInput} spellcheck="false" />
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <span class="setting-label"></span>
        <button class="button" onclick={saveAdvanced} disabled={busy} type="button">
          {$t("settings.rpc.save")}
        </button>
      </div>
    </div>
{/if}

<style>
  .rpc-test-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .badge {
    font-size: 14px;
    font-weight: 700;
  }
  .badge.ok { color: var(--green, #4ade80); }
  .badge.fail { color: var(--red, #f87171); }
  .rpc-input {
    width: 100%;
    padding: 8px 10px;
    background: var(--button-elevated);
    border: 1px solid var(--input-border, var(--border));
    border-radius: calc(var(--border-radius) / 2);
    color: var(--text);
    font-family: ui-monospace, monospace;
    font-size: 12.5px;
    outline: none;
  }
  .rpc-input:focus-visible {
    border-color: var(--accent);
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .setting-row.stack {
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }
</style>
