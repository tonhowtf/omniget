<script lang="ts">
  /** Trakt: device flow com as credenciais do próprio usuário, e o export. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openUrl, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Creds = { client_id_hint: string; has_client_id: boolean; has_client_secret: boolean; connected: boolean; username: string; expires_at: number; app_url: string };
  type DeviceCode = { device_code: string; user_code: string; verification_url: string; expires_in: number; interval: number };
  type Entry = { date: string; kind: string; title: string; year: number | null; rating: number | null; list: string; url: string };
  type PartCount = { part: string; entries: number };
  type Result = { used_session: boolean; username: string; entries: number; by_part: PartCount[]; requests: number; files: string[]; dest: string; sample: Entry[] };

  const PARTS = ["history", "ratings", "watchlist", "lists"];

  let creds = $state<Creds | null>(null);
  let clientId = $state("");
  let clientSecret = $state("");
  let device = $state<DeviceCode | null>(null);
  let connecting = $state(false);

  let dest = $state("");
  let parts = $state<string[]>([...PARTS]);
  let formats = $state<string[]>(["json", "csv", "md"]);
  let maxPages = $state(50);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const canRun = $derived(!!dest && !!creds?.connected && parts.length > 0 && formats.length > 0);

  function toggle(list: string[], v: string): string[] {
    return list.includes(v) ? list.filter((x) => x !== v) : [...list, v];
  }

  async function refresh() {
    try {
      creds = await invoke<Creds>("tool_trakt_creds");
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "trakt-export") progress = p;
    });
    await refresh();
  });
  onDestroy(() => unlisten?.());

  async function saveApp() {
    try {
      creds = await invoke<Creds>("tool_trakt_save_app", { clientId: clientId.trim(), clientSecret: clientSecret.trim() });
      clientId = "";
      clientSecret = "";
      showToast("success", $t("tools.trakt.saved"));
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function connect() {
    if (connecting) return;
    connecting = true;
    device = null;
    try {
      device = await invoke<DeviceCode>("tool_trakt_device_code");
      await openUrl(device.verification_url);
      const r = await invoke<{ connected: boolean; username: string; message: string }>("tool_trakt_connect", { code: device });
      showToast("success", r.message);
      await refresh();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      connecting = false;
      device = null;
    }
  }

  async function disconnect(forget: boolean) {
    try {
      creds = await invoke<Creds>("tool_trakt_disconnect", { forget });
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function run() {
    if (!canRun || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_trakt_export", { opts: { dest, parts, formats, max_pages: maxPages, delay_ms: 800 } });
      showToast("success", `${result.entries} ${$t("tools.lists.entries")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.trakt.app")}</div>
          <div class="group-row-sub">{$t("tools.trakt.app_note")}</div>
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(creds?.app_url ?? "https://app.trakt.tv/settings/apps/api/new")}>{$t("tools.trakt.create_app")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">client_id</div><div class="group-row-sub mono">{creds?.has_client_id ? creds.client_id_hint : $t("tools.trakt.missing")}</div></div>
        <div class="group-row-trailing"><input class="input" type="text" bind:value={clientId} placeholder="client_id" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">client_secret</div><div class="group-row-sub">{creds?.has_client_secret ? $t("tools.trakt.stored") : $t("tools.trakt.missing")}</div></div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="password" bind:value={clientSecret} placeholder="client_secret" />
          <button class="btn btn-secondary btn-sm" type="button" disabled={!clientId.trim() && !clientSecret.trim()} onclick={saveApp}>{$t("tools.common.save")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.lists.session")}</div>
          <div class="group-row-sub">
            {#if device}
              {$t("tools.trakt.type_code")} <strong class="code">{device.user_code}</strong> — {device.verification_url}
            {:else if creds?.connected}
              {$t("tools.trakt.connected_as")} {creds.username || "—"}
            {:else}
              {$t("tools.trakt.not_connected")}
            {/if}
          </div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if creds?.connected}
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => disconnect(false)}>{$t("tools.trakt.disconnect")}</button>
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => disconnect(true)}>{$t("tools.trakt.forget")}</button>
          {:else}
            <button class="btn btn-secondary btn-sm" type="button" disabled={connecting || !creds?.has_client_secret} onclick={connect}>{connecting ? $t("tools.trakt.waiting") : $t("tools.trakt.connect")}</button>
          {/if}
        </div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.lists.dest")}</div><div class="group-row-sub mono">{dest || "—"}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.trakt.parts")}</div></div>
        <div class="group-row-trailing btn-row wrap">
          {#each PARTS as p (p)}
            <button class="btn btn-sm" class:btn-primary={parts.includes(p)} class:btn-ghost={!parts.includes(p)} type="button" onclick={() => (parts = toggle(parts, p))}>{$t(`tools.trakt.part_${p}`)}</button>
          {/each}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.lists.formats")}</div></div>
        <div class="group-row-trailing btn-row">
          {#each ["json", "csv", "md"] as f (f)}
            <button class="btn btn-sm" class:btn-primary={formats.includes(f)} class:btn-ghost={!formats.includes(f)} type="button" onclick={() => (formats = toggle(formats, f))}>{f.toUpperCase()}</button>
          {/each}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.trakt.max_pages")}</div><div class="group-row-sub">{$t("tools.trakt.max_pages_note")}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="1" max="500" step="5" bind:value={maxPages} style:width="6em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !canRun} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.trakt.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.entries} {$t("tools.lists.entries")}</div>
            <div class="group-row-sub">{result.username} · {result.requests} {$t("tools.lists.requests")}</div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result?.dest ?? "")}>↗</button></div>
        </div>
        {#each result.by_part as p (p.part)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub">{p.part}</div></div>
            <div class="group-row-trailing">{p.entries}</div>
          </div>
        {/each}
      </div>
    </section>
    <section>
      <div class="group">
        {#each result.sample as e (e.title + e.date + e.list)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{e.title}{#if e.year} <span class="dim">({e.year})</span>{/if}</div>
              <div class="group-row-sub">{e.date || "—"} · {e.list}{#if e.rating} · {e.rating.toFixed(1)}/10{/if}</div>
            </div>
            {#if e.url}<div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(e.url)}>↗</button></div>{/if}
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .code { font-family: var(--font-mono); letter-spacing: 0.1em; }
  .wrap { flex-wrap: wrap; justify-content: flex-end; }
</style>
