<script lang="ts">
  import { t } from "$lib/i18n";
  import { createSettingsDraft } from "$lib/stores/settings-drafts.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingsSaveBar from "./rebrand/SettingsSaveBar.svelte";

  // Proxy is a transactional form (Save/Discard). The password never comes back
  // from the backend: `has_password` only says one is stored, and the draft carries
  // a keep/set/remove operation that lives in memory until it is saved.

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

  let form = $state<HTMLFormElement | null>(null);
  let proxy = $derived(draft.value);

  function onPassword(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    draft.value.password = value;
    draft.value.passwordOp = value ? "set" : "keep";
  }

  function save() {
    // Port 1–65535 and a host are required only while the proxy is on.
    if (form && !form.reportValidity()) return;
    void draft.save();
  }
</script>

<form class="network-form" bind:this={form} novalidate onsubmit={(e) => { e.preventDefault(); save(); }}>
  <SettingRow id="settings-proxy-enabled" label={$t("settings.proxy.enabled")} settingKey="settings.proxy.enabled">
    {#snippet control({ id, describedBy })}
      <button {id} type="button" class="toggle" class:on={proxy.enabled} role="switch" aria-checked={proxy.enabled} aria-labelledby={`${id}-label`} aria-describedby={describedBy}
        onclick={() => { draft.value.enabled = !proxy.enabled; }}><span class="toggle-knob"></span></button>
    {/snippet}
  </SettingRow>

  {#if proxy.enabled}
    <SettingRow id="settings-proxy-type" label={$t("settings.proxy.type")} settingKey="settings.proxy.type">
      {#snippet control({ id, describedBy })}
        <select {id} class="select" aria-describedby={describedBy} bind:value={draft.value.proxy_type}>
          <option value="http">HTTP</option>
          <option value="https">HTTPS</option>
          <option value="socks5">SOCKS5</option>
        </select>
      {/snippet}
    </SettingRow>
    <SettingRow id="settings-proxy-host" label={$t("settings.proxy.host")} settingKey="settings.proxy.host">
      {#snippet control({ id, describedBy })}
        <input {id} type="text" class="input-text" dir="ltr" spellcheck="false" autocomplete="off" required placeholder="127.0.0.1"
          aria-describedby={describedBy} bind:value={draft.value.host} />
      {/snippet}
    </SettingRow>
    <SettingRow id="settings-proxy-port" label={$t("settings.proxy.port")} settingKey="settings.proxy.port">
      {#snippet control({ id, describedBy })}
        <input {id} type="number" class="input-number" dir="ltr" min="1" max="65535" step="1" required
          aria-describedby={describedBy} bind:value={draft.value.port} />
      {/snippet}
    </SettingRow>
    <SettingRow id="settings-proxy-username" label={$t("settings.proxy.username")} settingKey="settings.proxy.username">
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
</form>

<style>
  .network-form { display: flex; flex-direction: column; min-inline-size: 0; }
  .password-field { display: flex; align-items: center; gap: var(--space-2); inline-size: 100%; }
  .password-field input { flex: 1 1 auto; min-inline-size: 0; }
</style>
