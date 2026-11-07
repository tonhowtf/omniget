<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { saveSettings } from "./settings-helpers";
  import { getPendingEffects } from "$lib/stores/settings-store.svelte";
  import SettingGroup from "./rebrand/SettingGroup.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingsEmptyState from "./rebrand/SettingsEmptyState.svelte";
  import SettingsIcon from "./rebrand/SettingsIcon.svelte";
  import SettingsDependencies from "./SettingsDependencies.svelte";
  import IntegrationsDisclosure from "./integrations/IntegrationsDisclosure.svelte";

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
  let loadError = $state(false);
  let confirmRotation = $state(false);
  let destroyed = false;

  async function refresh() {
    loading = true;
    loadError = false;
    try {
      info = await invoke<BridgeInfo>("get_bridge_info");
    } catch {
      loadError = true;
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
    } catch {
      showToast("error", $t("common.error") as string);
    }
  }

  async function rotate() {
    if (!confirmRotation || rotating || pairing) return;
    rotating = true;
    try {
      info = await invoke<BridgeInfo>("rotate_bridge_token");
      revealed = false;
      confirmRotation = false;
      pairResult = "none";
      showToast("success", $t("settings.bridge.rotated") as string);
    } catch {
      showToast("error", $t("common.error") as string);
    } finally {
      rotating = false;
    }
  }

  let pairing = $state(false);
  let pairSecondsLeft = $state(0);
  let pairResult = $state<"none" | "success" | "expired">("none");
  let pairTimer: ReturnType<typeof setInterval> | null = null;
  let unlistenPaired: (() => void) | undefined;

  function stopPairTimer() {
    if (pairTimer) clearInterval(pairTimer);
    pairTimer = null;
  }

  async function startPairing() {
    try {
      const res = await invoke<{ ok: boolean; window_secs: number; enabled: boolean }>(
        "bridge_open_pairing"
      );
      if (!res.ok || !res.enabled) {
        showToast("error", $t("settings.bridge.pair_unavailable") as string);
        return;
      }
      pairing = true;
      pairResult = "none";
      pairSecondsLeft = res.window_secs;
      stopPairTimer();
      pairTimer = setInterval(() => {
        pairSecondsLeft -= 1;
        if (pairSecondsLeft <= 0) {
          pairing = false;
          pairResult = "expired";
          stopPairTimer();
        }
      }, 1000);
    } catch {
      showToast("error", $t("settings.bridge.pair_unavailable") as string);
    }
  }

  onMount(() => {
    listen("bridge-paired", () => {
      if (!pairing) return;
      pairing = false;
      pairResult = "success";
      stopPairTimer();
      showToast("success", $t("settings.bridge.pair_success") as string);
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenPaired = fn;
    }).catch(() => {
      showToast("error", $t("settings.bridge.pair_listener_failed"));
    });
  });

  onDestroy(() => {
    destroyed = true;
    stopPairTimer();
    unlistenPaired?.();
  });

  function maskedToken(value: string): string {
    if (!value) return "";
    if (value.length <= 8) return "•".repeat(value.length);
    return value.slice(0, 4) + "…" + value.slice(-4);
  }

  let turningOn = $state(false);
  let turnedOn = $state(false);

  async function turnOn() {
    if (turningOn) return;
    turningOn = true;
    if (await saveSettings({ bridge: { enabled: true } })) {
      turnedOn = true;
      if (info) info = { ...info, enabled: true };
    }
    turningOn = false;
  }

  let restartPending = $derived(
    turnedOn || getPendingEffects().some((effect) => effect.id === "bridge.lifecycle" && effect.state === "restart_required")
  );
  let ready = $derived(!!info && info.enabled && !restartPending && info.port !== 0 && !!info.token && !loadError);

  onMount(refresh);
</script>

<SettingGroup id="extension" icon="plug" title={$t("settings.bridge.title")} description={$t("settings.integrations.extension.desc")}>
  {#if loading && !info}
    <p class="bridge-state" role="status" aria-busy="true">{$t("settings.rebrand.state.loading")}</p>
  {:else if loadError}
    <SettingsEmptyState error title={$t("settings.bridge.load_failed")} onRetry={() => void refresh()} />
  {:else if info && info.enabled && restartPending}
    <SettingsEmptyState icon="plug" title={$t("settings.integrations.extension.restart_title")} hint={$t("settings.integrations.extension.restart_hint")} />
  {:else if info && !info.enabled}
    {#snippet turnOnAction()}
      <button type="button" class="btn btn-primary btn-sm" disabled={turningOn} aria-busy={turningOn} onclick={turnOn}>{$t("settings.integrations.extension.turn_on")}</button>
    {/snippet}
    <SettingsEmptyState icon="plug" title={$t("settings.integrations.extension.off_title")} hint={$t("settings.integrations.extension.off_hint")} action={turnOnAction} />
  {:else if !ready}
    <SettingsEmptyState error title={$t("settings.integrations.extension.failed_title")} hint={$t("settings.integrations.extension.failed_hint")} onRetry={() => void refresh()} />
  {:else}
    <SettingRow id="bridge-pair" label={$t("settings.bridge.pair_label")} help={$t("settings.bridge.pair_hint")} settingKey="settings.bridge.pair_label">
      {#snippet control(c)}
        <button
          class="btn btn-primary btn-sm"
          type="button"
          id={c.id}
          aria-describedby={[c.describedBy, "bridge-pair-state"].filter(Boolean).join(" ")}
          disabled={pairing || rotating}
          onclick={startPairing}
        >
          {pairing ? $t("settings.bridge.pair_waiting") : $t("settings.bridge.pair")}
        </button>
      {/snippet}
    </SettingRow>
    <p class="bridge-state" id="bridge-pair-state" role="status">
      {#if pairing}
        {$t("settings.bridge.pair_active", { secs: pairSecondsLeft })}
      {:else if pairResult === "success"}
        <span class="pair-success"><SettingsIcon name="check" size={18} />{$t("settings.bridge.pair_success")}</span>
      {:else if pairResult === "expired"}
        <span class="pair-expired"><SettingsIcon name="alert" size={18} />{$t("settings.bridge.pair_expired")}</span>
      {/if}
    </p>
  {/if}

  <div class="ext-install" data-setting-key="settings.integrations.extension.install">
    <h3 class="ext-subhead">{$t("settings.integrations.extension.install")}</h3>
    <SettingsDependencies />
  </div>

  {#if ready && info}
    {@const bridge = info}
    <IntegrationsDisclosure id="extension-advanced">
      <SettingRow id="bridge-endpoint" label={$t("settings.bridge.endpoint_label")} settingKey="settings.bridge.endpoint_label">
        {#snippet control(c)}
          <div class="bridge-value">
            <output id={c.id} class="bridge-mono" dir="ltr">{bridge.url}</output>
            <button class="btn btn-secondary btn-sm" type="button" aria-describedby={c.id} onclick={() => copy(bridge.url, "url")}>
              {$t("settings.bridge.copy")}
            </button>
          </div>
        {/snippet}
      </SettingRow>
      <SettingRow id="bridge-token" label={$t("settings.bridge.token_label")} settingKey="settings.bridge.token_label">
        {#snippet control(c)}
          <div class="bridge-value">
            <output id={c.id} class="bridge-mono token-display" dir="ltr">{revealed ? bridge.token : maskedToken(bridge.token)}</output>
            <div class="bridge-actions">
              <button class="btn btn-secondary btn-sm" type="button" aria-pressed={revealed} aria-describedby={`${c.id}-label`} onclick={() => (revealed = !revealed)}>
                {revealed ? $t("settings.bridge.hide") : $t("settings.bridge.reveal")}
              </button>
              <button class="btn btn-secondary btn-sm" type="button" aria-describedby={`${c.id}-label`} onclick={() => copy(bridge.token, "token")}>
                {$t("settings.bridge.copy")}
              </button>
            </div>
          </div>
        {/snippet}
      </SettingRow>
      <SettingRow id="bridge-rotate" label={$t("settings.bridge.rotate_label")} help={$t("settings.bridge.rotate_hint")} settingKey="settings.bridge.rotate_label">
        {#snippet control(c)}
          <button
            class="btn btn-secondary btn-sm danger"
            type="button"
            id={c.id}
            aria-describedby={c.describedBy}
            aria-expanded={confirmRotation}
            aria-controls="bridge-rotation-confirm-group"
            disabled={rotating || pairing}
            onclick={() => { confirmRotation = true; }}
          >
            {rotating ? $t("settings.bridge.rotating") : $t("settings.bridge.rotate")}
          </button>
        {/snippet}
      </SettingRow>
      {#if confirmRotation}
        <div class="rotation-confirm" id="bridge-rotation-confirm-group" role="group" aria-labelledby="bridge-rotation-confirm">
          <p id="bridge-rotation-confirm">{$t("settings.bridge.rotate_confirm")}</p>
          <div class="bridge-actions">
            <button class="btn btn-secondary btn-sm" type="button" disabled={rotating} onclick={() => { confirmRotation = false; }}>{$t("common.cancel")}</button>
            <button class="btn btn-secondary btn-sm danger" type="button" disabled={rotating || pairing} onclick={rotate}>
              {rotating ? $t("settings.bridge.rotating") : $t("settings.bridge.rotate_confirm_action")}
            </button>
          </div>
        </div>
      {/if}
    </IntegrationsDisclosure>
  {/if}
</SettingGroup>

<style>
  .rotation-confirm { margin-block-start: var(--space-2); padding: var(--space-3); border: 1px solid var(--content-border); border-radius: var(--radius-md); }
  .rotation-confirm p { margin: 0 0 var(--space-2); font-size: 13px; line-height: 1.5; }
  .bridge-state { margin: 0; font-size: 12px; line-height: 18px; color: var(--text-muted); }
  .bridge-state:empty { display: none; }
  .pair-success, .pair-expired { display: inline-flex; align-items: center; gap: var(--space-1); }
  .pair-success { color: var(--success); }
  .pair-expired { color: var(--warning); }
  .ext-install { margin-block-start: var(--space-3); padding-block-start: var(--space-3); border-block-start: 1px solid var(--separator); }
  .ext-subhead { margin: 0; font-size: 13px; line-height: 20px; font-weight: 600; color: var(--text); }
  .bridge-value { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: var(--space-2); min-inline-size: 0; inline-size: 100%; }
  .bridge-mono { min-inline-size: 0; overflow-wrap: anywhere; font-size: 12px; font-family: var(--font-mono, ui-monospace, monospace); color: var(--text-muted); }
  .token-display { word-break: break-all; }
  .bridge-actions { display: inline-flex; flex-wrap: wrap; gap: var(--space-2); }
  .danger { color: var(--danger); }
</style>
