<script lang="ts">
  import { tick } from "svelte";
  import { t } from "$lib/i18n";
  import { createSettingsDraft } from "$lib/stores/settings-drafts.svelte";
  import { getSettings, saveSettings as updateSettings } from "./settings-helpers";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingStatus from "./rebrand/SettingStatus.svelte";
  import SettingsSaveBar from "./rebrand/SettingsSaveBar.svelte";
  import GeneralChoice from "./rebrand/GeneralChoice.svelte";
  import AdvancedDisclosure from "./advanced/AdvancedDisclosure.svelte";
  import { proxyModeStep, type ProxyMode } from "./advanced/proxy-mode";

  type ProxyDraft = {
    enabled: boolean;
    proxy_type: string;
    host: string;
    port: number;
    username: string;
    password: string;
    passwordOp: "keep" | "set" | "remove";
    hasPassword: boolean;
  };

  const draft = createSettingsDraft<ProxyDraft & Record<string, unknown>>({
    id: "proxy",
    read: (s) => ({
      enabled: s.proxy?.enabled ?? false,
      proxy_type: s.proxy?.proxy_type ?? "http",
      host: s.proxy?.host ?? "",
      port: s.proxy?.port ?? 8080,
      username: s.proxy?.username ?? "",
      password: "",
      passwordOp: "keep",
      hasPassword: s.proxy?.has_password ?? false,
    }),
    toPatch: (d) => ({
      proxy: { enabled: d.enabled, proxy_type: d.proxy_type, host: d.host.trim(), port: d.port, username: d.username },
    }),
    secretOperations: (d) => ({
      proxyPassword: d.passwordOp === "set" ? { op: "set", value: d.password } : { op: d.passwordOp },
    }),
  });

  let settings = $derived(getSettings());
  let form = $state<HTMLFormElement | null>(null);
  let proxy = $derived(draft.value);

  function onPassword(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    draft.value.password = value;
    draft.value.passwordOp = value ? "set" : "keep";
  }

  function save() {
    if (form && !form.reportValidity()) return;
    void draft.save();
  }

  async function chooseMode(mode: string) {
    const step = proxyModeStep(mode as ProxyMode, settings?.proxy?.enabled ?? false, draft.value.host);
    if (step.discard) draft.discard();
    draft.value.enabled = step.enabled;
    if (step.save) { void draft.save(); return; }
    if (step.focusHost) {
      await tick();
      document.getElementById("settings-proxy-host")?.focus();
    }
  }
</script>

{#snippet drawDirect()}
  <rect class="pc" x="8" y="15" width="18" height="13" rx="2" />
  <path class="pc-foot" d="M13 33h8" />
  <path class="wire" d="M28 22h22" />
  <circle class="net" cx="61" cy="22" r="10" />
  <path class="net-line" d="M51 22h20M61 12c4 3 4 17 0 20M61 12c-4 3-4 17 0 20" />
{/snippet}

{#snippet drawProxy()}
  <rect class="pc" x="4" y="15" width="16" height="12" rx="2" />
  <path class="pc-foot" d="M9 32h6" />
  <path class="wire" d="M21 21h9M48 21h9" />
  <rect class="relay" x="30" y="13" width="18" height="16" rx="3" />
  <path class="relay-line" d="M34 19h10M34 23h10" />
  <circle class="net" cx="66" cy="21" r="9" />
  <path class="net-line" d="M57 21h18M66 12c3.500 3 3.500 15 0 18M66 12c-3.500 3-3.500 15 0 18" />
{/snippet}

<form class="network-form" bind:this={form} novalidate onsubmit={(e) => { e.preventDefault(); save(); }}>
  <div data-setting-key="settings.proxy.enabled">
    <GeneralChoice
      name="settings-proxy-mode"
      label={$t("settings.advanced.proxy_mode") as string}
      value={proxy.enabled ? "proxy" : "direct"}
      options={[
        { value: "direct", draw: drawDirect, label: $t("settings.advanced.proxy_direct") as string, help: $t("settings.advanced.proxy_direct_desc") as string },
        { value: "proxy", draw: drawProxy, label: $t("settings.advanced.proxy_use") as string, help: $t("settings.advanced.proxy_use_desc") as string },
      ]}
      onchange={chooseMode}
    />
    <SettingStatus path="proxy.enabled" />
  </div>

  {#if proxy.enabled}
    <SettingRow id="settings-proxy-host" label={$t("settings.advanced.proxy_address")} help={$t("settings.advanced.proxy_address_desc")} settingKey="settings.proxy.host">
      {#snippet control({ id, describedBy })}
        <span class="address">
          <input {id} type="text" class="input-text" dir="ltr" spellcheck="false" autocomplete="off" required placeholder="127.0.0.1"
            aria-describedby={describedBy} bind:value={draft.value.host} />
          <span class="colon" aria-hidden="true">:</span>
          <input id="settings-proxy-port" data-setting-key="settings.proxy.port" type="number" class="input-number port" dir="ltr" min="1" max="65535" step="1" required
            aria-label={$t("settings.proxy.port") as string} bind:value={draft.value.port} />
        </span>
      {/snippet}
    </SettingRow>
    <SettingRow id="settings-proxy-type" label={$t("settings.proxy.type")} settingKey="settings.proxy.type">
      {#snippet control({ id, describedBy })}
        <select {id} class="select" aria-describedby={describedBy} bind:value={draft.value.proxy_type}>
          <option value="http">HTTP</option>
          <option value="https">HTTPS</option>
          <option value="socks5">SOCKS5</option>
        </select>
      {/snippet}
    </SettingRow>
    <SettingRow id="settings-proxy-username" label={$t("settings.proxy.username")} help={$t("settings.advanced.proxy_login_desc")} settingKey="settings.proxy.username">
      {#snippet control({ id, describedBy })}
        <input {id} type="text" class="input-text" dir="ltr" spellcheck="false" autocomplete="off"
          aria-describedby={describedBy} bind:value={draft.value.username} />
      {/snippet}
    </SettingRow>
    <SettingRow id="settings-proxy-password" label={$t("settings.proxy.password")} settingKey="settings.proxy.password">
      {#snippet control({ id, describedBy })}
        <span class="password-field">
          <input {id} type="password" class="input-text" dir="ltr" autocomplete="new-password" aria-describedby={describedBy}
            placeholder={proxy.hasPassword && proxy.passwordOp === "keep" ? "••••••••" : ""}
            value={proxy.password} oninput={onPassword} />
          {#if proxy.hasPassword && proxy.passwordOp !== "remove"}
            <button type="button" class="btn btn-secondary btn-sm" onclick={() => { draft.value.password = ""; draft.value.passwordOp = "remove"; }}>
              {$t("settings.rebrand.downloads.remove")}
            </button>
          {/if}
        </span>
      {/snippet}
    </SettingRow>
  {/if}

  <SettingsSaveBar dirty={draft.dirty} saving={draft.saving} error={draft.error} onSave={save} onDiscard={() => draft.discard()} />

  {#if settings?.proxy?.enabled && !draft.dirty}
    <SettingRow id="settings-proxy-check" label={$t("settings.advanced.proxy_check")} help={$t("settings.advanced.proxy_check_desc")} settingKey="settings.advanced.proxy_check">
      {#snippet control({ id, describedBy })}
        <a {id} class="btn btn-secondary btn-sm" href="/downloads?tool=proxy-check" aria-describedby={describedBy}>{$t("settings.advanced.proxy_check_action")}</a>
      {/snippet}
    </SettingRow>
  {/if}
</form>

{#if settings}
  <AdvancedDisclosure id="settings-connection">
    <SettingRow id="setting-advanced-user_agent" label={$t("settings.advanced.user_agent")} help={$t("settings.advanced.user_agent_desc")}
      settingKey="settings.advanced.user_agent" status="advanced.user_agent">
      {#snippet control({ id, describedBy })}
        <input {id} aria-describedby={describedBy} type="text" class="input-text" dir="ltr" spellcheck="false" placeholder={$t("settings.advanced.user_agent_placeholder")}
          value={settings.advanced?.user_agent ?? ""} onchange={(e) => updateSettings({ advanced: { user_agent: (e.target as HTMLInputElement).value.trim() } })} />
      {/snippet}
    </SettingRow>
  </AdvancedDisclosure>
{/if}

<style>
  .network-form { display: flex; flex-direction: column; min-inline-size: 0; }
  .address { display: flex; align-items: center; gap: var(--space-1); inline-size: 100%; }
  .address input:first-child { flex: 1 1 auto; min-inline-size: 0; }
  .address .port { flex: 0 0 auto; inline-size: 80px !important; }
  .colon { color: var(--text-muted); }
  .password-field { display: flex; align-items: center; gap: var(--space-2); inline-size: 100%; }
  .password-field input { flex: 1 1 auto; min-inline-size: 0; }
  .pc, .relay { fill: var(--surface-hi-src, var(--surface-src)); stroke: var(--tertiary); }
  .relay { stroke: var(--accent); }
  .pc-foot, .relay-line { fill: none; stroke: var(--tertiary); stroke-width: 2; stroke-linecap: round; }
  .wire { fill: none; stroke: var(--accent); stroke-width: 1.500; stroke-linecap: round; stroke-dasharray: 3 2; }
  .net { fill: none; stroke: var(--tertiary); }
  .net-line { fill: none; stroke: var(--tertiary); opacity: 0.7; }
</style>
