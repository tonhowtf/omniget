<script lang="ts">
  import { parseCommand } from "$lib/llm/connection-setup";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { loadRoster } from "$lib/stores/llm-store.svelte";

  type AcpCli = { id: string; name: string; command: string; args: string[]; path: string | null; installed: boolean };
  let detectionError = $state("");
  let clis = $state<AcpCli[]>([]);
  let busy = $state<string | null>(null);
  let customName = $state("");
  let customCommand = $state("");

  async function detect() {
    detectionError = "";
    try {
      clis = (await invoke("llm_acp_detect")) as AcpCli[];
    } catch (error) {
      detectionError = String(error);
    }
  }
  onMount(() => { void detect(); });

  async function add(name: string, command: string, args: string[]) {
    busy = command;
    try {
      await invoke("llm_acp_agent_create", { name, command, args });
      await loadRoster();
      showToast("success", $t("llm.accounts.acp_added", { name }) as string);
      return true;
    } catch (error) {
      showToast("error", String(error));
      return false;
    } finally {
      busy = null;
    }
  }

  async function addCustom() {
    if (busy) return;
    let parts: string[];
    try { parts = parseCommand(customCommand.trim()); }
    catch (error) { showToast("error", String(error)); return; }
    if (parts.length === 0) return;
    if (await add(customName.trim() || parts[0], parts[0], parts.slice(1))) {
      customName = "";
      customCommand = "";
    }
  }
</script>

<section class="surface-card acp">
  <h2 class="section-title">{$t("llm.accounts.acp_title")}</h2>
  <p class="field-hint">{$t("llm.accounts.acp_hint")}</p>
  {#if detectionError}<p role="alert">{detectionError}</p><button class="button" onclick={detect}>{$t("llm.accounts.refresh")}</button>{/if}
  <ul class="acp-list">
    {#each clis as cli (cli.id)}
      <li class="acp-row" class:missing={!cli.installed}>
        <div class="acp-text">
          <span class="acp-name">{cli.name}</span>
          <code class="acp-cmd">{cli.path ?? `${cli.command} ${cli.args.join(" ")}`}</code>
        </div>
        {#if cli.installed}
          <button type="button" class="button" disabled={busy !== null} onclick={() => add(cli.name, cli.command, cli.args)}>
            {$t("llm.accounts.acp_add")}
          </button>
        {:else}
          <span class="acp-state">{$t("llm.accounts.acp_missing")}</span>
        {/if}
      </li>
    {/each}
  </ul>
  <form class="acp-custom" onsubmit={(e) => { e.preventDefault(); addCustom(); }}>
    <label>{$t("llm.accounts.acp_custom_name")}<input disabled={busy !== null} type="text" bind:value={customName} /></label>
    <label class="grow">{$t("llm.accounts.acp_command")}<input disabled={busy !== null} type="text" bind:value={customCommand} placeholder="my-agent --acp" spellcheck="false" autocapitalize="off" /></label>
    <button type="submit" class="button" disabled={busy !== null || !customCommand.trim()}>{$t("llm.accounts.acp_add")}</button>
  </form>
</section>

<style>
  .acp {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-4);
    padding: var(--space-5);
  }
  .acp-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .acp-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .acp-row.missing {
    color: var(--text-muted);
  }
  .acp-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .acp-name {
    font-weight: 600;
  }
  .acp-cmd {
    font-size: var(--text-xs, 11px);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .acp-state {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .acp-custom label { display: flex; flex-direction: column; gap: var(--space-2); font-size: var(--text-sm); min-width: 0; }
  .acp-custom {
    flex-wrap: wrap;
    align-items: flex-end;
    display: flex;
    gap: var(--space-2);
  }
  .acp-custom input {
    padding: 6px 10px;
    border-radius: var(--radius-md, 8px);
    border: 1px solid var(--separator, rgba(127, 127, 127, 0.3));
    background: var(--fill-quaternary, rgba(127, 127, 127, 0.08));
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    min-width: 0;
  }
  .acp-custom .grow {
    flex: 1;
    font-family: var(--font-mono, ui-monospace, monospace);
  }
</style>
