<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Preset = "all" | "messages" | "read";
  type Tunnel = { state: "off" | "downloading" | "starting" | "on" | "error"; url: string | null; error: string | null };
  type Created = { principal_id: string; name: string; token: string; url: string; remote: boolean; preset: Preset; expires_at: number | null; prompt: string };
  type Listed = { principal_id: string; name: string; remote: boolean; preset: Preset | null; expires_at: number | null; expired: boolean };

  let { enabled = true }: { enabled?: boolean } = $props();

  let name = $state("");
  let remote = $state(false);
  let preset = $state<Preset>("all");
  let touchedPreset = $state(false);
  let days = $state(7);
  let busy = $state(false);
  let created = $state<Created | null>(null);
  let tunnel = $state<Tunnel>({ state: "off", url: null, error: null });
  let accesses = $state<Listed[]>([]);

  let tunnelBusy = $derived(tunnel.state === "downloading" || tunnel.state === "starting");

  $effect(() => {
    if (!touchedPreset) preset = remote ? "messages" : "all";
  });

  async function refresh() {
    try {
      [tunnel, accesses] = await Promise.all([invoke<Tunnel>("connect_tunnel_status"), invoke<Listed[]>("connect_ai_list")]);
    } catch {
      accesses = [];
    }
  }

  onMount(() => {
    void refresh();
    const stops = [
      listen<Tunnel>("connect-tunnel-changed", (e) => (tunnel = e.payload)),
      listen("connect-changed", () => void refresh()),
    ];
    return () => stops.forEach((s) => void s.then((u) => u()));
  });

  function errorText(raw: unknown): string {
    const text = String(raw);
    const key = text.split(":")[0];
    const known = ["tunnel_download_failed", "tunnel_download_corrupt", "tunnel_timeout", "tunnel_exited", "tunnel_start_failed", "BRIDGE_OFF"];
    return known.includes(key) ? ($t(`connect.any.errors.${key}`) as string) : text;
  }

  async function generate() {
    busy = true;
    created = null;
    try {
      created = await invoke<Created>("connect_ai_create", { name: name.trim(), preset, remote, days: remote ? days : null });
      await navigator.clipboard.writeText(created.prompt);
      showToast("success", $t("connect.any.copied") as string);
      await refresh();
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      busy = false;
    }
  }

  async function copyPrompt() {
    if (!created) return;
    await navigator.clipboard.writeText(created.prompt);
    showToast("success", $t("connect.any.copied") as string);
  }

  async function copyUrl() {
    if (!created) return;
    await navigator.clipboard.writeText(created.url);
    showToast("success", $t("agents.mcp.copied") as string);
  }

  async function stopTunnel() {
    try {
      tunnel = await invoke<Tunnel>("connect_tunnel_stop");
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function startTunnel() {
    try {
      tunnel = await invoke<Tunnel>("connect_tunnel_start");
    } catch (e) {
      showToast("error", errorText(e));
    }
  }

  async function revoke(id: string) {
    try {
      await invoke("tool_mcp_client_revoke", { id });
      if (created?.principal_id === id) created = null;
      await refresh();
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function date(ts: number | null): string {
    return ts ? new Date(ts * 1000).toLocaleDateString([], { day: "numeric", month: "short" }) : "";
  }
</script>

<section class="any" aria-labelledby="connect-any-title">
  <div class="head">
    <strong id="connect-any-title">{$t("connect.any.title")}</strong>
    <small>{$t("connect.any.intro")}</small>
  </div>

  <div class="form">
    <label class="field">
      <span>{$t("connect.any.name")}</span>
      <input class="input" type="text" maxlength="60" bind:value={name} placeholder={$t("connect.any.name_placeholder") as string} />
    </label>

    <fieldset class="field">
      <legend>{$t("connect.any.where")}</legend>
      <div class="choices">
        <label class="choice" class:on={!remote}><input type="radio" name="connect-any-where" checked={!remote} onchange={() => (remote = false)} /><span><strong>{$t("connect.any.where_local")}</strong><small>{$t("connect.any.where_local_hint")}</small></span></label>
        <label class="choice" class:on={remote}><input type="radio" name="connect-any-where" checked={remote} onchange={() => (remote = true)} /><span><strong>{$t("connect.any.where_cloud")}</strong><small>{$t("connect.any.where_cloud_hint")}</small></span></label>
      </div>
    </fieldset>

    <div class="row">
      <label class="field">
        <span>{$t("connect.any.preset")}</span>
        <select class="select" bind:value={preset} onchange={() => (touchedPreset = true)}>
          <option value="all">{$t("connect.any.preset_all")}</option>
          <option value="messages">{$t("connect.any.preset_messages")}</option>
          <option value="read">{$t("connect.any.preset_read")}</option>
        </select>
      </label>
      {#if remote}
        <label class="field">
          <span>{$t("connect.any.expires")}</span>
          <select class="select" bind:value={days}>
            <option value={1}>{$t("connect.any.days_1")}</option>
            <option value={7}>{$t("connect.any.days_7")}</option>
            <option value={30}>{$t("connect.any.days_30")}</option>
          </select>
        </label>
      {/if}
    </div>

    {#if remote}
      <p class="note" role="note">{$t("connect.any.cloud_warning")}</p>
    {/if}

    <button type="button" class="btn btn-primary" disabled={busy || !enabled || tunnelBusy} onclick={generate}>
      {#if busy}{remote && tunnel.state !== "on" ? $t("connect.any.opening") : $t("connect.connecting")}{:else}{$t("connect.any.generate")}{/if}
    </button>
  </div>

  {#if created}
    <div class="result" role="status">
      <strong>{$t("connect.any.ready", { name: created.name })}</strong>
      <small>{$t("connect.any.ready_hint")}</small>
      <textarea class="input prompt" readonly rows="6" value={created.prompt} aria-label={$t("connect.any.prompt_label") as string}></textarea>
      <div class="actions">
        <button type="button" class="btn btn-secondary btn-sm" onclick={copyPrompt}>{$t("connect.any.copy_prompt")}</button>
        <button type="button" class="btn btn-secondary btn-sm" onclick={copyUrl}>{$t("connect.any.copy_url")}</button>
      </div>
    </div>
  {/if}

  {#if tunnel.state !== "off"}
    <div class="tunnel" class:err={tunnel.state === "error"}>
      <span>
        <strong>{$t(`connect.any.tunnel_${tunnel.state}`)}</strong>
        {#if tunnel.url}<small class="mono">{tunnel.url}</small>{/if}
        {#if tunnel.state === "on"}<small>{$t("connect.any.tunnel_on_hint")}</small>{/if}
        {#if tunnel.error}<small>{errorText(tunnel.error)}</small>{/if}
      </span>
      {#if tunnel.state === "on" || tunnel.state === "error"}
        <button type="button" class="btn btn-secondary btn-sm" onclick={tunnel.state === "on" ? stopTunnel : startTunnel}>{tunnel.state === "on" ? $t("connect.any.tunnel_stop") : $t("connect.any.tunnel_retry")}</button>
      {/if}
    </div>
  {/if}

  {#if accesses.length}
    <ul class="list" aria-label={$t("connect.any.list") as string}>
      {#each accesses as a (a.principal_id)}
        <li>
          <span class="who"><bdi>{a.name}</bdi></span>
          <small>{a.remote ? $t("connect.any.where_cloud") : $t("connect.any.where_local")}{#if a.preset} · {$t(`connect.any.preset_${a.preset}`)}{/if}{#if a.expires_at} · {a.expired ? $t("connect.any.expired") : $t("connect.any.until", { date: date(a.expires_at) })}{/if}</small>
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => revoke(a.principal_id)}>{$t("connect.any.revoke")}</button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .any { display: flex; flex-direction: column; gap: var(--space-3); padding-block-end: var(--space-3); margin-block-end: var(--space-3); border-block-end: 1px solid var(--separator); }
  .head, .result { display: flex; flex-direction: column; gap: 2px; }
  strong { font-size: 13px; line-height: 18px; font-weight: 600; color: var(--text); }
  small { display: block; font-size: 12px; line-height: 17px; color: var(--text-muted); }
  .form { display: flex; flex-direction: column; gap: var(--space-2); align-items: flex-start; }
  .field { display: flex; flex-direction: column; gap: 4px; min-inline-size: 0; inline-size: 100%; max-inline-size: 420px; margin: 0; padding: 0; border: 0; font-size: 12px; color: var(--text-muted); }
  .row { display: flex; flex-wrap: wrap; gap: var(--space-2); inline-size: 100%; }
  .row .field { flex: 1 1 180px; }
  .choices { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 200px), 1fr)); gap: var(--space-2); }
  .choice { display: flex; gap: var(--space-2); align-items: flex-start; padding: var(--space-2); border: 1px solid var(--content-border); border-radius: var(--radius-md); background: var(--primary); cursor: pointer; }
  .choice.on { border-color: var(--accent); }
  .choice input { margin-block-start: 3px; }
  .note { margin: 0; max-inline-size: 560px; font-size: 12px; line-height: 17px; color: var(--warning); }
  .prompt { inline-size: 100%; margin-block-start: var(--space-2); font-family: var(--font-mono); font-size: 11px; resize: vertical; }
  .actions { display: flex; flex-wrap: wrap; gap: var(--space-1); margin-block-start: var(--space-1); }
  .tunnel { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--space-3); padding: var(--space-2); border: 1px solid var(--success); border-radius: var(--radius-md); }
  .tunnel.err { border-color: var(--warning); }
  .tunnel > span { min-inline-size: 0; }
  .mono { font-family: var(--font-mono); overflow-wrap: anywhere; color: var(--text); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .list li { display: flex; align-items: center; gap: var(--space-2); min-inline-size: 0; }
  .who { font-size: 13px; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-inline-size: 40%; }
  .list small { flex: 1; min-inline-size: 0; }
</style>
