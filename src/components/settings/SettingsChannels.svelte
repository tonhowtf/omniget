<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t, locale as i18nLocale } from "$lib/i18n";
  import { get } from "svelte/store";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SettingGroup from "./rebrand/SettingGroup.svelte";
  import SettingsEmptyState from "./rebrand/SettingsEmptyState.svelte";

  type Channel = {
    id: string;
    url: string;
    title: string;
    added_at_ms: number;
    last_checked_ms: number | null;
    enabled: boolean;
    auto_download: boolean;
    interval_minutes: number;
  };

  let channels = $state<Channel[]>([]);
  let newUrl = $state("");
  let adding = $state(false);
  let loading = $state(true);
  let loadError = $state(false);
  let busyIds = $state<string[]>([]);
  let checkingIds = $state<string[]>([]);

  function begin(id: string): boolean {
    if (busyIds.includes(id)) return false;
    busyIds = [...busyIds, id];
    return true;
  }

  function finish(id: string) {
    busyIds = busyIds.filter((value) => value !== id);
  }

  async function syncTray() {
    try {
      await invoke("sync_channels_tray", {
        header: $t("settings.channels.tray_header"),
        channels: channels.map((c) => [c.id, c.title]),
      });
    } catch {
    }
  }

  async function load() {
    loading = true;
    loadError = false;
    try {
      channels = await invoke<Channel[]>("channels_list");
      await syncTray();
    } catch {
      loadError = true;
    } finally {
      loading = false;
    }
  }

  onMount(load);

  async function addChannel() {
    const url = newUrl.trim();
    if (!url || adding || loading || loadError) return;
    adding = true;
    try {
      await invoke("channel_add", { url });
      newUrl = "";
      await load();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : $t("common.error"));
    } finally {
      adding = false;
    }
  }

  async function removeChannel(id: string) {
    if (busyIds.includes(id) || !(await confirm($t("settings.channels.remove_confirm"))) || !begin(id)) return;
    try {
      await invoke("channel_remove", { id });
      await load();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : $t("common.error"));
    } finally {
      finish(id);
    }
  }

  async function patch(
    id: string,
    fields: { enabled?: boolean; autoDownload?: boolean; intervalMinutes?: number },
  ) {
    if (!begin(id)) return;
    try {
      await invoke("channel_update", { id, ...fields });
      await load();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : $t("common.error"));
    } finally {
      finish(id);
    }
  }

  async function checkNow(id: string) {
    if (!begin(id)) return;
    checkingIds = [...checkingIds, id];
    try {
      const n = await invoke<number>("channel_check_now", { id });
      showToast(
        "success",
        $t("settings.channels.checked", { count: n }) as string,
      );
      await load();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : $t("common.error"));
    } finally {
      checkingIds = checkingIds.filter((value) => value !== id);
      finish(id);
    }
  }

  function lastChecked(ms: number | null): string {
    if (!ms) return $t("settings.channels.never");
    const seconds = Math.max(0, Math.floor((Date.now() - ms) / 1000));
    const unit: Intl.RelativeTimeFormatUnit = seconds < 60 ? "second" : seconds < 3600 ? "minute" : seconds < 86400 ? "hour" : "day";
    const divisor = unit === "second" ? 1 : unit === "minute" ? 60 : unit === "hour" ? 3600 : 86400;
    return new Intl.RelativeTimeFormat(get(i18nLocale) || "en", { numeric: "auto" }).format(-Math.floor(seconds / divisor), unit);
  }

  function onIntervalChange(id: string, e: Event) {
    const input = e.target as HTMLInputElement;
    const value = Number(input.value);
    if (!Number.isInteger(value) || value < 5 || value > 1440) {
      input.value = String(channels.find((channel) => channel.id === id)?.interval_minutes ?? 60);
      showToast("error", $t("settings.channels.interval_invalid"));
      return;
    }
    void patch(id, { intervalMinutes: value });
  }
</script>

<SettingGroup id="channels" icon="film" title={$t('settings.channels.title')} description={$t('settings.channels.empty_hint')} advanced>
  <div class="add-block" data-setting-key="settings.channels.add">
    <div class="add-row">
      <input
        type="text"
        class="input-text"
        placeholder={$t('settings.channels.add_placeholder')}
        aria-label={$t('settings.channels.add_placeholder')}
        aria-describedby="channels-add-hint"
        dir="ltr"
        disabled={adding}
        bind:value={newUrl}
        onkeydown={(e) => { if (e.key === "Enter") addChannel(); }}
      />
      <button class="add-btn" disabled={!newUrl.trim() || adding || loading || loadError} onclick={addChannel}>
        {adding ? $t('common.loading') : $t('settings.channels.add')}
      </button>
    </div>
    <span class="setting-path add-hint" id="channels-add-hint">{$t('settings.channels.add_hint')}</span>
  </div>

  {#if loadError && !loading}
    <SettingsEmptyState error title={$t('settings.channels.load_failed')} onRetry={() => void load()} />
  {/if}
  {#if loading && channels.length === 0}
    <p class="channels-loading" role="status" aria-busy="true">{$t('settings.rebrand.state.loading')}</p>
  {:else if channels.length === 0}
    {#if !loadError}
      <SettingsEmptyState title={$t('settings.channels.empty_title')} />
    {/if}
  {:else}
    <div class="card">
      {#each channels as ch, i (ch.id)}
        {#if i > 0}<div class="divider"></div>{/if}
        <div class="channel" aria-busy={busyIds.includes(ch.id)}>
          <div class="channel-head">
            <div class="channel-id">
              <span class="channel-title" title={ch.url}>{ch.title}</span>
              <span class="setting-path">{$t('settings.channels.last_checked', { when: lastChecked(ch.last_checked_ms) })}</span>
            </div>
            <div class="channel-actions">
              <button class="link-btn" disabled={busyIds.includes(ch.id)} onclick={() => checkNow(ch.id)}>{checkingIds.includes(ch.id) ? $t('settings.channels.checking') : $t('settings.channels.check_now')}</button>
              <button class="link-btn danger" disabled={busyIds.includes(ch.id)} onclick={() => removeChannel(ch.id)}>{$t('settings.channels.remove')}</button>
            </div>
          </div>
          <div class="channel-controls">
            <label class="ctrl">
              <button
                class="toggle"
                disabled={busyIds.includes(ch.id)}
                class:on={ch.enabled}
                onclick={() => patch(ch.id, { enabled: !ch.enabled })}
                role="switch"
                aria-checked={ch.enabled}
                aria-label={$t('settings.channels.enabled') as string}
              ><span class="toggle-knob"></span></button>
              <span>{$t('settings.channels.enabled')}</span>
            </label>
            <label class="ctrl">
              <button
                class="toggle"
                disabled={busyIds.includes(ch.id)}
                class:on={ch.auto_download}
                onclick={() => patch(ch.id, { autoDownload: !ch.auto_download })}
                role="switch"
                aria-checked={ch.auto_download}
                aria-label={$t('settings.channels.auto_download') as string}
              ><span class="toggle-knob"></span></button>
              <span>{$t('settings.channels.auto_download')}</span>
            </label>
            <label class="ctrl interval">
              <span>{$t('settings.channels.interval')}</span>
              <input
                type="number"
                class="input-number"
                disabled={busyIds.includes(ch.id)}
                min="5"
                max="1440"
                value={ch.interval_minutes}
                onchange={(e) => onIntervalChange(ch.id, e)}
              />
            </label>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</SettingGroup>

<style>
  .channels-loading { margin: 0; padding-block: var(--space-3); font-size: 13px; color: var(--text-muted); }
  .link-btn:disabled { opacity: 0.5; cursor: not-allowed; }

  .add-block { padding-block: var(--space-2); min-inline-size: 0; }

  .add-row {
    display: flex;
    gap: 8px;
    width: 100%;
  }

  .add-row .input-text {
    flex: 1;
    min-width: 0;
  }

  .add-btn {
    flex-shrink: 0;
    padding: 0 16px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 600;
    cursor: pointer;
  }

  .add-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .add-hint {
    display: block;
    margin-top: 8px;
  }



  .channel {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 4px 0;
  }

  .channel-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .channel-id {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .channel-title {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .channel-actions {
    display: flex;
    gap: 12px;
    flex-shrink: 0;
  }

  .link-btn {
    background: none;
    border: none;
    color: var(--accent);
    cursor: pointer;
    font-size: 13px;
    padding: 0;
  }

  .link-btn.danger {
    color: var(--error);
  }

  .channel-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 16px;
  }

  .ctrl {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }

  .ctrl.interval .input-number {
    width: 72px;
  }
</style>
