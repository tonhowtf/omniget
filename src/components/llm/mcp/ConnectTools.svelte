<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Tool = { id: string; name: string; installed: boolean; connected: boolean; skill: boolean };

  let { onchange }: { onchange?: () => void } = $props();

  let tools = $state<Tool[]>([]);
  let showAll = $state(false);
  let busy = $state<string | null>(null);
  let prompt = $state("");
  let loaded = $state(false);

  let installed = $derived(tools.filter((x) => x.installed));
  let shown = $derived(showAll ? tools : installed);
  let missingSkill = $derived(installed.filter((x) => !x.skill).length);

  async function refresh() {
    try {
      [tools, prompt] = await Promise.all([
        invoke<Tool[]>("connect_tools"),
        invoke<string>("connect_prompt"),
      ]);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      loaded = true;
    }
  }

  onMount(() => {
    void refresh();
    const un = listen("connect-changed", () => {
      void refresh();
      onchange?.();
    });
    return () => void un.then((f) => f());
  });

  async function connect(tool: Tool) {
    busy = tool.id;
    try {
      const r = await invoke<{ message: string }>("connect_tool", {
        client: tool.id,
        workspace: null,
        agents: true,
        approvals: false,
      });
      showToast("success", r.message);
      await refresh();
      onchange?.();
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = null;
    }
  }

  async function installSkill() {
    busy = "skill";
    try {
      const r = await invoke<Record<string, { ok: boolean; error?: string }>>("connect_install_skill", { targets: [] });
      const failed = Object.entries(r).filter(([, v]) => !v.ok);
      const ok = Object.keys(r).length - failed.length;
      if (failed.length) {
        showToast("info", $t("connect.skill_partial", { ok, failed: failed.map(([k]) => k).join(", ") }) as string);
      } else {
        showToast("success", $t("connect.skill_done", { count: ok }) as string);
      }
      await refresh();
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = null;
    }
  }

  async function copyPrompt() {
    await navigator.clipboard.writeText(prompt);
    showToast("success", $t("llm.mcp.copied") as string);
  }
</script>

<div class="group" aria-busy={!loaded}>
  <div class="group-row">
    <div class="group-row-content">
      <div class="group-row-title">{$t("connect.tools_title")}</div>
      <div class="group-row-sub">{$t("connect.tools_intro")}</div>
    </div>
  </div>

  {#each shown as tool (tool.id)}
    <div class="group-row">
      <div class="group-row-content">
        <div class="group-row-title">{tool.name}</div>
        <div class="group-row-sub">
          {tool.connected ? $t("connect.state_connected") : tool.installed ? $t("connect.state_found") : $t("connect.state_missing")}
          {#if tool.skill}· {$t("connect.state_skill")}{/if}
        </div>
      </div>
      <div class="group-row-trailing">
        <button type="button" class="button" disabled={busy !== null} onclick={() => connect(tool)}>
          {busy === tool.id ? $t("connect.connecting") : tool.connected ? $t("connect.reconnect") : $t("connect.connect")}
        </button>
      </div>
    </div>
  {/each}

  {#if loaded && installed.length === 0 && !showAll}
    <div class="group-row">
      <div class="group-row-content"><div class="group-row-sub">{$t("connect.none_found")}</div></div>
    </div>
  {/if}

  <div class="group-row">
    <div class="group-row-content">
      <div class="group-row-title">{$t("connect.skill_title")}</div>
      <div class="group-row-sub">{$t("connect.skill_intro")}</div>
    </div>
    <div class="group-row-trailing">
      <button type="button" class="button" disabled={busy !== null || installed.length === 0} onclick={installSkill}>
        {missingSkill > 0 ? $t("connect.skill_install", { count: missingSkill }) : $t("connect.skill_reinstall")}
      </button>
    </div>
  </div>

  {#if prompt}
    <div class="group-row">
      <div class="group-row-content">
        <div class="group-row-title">{$t("connect.prompt_title")}</div>
        <div class="group-row-sub mono">{prompt}</div>
      </div>
      <div class="group-row-trailing">
        <button type="button" class="button" onclick={copyPrompt}>{$t("llm.mcp.copy")}</button>
      </div>
    </div>
  {/if}

  {#if tools.length > installed.length}
    <div class="group-row">
      <div class="group-row-content">
        <button type="button" class="link" onclick={() => (showAll = !showAll)}>
          {showAll ? $t("connect.show_installed") : $t("connect.show_all", { count: tools.length })}
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    overflow-wrap: anywhere;
  }

  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
  }
</style>
