<script lang="ts">
  /**
   * Pair a device: one-time link (10 min) with the secret after the `#`,
   * shown as a QR code and as text. The scope is kept on the server side.
   */
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, type PairLink, type RemoteScope, type RemoteStatus } from "./types";

  let { status, extraBases = [] }: { status: RemoteStatus; extraBases?: { kind: string; url: string }[] } = $props();

  let scope = $state<RemoteScope>("read");
  let base = $state("");
  let label = $state("");
  let link = $state<PairLink | null>(null);
  let busy = $state(false);
  let now = $state(Date.now());
  let clock: ReturnType<typeof setInterval> | null = null;

  const bases = $derived([...status.endpoints, ...extraBases]);
  const left = $derived(link ? Math.max(0, Math.round((new Date(link.expiresAt).getTime() - now) / 1000)) : 0);

  $effect(() => {
    if (!base || !bases.some((b) => b.url === base)) base = bases[0]?.url ?? "";
  });

  function tick() {
    now = Date.now();
    if (link && left <= 0) {
      link = null;
      stopClock();
    }
  }
  function stopClock() {
    if (clock) clearInterval(clock);
    clock = null;
  }
  onDestroy(stopClock);

  async function create() {
    busy = true;
    try {
      link = await invoke<PairLink>("remote_pair_create", { scope, base: base || null, label: label.trim() || null });
      now = Date.now();
      stopClock();
      clock = setInterval(tick, 1000);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function cancel() {
    try {
      await invoke("remote_pair_cancel");
    } catch {
      // Nothing pending.
    }
    link = null;
    stopClock();
  }

  async function copy() {
    if (!link) return;
    await navigator.clipboard.writeText(link.url);
    showToast("success", $t("llm.central.remote.copied") as string);
  }

  function baseLabel(kind: string): string {
    return $t(`llm.central.remote.base.${kind}`) as string;
  }
</script>

<div class="group">
  <div class="group-label">{$t("llm.central.remote.pair_title")}</div>
  {#if !status.running}
    <div class="group-row">
      <div class="group-row-content">
        <div class="group-row-sub">{$t("llm.central.remote.pair_needs_on")}</div>
      </div>
    </div>
  {:else}
    <div class="group-row">
      <div class="group-row-content">
        <div class="group-row-title">{$t("llm.central.remote.scope")}</div>
        <div class="group-row-sub">
          {scope === "drive" ? $t("llm.central.remote.scope_drive_hint") : $t("llm.central.remote.scope_read_hint")}
        </div>
      </div>
      <div class="group-row-trailing seg">
        <button type="button" class="button" class:primary={scope === "read"} onclick={() => (scope = "read")}>
          {$t("llm.central.remote.scope_read")}
        </button>
        <button type="button" class="button" class:primary={scope === "drive"} onclick={() => (scope = "drive")}>
          {$t("llm.central.remote.scope_drive")}
        </button>
      </div>
    </div>
    <div class="group-row">
      <div class="group-row-content">
        <div class="group-row-title">{$t("llm.central.remote.address")}</div>
        <div class="group-row-sub">{$t("llm.central.remote.address_hint")}</div>
      </div>
      <div class="group-row-trailing">
        <select bind:value={base}>
          {#each bases as b (b.url)}
            <option value={b.url}>{baseLabel(b.kind)} — {b.url}</option>
          {/each}
        </select>
      </div>
    </div>
    <div class="group-row">
      <div class="group-row-content">
        <div class="group-row-title">{$t("llm.central.remote.device_name")}</div>
      </div>
      <div class="group-row-trailing">
        <input type="text" maxlength="60" placeholder={$t("llm.central.remote.device_name_ph")} bind:value={label} />
        <button type="button" class="button primary" disabled={busy || !base} onclick={create}>
          {$t("llm.central.remote.generate")}
        </button>
      </div>
    </div>
    {#if link}
      <div class="group-row qr-row">
        <div class="qr" aria-label={$t("llm.central.remote.qr_alt")}>
          {@html link.qrSvg}
        </div>
        <div class="group-row-content">
          <div class="group-row-title">{$t("llm.central.remote.scan")}</div>
          <code class="url">{link.url}</code>
          <div class="group-row-sub">
            {$t("llm.central.remote.expires_in", { s: left })} · {link.scope === "drive"
              ? $t("llm.central.remote.scope_drive")
              : $t("llm.central.remote.scope_read")}
          </div>
          <div class="actions">
            <button type="button" class="button" onclick={copy}>{$t("llm.central.remote.copy_link")}</button>
            <button type="button" class="button" onclick={cancel}>{$t("llm.central.remote.cancel")}</button>
          </div>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .seg {
    display: flex;
    gap: 6px;
  }
  .group-row-trailing input[type="text"] {
    width: 180px;
    margin-right: 8px;
  }
  .qr-row {
    align-items: flex-start;
    gap: var(--space-4, 16px);
  }
  .qr {
    width: 220px;
    height: 220px;
    flex-shrink: 0;
    background: #fff;
    border-radius: 12px;
    padding: 6px;
    box-sizing: border-box;
  }
  .qr :global(svg) {
    width: 100%;
    height: 100%;
  }
  .url {
    display: block;
    word-break: break-all;
    font-size: 12px;
    margin: 6px 0;
    user-select: all;
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 8px;
  }
</style>
