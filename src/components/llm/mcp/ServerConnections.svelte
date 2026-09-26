<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { isAbsolutePath, shortPath } from "$lib/stores/assist-missions-store.svelte";
  type ExecGrant = { id: string; principal: string; principal_name: string; workspace: string; workspace_full: string; executors: { id: string; name: string }[]; max_tokens: number; media: boolean; revoked: boolean };
  type FileRoot = { id: string; principal: string; principal_name: string; path: string; path_full: string };
  type DerivedBot = { bot: string; name: string; grant_id: string; principal: string; principal_name: string; source_name: string; created_ms: number; retired: boolean; grant_active: boolean };
  let execGrants = $state<ExecGrant[]>([]);
  let derivedBots = $state<DerivedBot[]>([]);
  let fileRoots = $state<FileRoot[]>([]);
  let activeGrants = $derived(execGrants.filter((g) => !g.revoked));
  let folder = $state("");
  let workspace = $state("");
  let endpoints = $state("");
  let executors = $state<{ id: string; name: string }[]>([]);
  let selectedExecutors = $state<string[]>([]);
  let maxTokens = $state(100000);
  let media = $state(false);
  let mediaCli = $state("");
  let budgetClient = $state("");
  let budgetMission = $state("");
  let budgetCredits = $state("");
  let mediaBusy = $state(false);
  let mediaMessage = $state("");
  // Parse decimal text into exact integers; do not multiply a floating-point credit value.
  function toMicrocredits(value: string): number | null {
    const text = value.trim();
    if (!/^\d{1,7}(?:[.,]\d{1,6})?$/.test(text)) return null;
    const [whole, fraction = ""] = text.split(/[.,]/);
    const amount = Number(whole) * 1_000_000 + Number(fraction.padEnd(6, "0"));
    return Number.isSafeInteger(amount) && amount > 0 && amount <= 1_000_000_000_000 ? amount : null;
  }
  const microcredits = $derived(toMicrocredits(budgetCredits));
  const validMission = $derived(/^[A-Za-z0-9_-]{1,128}$/.test(budgetMission.trim()));
  async function chooseMediaCli() {
    try {
      const selected = await open({ directory: false, multiple: false });
      if (typeof selected === "string") mediaCli = selected;
    } catch (error) { mediaMessage = String(error); }
  }
  async function configureMedia() {
    if (!mediaCli.trim()) return;
    mediaBusy = true; mediaMessage = "";
    try {
      await invoke("tool_mcp_media_configure", { path: mediaCli.trim() });
      mediaMessage = $t("mcp_connections.media_configured");
    } catch (error) { mediaMessage = String(error); }
    finally { mediaBusy = false; }
  }
  async function authorizeMediaBudget() {
    const amount = toMicrocredits(budgetCredits);
    if (amount === null || !validMission || !clients.some(c => c.id === budgetClient)) {
      mediaMessage = $t("mcp_connections.media_budget_invalid"); return;
    }
    mediaBusy = true; mediaMessage = "";
    try {
      await invoke("tool_mcp_media_budget", { principal: budgetClient, mission: budgetMission.trim(), microcredits: amount });
      mediaMessage = $t("mcp_connections.media_budget_saved");
      budgetCredits = "";
    } catch (error) { mediaMessage = String(error); }
    finally { mediaBusy = false; }
  }
  let { url }: { url: string } = $props();
  type Client = { id: string; name: string; scopes: string[] };
  let clients = $state<Client[]>([]);
  let name = $state("");
  let scopes = $state(["discover", "enqueue", "control", "diagnostics", "artifacts", "history"]);
  let gatewayWrite = $state(false);
  let gatewayTransfer = $state(false);
  const gatewayCanWrite = $derived(scopes.some(scope => ["enqueue", "control", "orchestration_create", "orchestration_control"].includes(scope)));
  const gatewayCanTransfer = $derived(scopes.includes("transfer"));
  $effect(() => {
    if (!gatewayCanWrite) gatewayWrite = false;
    if (!gatewayCanTransfer) gatewayTransfer = false;
  });
  let grant = $state<{ principal: Client; token: string } | null>(null);
  let busy = $state(false);
  let message = $state("");
  let reveal = $state(false);
  let snippets = $state<[string,string][]>([]);
  let client = $state(0);
  const options = ["discover", "enqueue", "control", "diagnostics", "artifacts", "history", "local_network", "transfer", "orchestration_read", "orchestration_control", "orchestration_create", "auth"];
  async function chooseFolder() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected === "string") folder = selected;
  }
  async function chooseWorkspace() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected === "string") workspace = selected;
  }
  async function refresh() {
    try { [clients, executors] = await Promise.all([invoke<Client[]>("tool_mcp_clients"), invoke<{id:string;name:string}[]>("tool_mcp_executors")]); }
    catch (e) { message = String(e); }
    await refreshGrants();
  }
  async function refreshGrants() {
    try {
      const r = await invoke<{ grants: ExecGrant[]; roots: FileRoot[]; derived?: DerivedBot[] }>("tool_mcp_execution_grants_list");
      execGrants = r.grants ?? []; fileRoots = r.roots ?? []; derivedBots = r.derived ?? [];
    } catch { execGrants = []; fileRoots = []; derivedBots = []; }
  }
  async function revokeGrant(id: string) {
    busy = true;
    try { await invoke("tool_mcp_execution_grant_revoke", { grantId: id }); message = $t("mcp_connections.grant_revoked"); await refreshGrants(); }
    catch (e) { message = String(e); }
    finally { busy = false; }
  }
  /** Bots a client derived through MCP (`agents_prepare`), shown under its grant; revoking one keeps the grant. */
  function derivedOf(grantId: string): DerivedBot[] {
    return derivedBots.filter((d) => d.grant_id === grantId);
  }
  async function revokeDerived(bot: string) {
    busy = true;
    try { await invoke("assist_mcp_derived_bot_revoke", { bot }); message = $t("mcp_connections.derived_revoked"); await refreshGrants(); }
    catch (e) { message = String(e); }
    finally { busy = false; }
  }
  async function revokeRoot(id: string) {
    busy = true;
    try { await invoke("tool_mcp_root_revoke", { id }); message = $t("mcp_connections.root_revoked"); await refreshGrants(); }
    catch (e) { message = String(e); }
    finally { busy = false; }
  }
  onMount(refresh);
  async function create() {
    folder = folder.trim(); workspace = workspace.trim();
    if (scopes.includes("transfer") && !folder) { message = $t("mcp_connections.folder_required"); return; }
    if (scopes.includes("transfer") && !isAbsolutePath(folder)) { message = $t("mission.path.err_absolute"); return; }
    if (scopes.includes("orchestration_create") && workspace && !isAbsolutePath(workspace)) { message = $t("mission.path.err_absolute"); return; }
    if (scopes.includes("local_network") && !endpoints.trim()) { message = $t("mcp_connections.endpoints_required"); return; }
    if (scopes.includes("orchestration_create") && (!workspace || !selectedExecutors.length || !Number.isSafeInteger(maxTokens) || maxTokens < 1)) { message = $t("mcp_connections.execution_required"); return; }
    const remoteWrite = gatewayWrite && gatewayCanWrite;
    const remoteTransfer = gatewayTransfer && gatewayCanTransfer;
    busy = true; message = "";
    try { grant = await invoke("tool_mcp_client_create", { name: name.trim(), scopes }); if (scopes.includes("transfer")) {
      try { await invoke("tool_mcp_root_grant", { principal: grant!.principal.id, path: folder }); }
      catch (error) { await invoke("tool_mcp_client_revoke", { id: grant!.principal.id }); grant = null; throw error; }
    }
    if (scopes.includes("local_network")) {
      try { await invoke("tool_mcp_network_grant", { principal: grant!.principal.id, endpoints: endpoints.split(/[\n,]+/).map(s => s.trim()).filter(Boolean) }); }
      catch (error) { await invoke("tool_mcp_client_revoke", { id: grant!.principal.id }); grant = null; throw error; }
    }
    if (scopes.includes("orchestration_create")) {
      try { await invoke("tool_mcp_execution_grant", { principal: grant!.principal.id, path: workspace, bots: selectedExecutors, maxTokens, media }); }
      catch (error) { await invoke("tool_mcp_client_revoke", { id: grant!.principal.id }); grant = null; throw error; }
    }
    if (remoteWrite || remoteTransfer) {
      try { await invoke("tool_mcp_gateway_grant", { principal: grant!.principal.id, write: remoteWrite, transfer: remoteTransfer }); }
      catch (error) {
        const failedClient = grant!.principal.id;
        grant = null;
        await invoke("tool_mcp_client_revoke", { id: failedClient });
        throw error;
      }
    }
    gatewayWrite = false; gatewayTransfer = false;
    name = ""; reveal = false; snippets = await invoke("tool_mcp_client_snippets", { token: grant!.token }); await refresh(); }
    catch (e) { message = String(e); }
    finally { busy = false; }
  }
  async function revoke(id: string) {
    busy = true;
    try { await invoke("tool_mcp_client_revoke", { id }); if (grant?.principal.id === id) grant = null; await refresh(); message = $t("mcp_connections.revoked"); }
    catch (e) { message = String(e); }
    finally { busy = false; }
  }
  function config() {
    if (snippets[client]) return snippets[client][1];
    return JSON.stringify({ mcpServers: { omniget: { url, headers: { Authorization: `Bearer ${grant?.token}` } } } }, null, 2);
  }
  async function copy() {
    try { await navigator.clipboard.writeText(config()); message = $t("mcp_connections.copied"); }
    catch (e) { message = String(e); }
  }
  // The Claude Code command reads the token from a hidden prompt, never from
  // its own text: the token is copied on its own.
  const tokenOutsideSnippet = $derived(snippets[client]?.[0] === "Claude Code");
  async function copyToken() {
    try { await navigator.clipboard.writeText(grant?.token ?? ""); message = $t("mcp_connections.copied"); }
    catch (e) { message = String(e); }
  }
</script>

<section aria-label={$t("mcp_connections.title")}>
  <h3>{$t("mcp_connections.title")}</h3>
  <p>{$t("mcp_connections.intro")}</p>
  <form onsubmit={(e) => { e.preventDefault(); void create(); }}>
    <label>{$t("mcp_connections.name")} <input bind:value={name} maxlength="80" required placeholder={$t("mcp_connections.placeholder")} /></label>
    <fieldset disabled={busy}>
      <legend>{$t("mcp_connections.permissions")}</legend>
      {#each options as value}
        <label class="scope"><input type="checkbox" bind:group={scopes} {value} />{$t(`mcp_connections.${value}`)}</label>
      {/each}
    </fieldset>
    {#if scopes.includes("local_network")}
      <label>{$t("mcp_connections.endpoints")} <textarea bind:value={endpoints} rows="2" placeholder="127.0.0.1:8080" required></textarea></label>
      <p>{$t("mcp_connections.endpoints_help")}</p>
    {/if}
    {#if scopes.includes("transfer")}
      <div class="path-row">
        <input bind:value={folder} aria-label={$t("mcp_connections.folder")} placeholder={$t("mission.path.folder_placeholder")} title={folder} autocomplete="off" spellcheck={false} />
        <button class="btn btn-secondary btn-sm path-btn" type="button" onclick={chooseFolder} title={folder || $t("mcp_connections.folder")}>{folder ? shortPath(folder, 28) : $t("mcp_connections.folder")}</button>
      </div>
      <p>{$t("mcp_connections.folder_help")}</p>
    {/if}
    {#if scopes.includes("orchestration_create")}
      <fieldset disabled={busy}>
        <legend>{$t("mcp_connections.execution_grant")}</legend>
        <div class="path-row">
          <input bind:value={workspace} aria-label={$t("mcp_connections.workspace")} placeholder={$t("mission.path.folder_placeholder")} title={workspace} autocomplete="off" spellcheck={false} />
          <button class="btn btn-secondary btn-sm path-btn" type="button" onclick={chooseWorkspace} title={workspace || $t("mcp_connections.workspace")}>{workspace ? shortPath(workspace, 28) : $t("mcp_connections.workspace")}</button>
        </div>
        {#each executors as executor (executor.id)}
          <label class="scope"><input type="checkbox" bind:group={selectedExecutors} value={executor.id} />{executor.name}</label>
        {:else}<p>{$t("mcp_connections.no_executor")}</p>{/each}
        <label>{$t("mcp_connections.max_tokens")} <input type="number" bind:value={maxTokens} min="1" max="100000000" step="1" required /></label>
        <label class="scope"><input type="checkbox" bind:checked={media} />{$t("mcp_connections.media_tools")}</label>
        <p>{$t("mcp_connections.execution_help")}</p>
      </fieldset>
    {/if}
    <fieldset disabled={busy}>
      <legend>{$t("mcp_connections.gateway_title")}</legend>
      <label class="scope"><input type="checkbox" bind:checked={gatewayWrite} disabled={!gatewayCanWrite} />{$t("mcp_connections.gateway_write")}</label>
      <p>{$t("mcp_connections.gateway_write_help")}</p>
      <label class="scope"><input type="checkbox" bind:checked={gatewayTransfer} disabled={!gatewayCanTransfer} />{$t("mcp_connections.gateway_transfer")}</label>
      <p>{$t("mcp_connections.gateway_transfer_help")}</p>
      <p>{$t("mcp_connections.gateway_help")}</p>
    </fieldset>
    <button class="btn btn-secondary btn-sm" disabled={busy || !name.trim() || !url}>{$t("mcp_connections.create")}</button>
  </form>
  <!-- Validation, create and revoke results sit next to the form and the client list, not below the media controls. -->
  {#if message}<p role="status">{message}</p>{/if}
  {#if grant}
    <div class="grant">
      <p>{grant.principal.name}: {$t("mcp_connections.created")}</p>
      <label>{$t("mcp_connections.client")} <select bind:value={client}>{#each snippets as [label], i}<option value={i}>{label}</option>{/each}</select></label>
      <p>{$t("mcp_connections.compatibility")}</p>
      <pre>{reveal ? config() : config().replace(grant.token, "••••••••")}</pre>
      {#if tokenOutsideSnippet}<p>{$t("mcp_connections.claude_code_hint")}</p>{/if}
      <button class="btn btn-secondary btn-sm" type="button" onclick={copy}>{$t("mcp_connections.copy")}</button>
      {#if tokenOutsideSnippet}<button class="btn btn-secondary btn-sm" type="button" onclick={copyToken}>{$t("mcp_connections.copy_token")}</button>{/if}
      <button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal = !reveal}>{reveal ? $t("mcp_connections.hide") : $t("mcp_connections.show")}</button>
    </div>
  {/if}
  <div class="group">
    {#each clients as client (client.id)}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{client.name}</div><div class="group-row-sub">{client.scopes.map((s) => options.includes(s) ? $t(`mcp_connections.${s}`) : s).join(" · ")}</div></div>
        <button class="btn btn-ghost btn-sm" type="button" disabled={busy} onclick={() => revoke(client.id)}>{$t("mcp_connections.revoke")}</button>
      </div>
    {:else}<p>{$t("mcp_connections.empty")}</p>{/each}
  </div>
  {#if activeGrants.length || fileRoots.length}
    <div class="group" aria-label={$t("mcp_connections.grants_title")}>
      <h4>{$t("mcp_connections.grants_title")}</h4>
      {#each activeGrants as g (g.id)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{g.principal_name} · {$t("mcp_connections.execution_grant")}</div>
            <div class="group-row-sub"><span class="path" title={g.workspace_full}>{shortPath(g.workspace, 56)}</span></div>
            <div class="group-row-sub">
              {$t("mcp_connections.grant_executors", { names: g.executors.map((e) => e.name).join(", ") || "—" })}
              · {$t("mcp_connections.grant_max_tokens", { count: g.max_tokens })}
              {#if g.media}· {$t("mcp_connections.media_tools")}{/if}
            </div>
          </div>
          <button class="btn btn-ghost btn-sm" type="button" disabled={busy} onclick={() => revokeGrant(g.id)}>{$t("mcp_connections.revoke")}</button>
        </div>
        {#each derivedOf(g.id) as d (d.bot)}
          <div class="group-row derived">
            <div class="group-row-content">
              <div class="group-row-title">
                {d.name}
                <span class="ext-tag">{$t("mcp_connections.derived_external", { name: d.principal_name })}</span>
                {#if d.retired}<span class="ext-tag muted">{$t("mcp_connections.derived_retired")}</span>{/if}
              </div>
              <div class="group-row-sub">{$t("mcp_connections.derived_from", { name: d.source_name })} · {$t("mcp_connections.derived_read_only")}</div>
            </div>
            {#if !d.retired}
              <button class="btn btn-ghost btn-sm" type="button" disabled={busy} onclick={() => revokeDerived(d.bot)}>{$t("mcp_connections.revoke")}</button>
            {/if}
          </div>
        {/each}
      {/each}
      {#each fileRoots as r (r.id)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{r.principal_name} · {$t("mcp_connections.transfer_root")}</div>
            <div class="group-row-sub"><span class="path" title={r.path_full}>{shortPath(r.path, 56)}</span></div>
          </div>
          <button class="btn btn-ghost btn-sm" type="button" disabled={busy} onclick={() => revokeRoot(r.id)}>{$t("mcp_connections.revoke")}</button>
        </div>
      {/each}
    </div>
  {/if}
  <section class="media-controls" aria-label={$t("mcp_connections.media_local_title")}>
    <h3>{$t("mcp_connections.media_local_title")}</h3>
    <p>{$t("mcp_connections.media_local_help")}</p>
    <form onsubmit={(e) => { e.preventDefault(); void configureMedia(); }}>
      <fieldset disabled={mediaBusy}>
        <legend>{$t("mcp_connections.media_cli_title")}</legend>
        <label>{$t("mcp_connections.media_cli_path")} <input bind:value={mediaCli} maxlength="4096" required autocomplete="off" spellcheck={false} /></label>
        <button class="btn btn-secondary btn-sm" type="button" onclick={chooseMediaCli}>{$t("mcp_connections.media_cli_choose")}</button>
        <p>{$t("mcp_connections.media_cli_help")}</p>
        <button class="btn btn-secondary btn-sm" disabled={!mediaCli.trim()}>{$t("mcp_connections.media_cli_save")}</button>
      </fieldset>
    </form>
    <form onsubmit={(e) => { e.preventDefault(); void authorizeMediaBudget(); }}>
      <fieldset disabled={mediaBusy || busy}>
        <legend>{$t("mcp_connections.media_budget_title")}</legend>
        <label>{$t("mcp_connections.client")}
          <select bind:value={budgetClient} required>
            <option value="">{$t("mcp_connections.media_client_choose")}</option>
            {#each clients as connection (connection.id)}<option value={connection.id}>{connection.name} ({connection.id})</option>{/each}
          </select>
        </label>
        <label>{$t("mcp_connections.media_mission_id")} <input bind:value={budgetMission} maxlength="128" required autocomplete="off" spellcheck={false} /></label>
        <p>{$t("mcp_connections.media_mission_help")}</p>
        <label>{$t("mcp_connections.media_credits")} <input type="text" inputmode="decimal" bind:value={budgetCredits} maxlength="14" required autocomplete="off" /></label>
        <p>{$t("mcp_connections.media_credits_help")}</p>
        <p>{$t("mcp_connections.media_budget_help")}</p>
        <button class="btn btn-secondary btn-sm" disabled={microcredits === null || !validMission || !clients.some(c => c.id === budgetClient)}>{$t("mcp_connections.media_budget_authorize")}</button>
      </fieldset>
    </form>
    {#if mediaMessage}<p role="status">{mediaMessage}</p>{/if}
  </section>
</section>
<style>
  section { display: grid; gap: var(--space-3); }
  h3, p { margin: 0; } p { color: var(--text-muted); font-size: var(--text-sm); }
  form { display: grid; gap: var(--space-3); } label { display: grid; gap: var(--space-2); }
  input:not([type="checkbox"]) { padding: var(--space-2); border: 1px solid var(--input-border); border-radius: var(--radius-sm); background: transparent; color: inherit; }
  fieldset { border: 1px solid var(--border); border-radius: var(--radius-sm); display: grid; gap: var(--space-2); }
  .scope { display: flex; align-items: center; gap: var(--space-2); font-size: var(--text-sm); }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; font-family: var(--font-mono); font-size: var(--text-xs); }
  .media-controls { border-top: 1px solid var(--border); padding-top: var(--space-4); }
  .path-row { display: flex; gap: var(--space-2); align-items: center; min-width: 0; }
  .path-row input { flex: 1 1 auto; min-width: 0; font-family: var(--font-mono); font-size: var(--text-xs); text-overflow: ellipsis; }
  .path-btn { flex-shrink: 0; max-width: 45%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* shortPath cuts the middle so the folder name stays visible; never cut the end. */
  .path { display: inline-block; max-width: 100%; overflow-wrap: anywhere; vertical-align: bottom; font-family: var(--font-mono); }
  .derived { padding-left: var(--space-4); }
  .ext-tag { margin-left: var(--space-1); padding: 0 6px; border-radius: 999px; font-size: var(--text-xs); font-weight: 400; color: var(--accent-text); background: var(--accent-soft); }
  .ext-tag.muted { color: var(--text-muted); background: var(--fill-2); }
  h4 { margin: 0; font-size: var(--text-sm); }
  .grant { border: 1px solid var(--border); border-radius: var(--radius-sm); padding: var(--space-3); }
</style>
