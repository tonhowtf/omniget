<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { rpcTestConnection, rpcSyncIdleStats } from "$lib/rpc";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { failureShownInline } from "$lib/stores/settings-status-surfaces";
  import SettingGroup from "./rebrand/SettingGroup.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingsIcon from "./rebrand/SettingsIcon.svelte";
  import { RPC_DEFAULTS, rpcIdentityChanged, rpcTestOutcome, type RpcTestOutcome } from "./integrations/rpc-defaults";

  let settings = $derived(getSettings());
  let busy = $state(false);
  let testing = $state(false);
  let testResult = $state<RpcTestOutcome | null>(null);
  let resetting = $state(false);
  let canReset = $derived(testResult !== null && testResult !== "ok" && rpcIdentityChanged(settings?.rpc));

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
    } catch {
      if (!failureShownInline({ rpc: { enabled: true } })) showToast("error", $t("settings.save_failed") as string);
    } finally {
      busy = false;
    }
  }

  async function testConnection() {
    if (testing) return;
    testing = true;
    testResult = null;
    try {
      testResult = rpcTestOutcome(await rpcTestConnection());
    } catch {
      testResult = "fail";
    } finally {
      testing = false;
    }
  }

  async function resetIdentity() {
    if (resetting) return;
    resetting = true;
    try {
      await updateSettings({ rpc: { ...RPC_DEFAULTS } });
      testResult = null;
      await testConnection();
    } catch {
      if (!failureShownInline({ rpc: { ...RPC_DEFAULTS } })) showToast("error", $t("settings.save_failed") as string);
    } finally {
      resetting = false;
    }
  }
</script>

{#if settings}
  <SettingGroup id="rpc" icon="chat" title={$t("settings.integrations.rpc.title")} description={$t("settings.rpc.desc")}>
    <SettingRow id="rpc-enabled" label={$t("settings.rpc.enable")} help={$t("settings.rpc.privacy_note")} settingKey="settings.rpc.enable" status="rpc.enabled">
      {#snippet control(c)}
        <button
          type="button"
          class="toggle"
          class:on={settings.rpc.enabled}
          id={c.id}
          aria-labelledby={`${c.id}-label`}
          aria-describedby={c.describedBy}
          onclick={toggleEnabled}
          disabled={busy}
          role="switch"
          aria-checked={settings.rpc.enabled}
        ><span class="toggle-knob"></span></button>
      {/snippet}
    </SettingRow>
    {#if settings.rpc.enabled}
      <div class="rpc-test" data-setting-key="settings.rpc.test">
        <button class="btn btn-secondary btn-sm" type="button" aria-describedby="rpc-test-result" onclick={testConnection} disabled={testing} aria-busy={testing}>
          {$t("settings.rpc.test")}
        </button>
        <p class="rpc-test-result" id="rpc-test-result" role="status">
          {#if testing}
            {$t("settings.integrations.rpc.testing")}
          {:else if testResult === "ok"}
            <span class="ok"><SettingsIcon name="check" size={18} />{$t("settings.rpc.test_ok")}</span>
          {:else if testResult === "missing_app_id"}
            <span class="fail"><SettingsIcon name="alert" size={18} />{$t("settings.integrations.rpc.missing_app_id")}</span>
          {:else if testResult === "fail"}
            <span class="fail"><SettingsIcon name="alert" size={18} />{$t(canReset ? "settings.integrations.rpc.fail_custom" : "settings.rpc.test_fail")}</span>
          {/if}
        </p>
        {#if canReset}
          <button class="btn btn-secondary btn-sm" type="button" aria-describedby="rpc-test-result" onclick={resetIdentity} disabled={resetting || testing} aria-busy={resetting}>
            {$t("settings.integrations.rpc.reset")}
          </button>
        {/if}
      </div>
    {/if}
  </SettingGroup>
{/if}

<style>
  .rpc-test { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); padding-block: var(--space-2); border-block-start: 1px solid var(--separator); }
  .rpc-test-result { margin: 0; font-size: 12px; line-height: 18px; color: var(--text-muted); }
  .rpc-test-result:empty { display: none; }
  .rpc-test-result .ok, .rpc-test-result .fail { display: inline-flex; align-items: center; gap: var(--space-1); }
  .rpc-test-result .ok { color: var(--success); }
  .rpc-test-result .fail { color: var(--danger); }
</style>
