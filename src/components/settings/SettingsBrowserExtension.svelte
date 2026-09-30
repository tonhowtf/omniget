<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SettingGroup from "./rebrand/SettingGroup.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingsEmptyState from "./rebrand/SettingsEmptyState.svelte";

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
    } catch (e: any) {
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
    // The backend emits `bridge-paired` the moment the extension fetches the
    // token from GET /v1/pair, so we can flip the countdown into a success
    // state instead of letting it silently expire.
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

  onMount(refresh);
</script>

<SettingGroup id="extension" title={$t("settings.bridge.title")} description={$t("settings.bridge.intro")}>
  {#if loading && !info}
    <p class="bridge-state" role="status" aria-busy="true">{$t("settings.rebrand.state.loading")}</p>
  {:else if loadError}
    <SettingsEmptyState error title={$t("settings.bridge.load_failed")} onRetry={() => void refresh()} />
  {:else if !info || !info.enabled || info.port === 0 || !info.token}
    <SettingsEmptyState title={$t("settings.bridge.unavailable")} />
  {:else}
    <div data-setting-key="settings.browser_ext.title">
      <SettingRow id="bridge-pair" label={$t("settings.bridge.pair_label")} help={$t("settings.bridge.pair_hint")} settingKey="settings.bridge.pair_label">
        {#snippet control(c)}
          <button
            class="bridge-action primary"
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
          <span class="pair-success">{$t("settings.bridge.pair_success")}</span>
        {:else if pairResult === "expired"}
          <span class="pair-expired">{$t("settings.bridge.pair_expired")}</span>
        {/if}
      </p>
    </div>
  {/if}
</SettingGroup>

{#if info && info.enabled && info.port !== 0 && info.token && !loadError}
  <!-- Endpoint, token and rotation are technical details: collapsed by default. -->
  <SettingGroup id="extension-details" title={$t("settings.rebrand.category.advanced")} advanced>
    <SettingRow id="bridge-endpoint" label={$t("settings.bridge.endpoint_label")} settingKey="settings.bridge.endpoint_label">
      {#snippet control(c)}
        <div class="bridge-value">
          <output id={c.id} class="bridge-mono" dir="ltr">{info!.url}</output>
          <button class="bridge-action" type="button" aria-describedby={c.id} onclick={() => copy(info!.url, "url")}>
            {$t("settings.bridge.copy")}
          </button>
        </div>
      {/snippet}
    </SettingRow>
    <SettingRow id="bridge-token" label={$t("settings.bridge.token_label")} settingKey="settings.bridge.token_label">
      {#snippet control(c)}
        <div class="bridge-value">
          <output id={c.id} class="bridge-mono token-display" dir="ltr">{revealed ? info!.token : maskedToken(info!.token)}</output>
          <div class="bridge-actions">
            <button class="bridge-action" type="button" aria-pressed={revealed} onclick={() => (revealed = !revealed)}>
              {revealed ? $t("settings.bridge.hide") : $t("settings.bridge.reveal")}
            </button>
            <button class="bridge-action" type="button" onclick={() => copy(info!.token, "token")}>
              {$t("settings.bridge.copy")}
            </button>
          </div>
        </div>
      {/snippet}
    </SettingRow>
    <SettingRow id="bridge-rotate" label={$t("settings.bridge.rotate_label")} help={$t("settings.bridge.rotate_hint")} settingKey="settings.bridge.rotate_label">
      {#snippet control(c)}
        <button
          class="bridge-action danger"
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
          <button class="bridge-action" type="button" disabled={rotating} onclick={() => { confirmRotation = false; }}>{$t("common.cancel")}</button>
          <button class="bridge-action danger" type="button" disabled={rotating || pairing} onclick={rotate}>
            {rotating ? $t("settings.bridge.rotating") : $t("settings.bridge.rotate_confirm_action")}
          </button>
        </div>
      </div>
    {/if}
  </SettingGroup>
{/if}

<style>
  .rotation-confirm { margin-top: 12px; padding: 12px; border: 1px solid var(--border); border-radius: var(--border-radius); }
  .rotation-confirm p { margin: 0 0 8px; line-height: 1.5; }

  .bridge-state { margin: 0; font-size: 12px; line-height: 18px; color: var(--text-muted); }
  .bridge-state:empty { display: none; }
  .bridge-value { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: 8px; min-inline-size: 0; inline-size: 100%; }
  .bridge-mono {
    min-inline-size: 0;
    overflow-wrap: anywhere;
    font-size: 12px;
    font-family: var(--font-mono, ui-monospace, monospace);
    color: var(--text-muted);
  }
  .pair-success {
    color: var(--success);
  }
  .pair-expired {
    color: var(--warning);
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
    border: none;
    background: rgba(255, 255, 255, 0.04);
    color: inherit;
    padding: 6px 12px;
    border-radius: 8px;
    font: inherit;
    cursor: pointer;
  }
  .bridge-action:hover { background: rgba(255, 255, 255, 0.08); }
  .bridge-action:disabled { opacity: 0.5; cursor: not-allowed; }
  .bridge-action.primary {
    background: var(--accent);
    color: var(--on-accent);
    border-color: transparent;
  }
  .bridge-action.primary:hover { background: var(--accent-lo, var(--accent)); }
  .bridge-action.danger { color: var(--danger); border-color: color-mix(in srgb, var(--danger) 30%, transparent); }
  .bridge-action.danger:hover { background: rgba(255, 139, 111, 0.1); }
</style>
