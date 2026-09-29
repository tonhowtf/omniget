<script lang="ts">
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings, saveSettings, toggleBool, changeNumber } from "./settings-helpers";

  let settings = $derived(getSettings());

  let proxySaveError = $state(false);
  let proxyHost = $state("");
  let proxyUsername = $state("");
  let proxyPassword = $state("");

  type ProxyField = "host" | "username" | "password";
  let proxyTimer: ReturnType<typeof setTimeout> | null = null;
  let pendingProxy = $state<Partial<Record<ProxyField, string>>>({});
  let proxySavePromise: Promise<void> | null = null;
  let proxySaveRequested = false;

  $effect(() => {
    if (settings) {
      if (!("host" in pendingProxy)) proxyHost = settings.proxy?.host ?? "";
      if (!("username" in pendingProxy)) proxyUsername = settings.proxy?.username ?? "";
      if (!("password" in pendingProxy)) proxyPassword = settings.proxy?.password ?? "";
    }
  });

  function flushProxyFields(): Promise<void> {
    if (proxyTimer) clearTimeout(proxyTimer);
    proxyTimer = null;
    proxySaveRequested = true;
    if (proxySavePromise) return proxySavePromise;
    proxySavePromise = (async () => {
      while (proxySaveRequested) {
        proxySaveRequested = false;
        const snapshot = { ...pendingProxy };
        if (!Object.keys(snapshot).length) continue;
        try {
          await updateSettings({ proxy: snapshot });
          for (const field of Object.keys(snapshot) as ProxyField[]) {
            if (pendingProxy[field] === snapshot[field]) delete pendingProxy[field];
          }
          proxySaveError = false;
        } catch {
          proxySaveError = true;
          return;
        }
      }
    })().finally(() => { proxySavePromise = null; });
    return proxySavePromise;
  }

  function queueProxyField(field: ProxyField, value: string) {
    pendingProxy[field] = value;
    if (proxyTimer) clearTimeout(proxyTimer);
    proxyTimer = setTimeout(() => { void flushProxyFields(); }, 800);
  }

  onDestroy(() => { void flushProxyFields(); });

  async function changeProxyType(e: Event) {
    const input = e.target as HTMLSelectElement;
    if (!await saveSettings({ proxy: { proxy_type: input.value } })) {
      input.value = getSettings()?.proxy?.proxy_type ?? "http";
    }
  }

  function handleProxyHost(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    proxyHost = value;
    queueProxyField("host", value);
  }

  function handleProxyUsername(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    proxyUsername = value;
    queueProxyField("username", value);
  }

  function handleProxyPassword(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    proxyPassword = value;
    queueProxyField("password", value);
  }
</script>

{#if settings}
<section class="section" data-setting-key="settings.proxy.title">
  <h5 class="section-title">{$t('settings.proxy.title')}</h5>
  <div class="card">
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.proxy.enabled')}</span>
      </div>
      <button
        class="toggle"
        class:on={settings.proxy?.enabled}
        onclick={() => toggleBool("proxy", "enabled", settings.proxy?.enabled ?? false)}
        role="switch"
        aria-checked={settings.proxy?.enabled ?? false}
        aria-label={$t('settings.proxy.enabled') as string}
      >
        <span class="toggle-knob"></span>
      </button>
    </div>
    {#if settings.proxy?.enabled}
      <div class="divider"></div>
      <div class="setting-row" data-setting-key="settings.proxy.type">
        <label class="setting-label" for="settings-proxy-type">{$t('settings.proxy.type')}</label>
        <select id="settings-proxy-type" class="select" value={settings.proxy?.proxy_type ?? 'http'} onchange={changeProxyType}>
          <option value="http">HTTP</option>
          <option value="https">HTTPS</option>
          <option value="socks5">SOCKS5</option>
        </select>
      </div>
      <div class="divider"></div>
      <div class="setting-row" data-setting-key="settings.proxy.host">
        <label class="setting-label" for="settings-proxy-host">{$t('settings.proxy.host')}</label>
        <input type="text" class="input-text" id="settings-proxy-host" value={proxyHost} oninput={handleProxyHost} onchange={() => flushProxyFields()} placeholder="127.0.0.1" spellcheck="false" />
      </div>
      <div class="divider"></div>
      <div class="setting-row" data-setting-key="settings.proxy.port">
        <label class="setting-label" for="settings-proxy-port">{$t('settings.proxy.port')}</label>
        <input id="settings-proxy-port" type="number" class="input-number" min="1" max="65535" value={settings.proxy?.port ?? 8080} onchange={(e) => changeNumber("proxy", "port", e)} />
      </div>
      <div class="divider"></div>
      <div class="setting-row" data-setting-key="settings.proxy.username">
        <label class="setting-label" for="settings-proxy-username">{$t('settings.proxy.username')}</label>
        <input type="text" class="input-text" id="settings-proxy-username" value={proxyUsername} oninput={handleProxyUsername} onchange={() => flushProxyFields()} placeholder="" spellcheck="false" />
      </div>
      <div class="divider"></div>
      <div class="setting-row" data-setting-key="settings.proxy.password">
        <label class="setting-label" for="settings-proxy-password">{$t('settings.proxy.password')}</label>
        <input type="password" class="input-text" id="settings-proxy-password" value={proxyPassword} oninput={handleProxyPassword} onchange={() => flushProxyFields()} placeholder="" />
      </div>
    {/if}
  </div>
  {#if proxySaveError}
    <div role="alert">
      <p>{$t('settings.proxy.save_failed')}</p>
      <button class="button" onclick={() => { void flushProxyFields(); }}>{$t('error.remedy.retry')}</button>
    </div>
  {/if}
</section>
{/if}
