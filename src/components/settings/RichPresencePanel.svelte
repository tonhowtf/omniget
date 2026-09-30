<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { createSettingsDraft } from "$lib/stores/settings-drafts.svelte";
  import { rpcTestConnection, rpcSyncIdleStats } from "$lib/rpc";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { failureShownInline } from "$lib/stores/settings-status-surfaces";
  import SettingGroup from "./rebrand/SettingGroup.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingsSaveBar from "./rebrand/SettingsSaveBar.svelte";

  let settings = $derived(getSettings());
  let busy = $state(false);
  let testResult = $state<"ok" | "fail" | null>(null);

  // Application ID and image key are free text: edited as a draft and written
  // together with "Save changes".
  const advanced = createSettingsDraft({
    id: "rpc",
    read: (s) => ({
      app_id: s.rpc?.app_id ?? "",
      large_image_key: s.rpc?.large_image_key ?? "",
    }),
    toPatch: (d) => ({
      rpc: {
        app_id: d.app_id.trim(),
        large_image_key: d.large_image_key.trim(),
      },
    }),
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
    } catch {
      // The row's SettingStatus (rpc.enabled) already shows the failure inline.
      if (!failureShownInline({ rpc: { enabled: true } })) showToast("error", $t("settings.save_failed") as string);
    } finally {
      busy = false;
    }
  }

  async function saveAdvanced() {
    if (advanced.saving) return;
    if (await advanced.save()) void rpcSyncIdleStats();
  }

  async function testConnection() {
    if (busy) return;
    busy = true;
    testResult = null;
    try {
      const res = await rpcTestConnection();
      testResult = res.ok ? "ok" : "fail";
    } catch {
      testResult = "fail";
    } finally {
      busy = false;
    }
  }
</script>

{#if settings}
  <SettingGroup id="rpc" title={$t("settings.rpc.title")} description={$t("settings.rpc.desc")}>
    <SettingRow id="rpc-enabled" label={$t("settings.rpc.enable")} help={$t("settings.rpc.privacy_note")} settingKey="settings.rpc.enable" status="rpc.enabled">
      {#snippet control(c)}
        <button
          type="button"
          class="toggle"
          class:on={settings.rpc.enabled}
          id={c.id}
          aria-describedby={c.describedBy}
          onclick={toggleEnabled}
          disabled={busy}
          role="switch"
          aria-checked={settings.rpc.enabled}
          aria-label={$t("settings.rpc.enable") as string}
        ><span class="toggle-knob"></span></button>
      {/snippet}
    </SettingRow>
    <SettingRow id="rpc-test" label={$t("settings.rpc.test")} help={$t("settings.rpc.test_desc")} settingKey="settings.rpc.test">
      {#snippet control(c)}
        <button
          class="button"
          id={c.id}
          aria-describedby={[c.describedBy, "rpc-test-result"].filter(Boolean).join(" ")}
          onclick={testConnection}
          disabled={busy || !settings.rpc.enabled}
          aria-busy={busy}
          type="button"
        >{$t("settings.rpc.test_button")}</button>
      {/snippet}
    </SettingRow>
    <p class="rpc-test-result" id="rpc-test-result" role="status">
      {#if testResult === "ok"}
        <span class="ok">{$t("settings.rpc.test_ok")}</span>
      {:else if testResult === "fail"}
        <span class="fail">{$t("settings.rpc.test_fail")}</span>
      {/if}
    </p>
  </SettingGroup>

  <SettingGroup id="rpc-advanced" title={$t("settings.rpc.advanced")} advanced>
    <form class="rpc-form" onsubmit={(e) => { e.preventDefault(); void saveAdvanced(); }}>
      <SettingRow id="rpc-app-id" label={$t("settings.rpc.app_id")} help={$t("settings.rpc.app_id_hint")} settingKey="settings.rpc.app_id">
        {#snippet control(c)}
          <input id={c.id} aria-describedby={c.describedBy} type="text" class="rpc-input" dir="ltr" bind:value={advanced.value.app_id} spellcheck="false" autocomplete="off" />
        {/snippet}
      </SettingRow>
      <SettingRow id="rpc-large-image" label={$t("settings.rpc.large_image")} help={$t("settings.rpc.large_image_hint")} settingKey="settings.rpc.large_image">
        {#snippet control(c)}
          <input id={c.id} aria-describedby={c.describedBy} type="text" class="rpc-input" dir="ltr" bind:value={advanced.value.large_image_key} spellcheck="false" autocomplete="off" />
        {/snippet}
      </SettingRow>
      <SettingsSaveBar dirty={advanced.dirty} saving={advanced.saving} error={advanced.error} onSave={() => void saveAdvanced()} onDiscard={() => advanced.discard()} />
    </form>
  </SettingGroup>
{/if}

<style>
  .rpc-test-result { margin: 0; font-size: 12px; line-height: 18px; }
  .rpc-test-result:empty { display: none; }
  .rpc-test-result .ok { color: var(--success); }
  .rpc-test-result .fail { color: var(--danger); }
  .rpc-form { display: flex; flex-direction: column; min-inline-size: 0; }
  .rpc-input {
    width: 100%;
    padding: 8px 10px;
    background: var(--button-elevated);
    border: none;
    border-radius: calc(var(--border-radius) / 2);
    color: var(--text);
    font-family: ui-monospace, monospace;
    font-size: var(--text-sm);
    outline: none;
  }
  .rpc-input:focus-visible {
    border-color: var(--accent);
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
</style>
