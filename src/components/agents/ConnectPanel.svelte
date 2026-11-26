<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { maskSnippet } from "$lib/mcp-servers";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SettingGroup from "#components/settings/rebrand/SettingGroup.svelte";
  import AgentLogo from "./AgentLogo.svelte";
  import ServerConnections from "./ServerConnections.svelte";
  import McpAccess from "./McpAccess.svelte";
  import ConnectAnyAi from "./ConnectAnyAi.svelte";
  import AdvancedDisclosure from "#components/settings/ai/AdvancedDisclosure.svelte";
  import SettingsEmptyState from "#components/settings/rebrand/SettingsEmptyState.svelte";

  type Tool = { id: string; name: string; installed: boolean; connected: boolean; skill: boolean };
  type Status = { enabled: boolean; bridge_enabled: boolean; port: number; url: string; token: string; snippets: [string, string][] };

  let status = $state<Status | null>(null);
  let tools = $state<Tool[]>([]);
  let prompt = $state("");
  let working = $state<string | null>(null);
  let browser = $state(false);
  let client = $state(0);
  let showAll = $state(false);
  let loading = $state(true);
  let failed = $state(false);

  let installed = $derived(tools.filter((tool) => tool.installed));
  let shown = $derived(showAll ? tools : installed);
  let missingSkill = $derived(installed.filter((tool) => !tool.skill).length);
  let snippet = $derived(status?.snippets?.[client]?.[1] ?? "");
  let tokenOutside = $derived(status?.snippets?.[client]?.[0] === "Claude Code");

  async function refresh() {
    try {
      status = await invoke<Status | null>("tool_mcp_status");
      [tools, prompt] = await Promise.all([invoke<Tool[]>("connect_tools"), invoke<string>("connect_prompt")]);
      failed = false;
    } catch (error) {
      failed = true;
      if (!loading) showToast("error", String(error));
    } finally {
      loading = false;
    }
  }

  function retry() {
    loading = true;
    failed = false;
    void refresh();
  }

  onMount(() => {
    void refresh();
    const stop = listen("connect-changed", () => void refresh());
    return () => void stop.then((unlisten) => unlisten());
  });

  async function run(id: string, action: () => Promise<void>) {
    working = id;
    try {
      await action();
      await refresh();
    } catch (error) {
      showToast("error", String(error));
    } finally {
      working = null;
    }
  }

  const toggleServer = () => run("server", async () => {
    await invoke("tool_mcp_set_enabled", { enabled: !status?.enabled });
  });

  const connect = (tool: Tool) => run(tool.id, async () => {
    const result = await invoke<{ message: string }>("connect_tool", { client: tool.id, workspace: null, agents: true, approvals: false, browser });
    showToast("success", result.message);
  });

  const installSkill = () => run("skill", async () => {
    const result = await invoke<Record<string, { ok: boolean; error?: string }>>("connect_install_skill", { targets: [] });
    const failed = Object.entries(result).filter(([, value]) => !value.ok);
    const ok = Object.keys(result).length - failed.length;
    if (failed.length) showToast("info", $t("connect.skill_partial", { ok, failed: failed.map(([name]) => name).join(", ") }) as string);
    else showToast("success", $t("connect.skill_done", { count: ok }) as string);
  });

  async function copy(text: string) {
    await navigator.clipboard.writeText(text);
    showToast("success", $t("agents.mcp.copied") as string);
  }
</script>

<SettingGroup id="connect" icon="plug" title={$t("settings.agents.connect_title") as string} description={$t("settings.agents.connect_desc") as string}>
  {#snippet actions()}
    <button class="toggle" class:on={status?.enabled} type="button" role="switch" aria-checked={status?.enabled ?? false} aria-label={$t("settings.agents.connect_title") as string} disabled={!status || working !== null} onclick={toggleServer}><span class="toggle-knob"></span></button>
  {/snippet}
  <div data-setting-key="settings.agents.connect_title">
  {#if loading}
    <p class="empty" role="status">{$t("settings.rebrand.state.loading")}</p>
  {:else if failed && !status}
    <SettingsEmptyState error title={$t("settings.rebrand.state.unavailable") as string} onRetry={retry} />
  {:else}
  {#if status && !status.enabled}<p class="warn" role="status"><bdi>{$t("settings.ai.connect_off")}</bdi></p>
  {:else if status && !status.bridge_enabled}<p class="warn" role="status">{$t("agents.mcp.bridge_off")}</p>{/if}
  <ConnectAnyAi enabled={status?.enabled ?? false} />
  <McpAccess />
  <div class="grid">
    {#each shown as tool (tool.id)}
      <div class="tool" class:connected={tool.connected}>
        <AgentLogo id={tool.id} name={tool.name} size={22} />
        <div class="tool-text">
          <span class="tool-name"><bdi>{tool.name}</bdi></span>
          <small>{tool.connected ? $t("connect.state_connected") : tool.installed ? $t("connect.state_found") : $t("connect.state_missing")}</small>
        </div>
        <button type="button" class="btn btn-secondary btn-sm" disabled={working !== null || !status?.enabled} onclick={() => connect(tool)}>
          {working === tool.id ? $t("connect.connecting") : tool.connected ? $t("connect.reconnect") : $t("connect.connect")}
        </button>
      </div>
    {:else}
      <p class="empty">{$t("connect.none_found")}</p>
    {/each}
  </div>
  {/if}
  </div>
  {#if tools.length > installed.length}
    <button type="button" class="link" onclick={() => (showAll = !showAll)}>{showAll ? $t("connect.show_installed") : $t("connect.show_all", { count: tools.length })}</button>
  {/if}

  <AdvancedDisclosure id="connect">
    <label class="check" data-setting-key="connect.scope_browser">
      <input type="checkbox" bind:checked={browser} />
      <span><strong>{$t("connect.scope_browser")}</strong><small>{$t("connect.scope_browser_hint")}</small></span>
    </label>
    <div class="line" data-setting-key="connect.skill_title">
      <span><strong>{$t("connect.skill_title")}</strong><small>{$t("connect.skill_intro")}</small></span>
      <button type="button" class="btn btn-secondary btn-sm" disabled={working !== null || installed.length === 0} onclick={installSkill}>{missingSkill > 0 ? $t("connect.skill_install", { count: missingSkill }) : $t("connect.skill_reinstall")}</button>
    </div>
    {#if prompt}
      <div class="line">
        <span><strong>{$t("connect.prompt_title")}</strong><small class="mono"><bdi>{prompt}</bdi></small></span>
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => copy(prompt)}>{$t("agents.mcp.copy")}</button>
      </div>
    {/if}
    {#if status?.url}
      <div class="line">
        <span><strong>{$t("agents.mcp.endpoint")}</strong><small class="mono">{status.url}</small></span>
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => copy(status!.url)}>{$t("agents.mcp.copy")}</button>
      </div>
    {/if}
    {#if status && status.snippets.length > 0}
      <div class="snippets">
        <div class="snippet-tabs">
          {#each status.snippets as [name], index (name)}
            <button type="button" class="btn btn-sm" class:btn-primary={client === index} class:btn-secondary={client !== index} onclick={() => (client = index)}>{name}</button>
          {/each}
        </div>
        <pre><bdi>{maskSnippet(snippet, status.token, false)}</bdi></pre>
        <div class="snippet-tabs">
          <button type="button" class="btn btn-secondary btn-sm" onclick={() => copy(snippet)}>{$t("agents.mcp.copy")}</button>
          {#if tokenOutside}<button type="button" class="btn btn-secondary btn-sm" onclick={() => copy(status!.token)}>{$t("mcp_connections.copy_token")}</button>{/if}
        </div>
      </div>
    {/if}
    {#if status}<ServerConnections url={status.url} />{/if}
  </AdvancedDisclosure>
</SettingGroup>

<style>
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 280px), 1fr)); gap: var(--space-2); }
  .tool {
    display: flex; align-items: center; gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--content-border);
    border-radius: var(--radius-md);
    background: var(--primary);
  }
  .tool.connected { border-color: var(--success); }
  .tool-text { flex: 1; display: flex; flex-direction: column; min-inline-size: 0; }
  .tool-name { font-size: 13px; line-height: 18px; font-weight: 600; color: var(--text); overflow-wrap: anywhere; }
  small { display: block; font-size: 12px; line-height: 17px; color: var(--text-muted); }
  strong { display: block; font-size: 13px; line-height: 18px; font-weight: 600; color: var(--text); }
  .warn, .empty { margin: 0 0 var(--space-2); font-size: 12px; color: var(--text-muted); }
  .link { align-self: flex-start; margin-block-start: var(--space-2); padding: 0; border: 0; background: none; color: var(--accent-text, var(--accent)); font: inherit; font-size: 12px; cursor: pointer; }
  .check, .line { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--space-3); padding-block: var(--space-2); border-block-start: 1px solid var(--separator); }
  .check { justify-content: flex-start; cursor: pointer; }
  .line > span { min-inline-size: 0; }
  .mono { font-family: var(--font-mono); overflow-wrap: anywhere; }
  .snippets { display: flex; flex-direction: column; gap: var(--space-2); padding-block: var(--space-2); border-block-start: 1px solid var(--separator); }
  .snippet-tabs { display: flex; flex-wrap: wrap; gap: var(--space-1); }
  pre { margin: 0; padding: var(--space-2); border-radius: var(--radius-sm); background: var(--primary); font-family: var(--font-mono); font-size: 11px; white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
