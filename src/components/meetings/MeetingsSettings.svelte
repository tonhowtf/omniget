<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    getSettings,
    saveSettings,
    whisperInstall,
    whisperModelDownload,
    whisperModelRemove,
    whisperStatus,
    teamsConfigure,
    teamsSignInFinish,
    teamsSignInStart,
    teamsSignOut,
    teamsStatus,
    type DeviceCode,
    type MeetingSettings,
    type Progress,
    type TeamsStatus,
    type WhisperStatus,
  } from "$lib/meetings/api";

  let form = $state<MeetingSettings | null>(null);
  let vexaKeySet = $state(false);
  let webhookSet = $state(false);
  let vexaKey = $state("");
  let webhook = $state("");
  let whisper = $state<WhisperStatus | null>(null);
  let saving = $state(false);
  let installing = $state(false);
  let downloads = $state<Record<string, number | null>>({});
  let unlisten: UnlistenFn | null = null;

  let ms = $state<TeamsStatus | null>(null);
  let clientId = $state("");
  let tenant = $state("organizations");
  let adminScopes = $state(false);
  let deviceCode = $state<DeviceCode | null>(null);
  let msBusy = $state(false);

  async function loadTeams() {
    ms = await teamsStatus();
    clientId = ms.client_id;
    tenant = ms.tenant || "organizations";
    adminScopes = ms.admin_scopes;
  }

  async function connect() {
    msBusy = true;
    try {
      ms = await teamsConfigure(clientId.trim(), tenant.trim(), adminScopes);
      deviceCode = await teamsSignInStart();
      ms = await teamsSignInFinish();
      showToast("success", $t("meetings.teams_connected", { name: ms.account?.name ?? "" }) as string);
    } catch (e) {
      showToast("error", String(e), 12000);
    } finally {
      deviceCode = null;
      msBusy = false;
    }
  }

  async function disconnect() {
    try {
      ms = await teamsSignOut();
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function copyCode() {
    if (!deviceCode) return;
    try {
      await navigator.clipboard.writeText(deviceCode.user_code);
    } catch {
    }
  }

  async function load() {
    const v = await getSettings();
    const { vexa_key_set, teams_webhook_set, ...rest } = v;
    form = rest;
    vexaKeySet = vexa_key_set;
    webhookSet = teams_webhook_set;
    whisper = await whisperStatus();
  }

  async function save() {
    if (!form) return;
    saving = true;
    try {
      const v = await saveSettings($state.snapshot(form), {
        vexaKey: vexaKey.trim() ? vexaKey.trim() : undefined,
        teamsWebhook: webhook.trim() ? webhook.trim() : undefined,
      });
      vexaKeySet = v.vexa_key_set;
      webhookSet = v.teams_webhook_set;
      vexaKey = "";
      webhook = "";
      showToast("success", $t("meetings.settings_saved") as string);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      saving = false;
    }
  }

  async function clearSecret(which: "vexa" | "webhook") {
    if (!form) return;
    try {
      const v = await saveSettings($state.snapshot(form), which === "vexa" ? { vexaKey: "" } : { teamsWebhook: "" });
      vexaKeySet = v.vexa_key_set;
      webhookSet = v.teams_webhook_set;
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function install() {
    installing = true;
    try {
      await whisperInstall();
      whisper = await whisperStatus();
    } catch (e) {
      showToast("error", String(e), 9000);
    } finally {
      installing = false;
    }
  }

  async function download(id: string) {
    downloads = { ...downloads, [id]: 0 };
    try {
      await whisperModelDownload(id);
      whisper = await whisperStatus();
      if (form) {
        form.whisper_model = id;
        await save();
      }
    } catch (e) {
      showToast("error", String(e));
    } finally {
      const { [id]: _, ...rest } = downloads;
      downloads = rest;
    }
  }

  async function remove(id: string) {
    try {
      await whisperModelRemove(id);
      whisper = await whisperStatus();
    } catch (e) {
      showToast("error", String(e));
    }
  }

  onMount(async () => {
    await load();
    await loadTeams();
    unlisten = await listen<Progress>("meetings://progress", (e) => {
      const p = e.payload;
      if (p.id.startsWith("whisper-model:")) {
        const id = p.id.slice("whisper-model:".length);
        if (id in downloads) downloads = { ...downloads, [id]: p.total ? Math.round((p.done / p.total) * 100) : null };
      }
    });
  });
  onDestroy(() => unlisten?.());
</script>

{#if form}
  <div class="settings">
    <section>
      <h3>{$t("meetings.set_transcription")}</h3>
      <label class="field">
        <span>{$t("meetings.set_engine")}</span>
        <select class="input" bind:value={form.transcription}>
          <option value="auto">{$t("meetings.engine_auto")}</option>
          <option value="local">{$t("meetings.engine_local")}</option>
          <option value="api">{$t("meetings.engine_api")}</option>
        </select>
      </label>
      {#if whisper}
        <p class="hint">
          {#if whisper.installed}
            {$t("meetings.whisper_installed")}
          {:else if whisper.can_install}
            {$t("meetings.whisper_missing")}
            <button class="btn btn-sm btn-secondary" onclick={install} disabled={installing}>{installing ? "…" : $t("meetings.whisper_install")}</button>
          {:else}
            {$t("meetings.whisper_install_hint")} <code>{whisper.install_hint}</code>
          {/if}
          {#if whisper.api_available}· {$t("meetings.api_available")}{/if}
        </p>
        <ul class="models">
          {#each whisper.models as mdl (mdl.id)}
            <li>
              <label class="check">
                <input type="radio" name="whisper-model" value={mdl.id} bind:group={form.whisper_model} disabled={!mdl.installed} />
                <span>{mdl.id}</span>
                <span class="hint">{mdl.size_mb} MB · {mdl.note}</span>
              </label>
              {#if mdl.id in downloads}
                <span class="hint">{downloads[mdl.id] === null ? "…" : `${downloads[mdl.id]}%`}</span>
              {:else if mdl.installed}
                <button class="btn btn-sm btn-ghost" onclick={() => remove(mdl.id)}>{$t("meetings.model_remove")}</button>
              {:else}
                <button class="btn btn-sm btn-secondary" onclick={() => download(mdl.id)}>{$t("meetings.model_download")}</button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
      <label class="field">
        <span>{$t("meetings.set_language")}</span>
        <input class="input" bind:value={form.language} placeholder="auto" />
      </label>
    </section>

    <section>
      <h3>{$t("meetings.set_summary")}</h3>
      <label class="check">
        <input type="checkbox" bind:checked={form.auto_summarize} />
        <span>{$t("meetings.set_auto_summarize")}</span>
      </label>
      <label class="field">
        <span>{$t("meetings.set_summary_language")}</span>
        <input class="input" bind:value={form.summary_language} placeholder={$t("meetings.same_as_meeting") as string} />
      </label>
      <p class="hint">{$t("meetings.summary_provider_hint")} <a href="/settings?tab=ai">{$t("meetings.open_ai_settings")}</a></p>
    </section>

    <section>
      <h3>{$t("meetings.set_teams")}</h3>
      <label class="field">
        <span>{$t("meetings.set_webhook")}</span>
        <input class="input" type="password" bind:value={webhook} placeholder={webhookSet ? ($t("meetings.secret_saved") as string) : "https://…"} autocomplete="off" />
      </label>
      {#if webhookSet}<button class="btn btn-sm btn-ghost" onclick={() => clearSecret("webhook")}>{$t("meetings.secret_clear")}</button>{/if}
      <p class="hint">{$t("meetings.webhook_hint")}</p>
      <p class="hint">{$t("meetings.m365_hint")} <a href="/llm/mcp">{$t("meetings.open_mcp")}</a></p>
    </section>

    <section>
      <h3>{$t("meetings.set_m365")}</h3>
      {#if ms?.signed_in}
        <p class="hint">{$t("meetings.teams_signed_in_as", { name: ms.account?.name ?? "", upn: ms.account?.upn ?? "" })}</p>
        <button class="btn btn-sm btn-ghost" onclick={disconnect}>{$t("meetings.teams_disconnect")}</button>
      {:else}
        <label class="field">
          <span>{$t("meetings.set_client_id")}</span>
          <input class="input" bind:value={clientId} placeholder="00000000-0000-0000-0000-000000000000" autocomplete="off" />
        </label>
        <label class="field">
          <span>{$t("meetings.set_tenant")}</span>
          <input class="input" bind:value={tenant} placeholder="organizations" />
        </label>
        <label class="check">
          <input type="checkbox" bind:checked={adminScopes} />
          <span>{$t("meetings.set_admin_scopes")}</span>
        </label>
        {#if deviceCode}
          <div class="code" role="status">
            <p>{$t("meetings.teams_device_step", { url: deviceCode.verification_uri })}</p>
            <button class="btn btn-secondary code-btn" onclick={copyCode} title={$t("meetings.teams_copy_code") as string}>{deviceCode.user_code}</button>
            <a class="btn btn-sm btn-primary" href={deviceCode.verification_uri} target="_blank" rel="noreferrer">{$t("meetings.teams_open_login")}</a>
          </div>
        {:else}
          <button class="btn btn-sm btn-primary" onclick={connect} disabled={msBusy || !clientId.trim()}>{$t("meetings.teams_connect")}</button>
        {/if}
        <p class="hint">{$t("meetings.m365_setup_hint")}</p>
      {/if}
    </section>

    <section>
      <h3>{$t("meetings.set_bot")}</h3>
      <label class="field">
        <span>{$t("meetings.set_vexa_url")}</span>
        <input class="input" bind:value={form.vexa_url} placeholder="https://api.cloud.vexa.ai" />
      </label>
      <label class="field">
        <span>{$t("meetings.set_vexa_key")}</span>
        <input class="input" type="password" bind:value={vexaKey} placeholder={vexaKeySet ? ($t("meetings.secret_saved") as string) : ""} autocomplete="off" />
      </label>
      {#if vexaKeySet}<button class="btn btn-sm btn-ghost" onclick={() => clearSecret("vexa")}>{$t("meetings.secret_clear")}</button>{/if}
      <label class="field">
        <span>{$t("meetings.set_bot_name")}</span>
        <input class="input" bind:value={form.bot_name} />
      </label>
      <p class="hint">{$t("meetings.bot_hint")}</p>
    </section>

    <div class="save">
      <button class="btn btn-primary" onclick={save} disabled={saving}>{$t("meetings.save")}</button>
    </div>
  </div>
{/if}

<style>
  .settings {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: var(--space-4);
  }
  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  h3 {
    margin: 0;
    font-size: var(--text-sm);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-sm);
  }
  .check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    flex-wrap: wrap;
  }
  .models {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .models li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .code {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .code p {
    margin: 0;
  }
  .code-btn {
    font-family: var(--font-mono);
    font-size: var(--text-lg);
    letter-spacing: 0.1em;
  }
  .save {
    grid-column: 1 / -1;
    display: flex;
    justify-content: flex-end;
  }
</style>
