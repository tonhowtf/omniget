<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings, toggleBool, changeNumber } from "./settings-helpers";

  let settings = $derived(getSettings());

  let proxyHost = $state("");
  let proxyUsername = $state("");
  let proxyPassword = $state("");
  let proxyTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    if (settings) {
      proxyHost = settings.proxy?.host ?? "";
      proxyUsername = settings.proxy?.username ?? "";
      proxyPassword = settings.proxy?.password ?? "";
    }
  });

  async function changeProxyType(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    await updateSettings({ proxy: { proxy_type: value } });
  }

  function handleProxyHost(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    proxyHost = value;
    if (proxyTimer) clearTimeout(proxyTimer);
    proxyTimer = setTimeout(async () => {
      await updateSettings({ proxy: { host: value } });
    }, 800);
  }

  function handleProxyUsername(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    proxyUsername = value;
    if (proxyTimer) clearTimeout(proxyTimer);
    proxyTimer = setTimeout(async () => {
      await updateSettings({ proxy: { username: value } });
    }, 800);
  }

  function handleProxyPassword(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    proxyPassword = value;
    if (proxyTimer) clearTimeout(proxyTimer);
    proxyTimer = setTimeout(async () => {
      await updateSettings({ proxy: { password: value } });
    }, 800);
  }
</script>

{#if settings}
<section class="section">
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
      <div class="setting-row">
        <span class="setting-label">{$t('settings.proxy.type')}</span>
        <select class="select" value={settings.proxy?.proxy_type ?? 'http'} onchange={changeProxyType}>
          <option value="http">HTTP</option>
          <option value="https">HTTPS</option>
          <option value="socks5">SOCKS5</option>
        </select>
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <span class="setting-label">{$t('settings.proxy.host')}</span>
        <input type="text" class="input-text" value={proxyHost} oninput={handleProxyHost} placeholder="127.0.0.1" spellcheck="false" />
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <span class="setting-label">{$t('settings.proxy.port')}</span>
        <input type="number" class="input-number" min="1" max="65535" value={settings.proxy?.port ?? 8080} onchange={(e) => changeNumber("proxy", "port", e)} />
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <span class="setting-label">{$t('settings.proxy.username')}</span>
        <input type="text" class="input-text" value={proxyUsername} oninput={handleProxyUsername} placeholder="" spellcheck="false" />
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <span class="setting-label">{$t('settings.proxy.password')}</span>
        <input type="password" class="input-text" value={proxyPassword} oninput={handleProxyPassword} placeholder="" />
      </div>
    {/if}
  </div>
</section>
{/if}
