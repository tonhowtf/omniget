<script lang="ts">
  /** Servidor MCP embutido (estudos 22 e 23): liga/desliga, endereço, trechos de configuração e a lista de tools. */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/tools/rt";

  type Tool = { name: string; description: string; inputSchema: { properties?: Record<string, unknown>; required?: string[] } };
  type Status = { enabled: boolean; bridge_enabled: boolean; port: number; url: string; token: string; tools: Tool[]; snippets: [string, string][] };

  let status = $state<Status | null>(null);
  let busy = $state(false);
  let client = $state(0);
  let showToken = $state(false);
  let selftest = $state("");

  async function refresh() { try { status = await invoke<Status>("tool_mcp_status"); } catch (e) { showToast("error", errText(e)); } }
  onMount(refresh);

  async function toggle() {
    if (!status || busy) return;
    busy = true;
    try { await invoke("tool_mcp_set_enabled", { enabled: !status.enabled }); await refresh(); }
    catch (e) { showToast("error", errText(e)); } finally { busy = false; }
  }
  async function test() {
    busy = true; selftest = "";
    try { selftest = await invoke<string>("tool_mcp_selftest"); showToast("success", "OK"); }
    catch (e) { selftest = errText(e); showToast("error", selftest); } finally { busy = false; }
  }
  async function copy(text: string) { await navigator.clipboard.writeText(text); showToast("success", $t("tools.common.copied") as string); }
  const mask = (tok: string) => (tok.length > 8 ? `${tok.slice(0, 4)}…${tok.slice(-4)}` : "••••");
  const snippetMasked = (s: string) => (status && !showToken ? s.replaceAll(status.token, mask(status.token)) : s);
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.mcp.server")}</div>
          <div class="group-row-sub">{status?.enabled ? $t("tools.mcp.on") : $t("tools.mcp.off")}{#if status && !status.bridge_enabled} · <span class="err">{$t("tools.mcp.bridge_off")}</span>{/if}</div>
        </div>
        <div class="group-row-trailing"><button class="toggle" class:on={status?.enabled} type="button" role="switch" aria-checked={status?.enabled ?? false} aria-label={$t("tools.mcp.server")} disabled={!status || busy} onclick={toggle}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.mcp.endpoint")}</div><div class="group-row-sub mono">{status?.url || "…"}</div></div>
        <div class="group-row-trailing btn-row">{#if status?.url}<button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(status!.url)}>{$t("tools.common.copy")}</button>{/if}<button class="btn btn-secondary btn-sm" type="button" disabled={busy || !status?.enabled} onclick={test}>{$t("tools.mcp.test")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.mcp.token")}</div><div class="group-row-sub mono">{status ? (showToken ? status.token : mask(status.token)) : "…"}</div><div class="group-row-sub">{$t("tools.mcp.token_hint")}</div></div>
        <div class="group-row-trailing btn-row"><button class="btn btn-ghost btn-sm" type="button" onclick={() => (showToken = !showToken)}>{showToken ? $t("tools.mcp.hide") : $t("tools.mcp.show")}</button>{#if status}<button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(status!.token)}>{$t("tools.common.copy")}</button>{/if}</div>
      </div>
      {#if selftest}<div class="group-row"><div class="group-row-sub mono">{selftest}</div></div>{/if}
    </div>
  </section>

  {#if status}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.mcp.connect")}</div><div class="group-row-sub">{$t("tools.mcp.connect_hint")}</div></div>
          <div class="group-row-trailing btn-row wrap">
            {#each status.snippets as [name], i (name)}
              <button class="btn btn-sm" class:btn-secondary={client === i} class:btn-ghost={client !== i} type="button" onclick={() => (client = i)}>{name}</button>
            {/each}
          </div>
        </div>
        <div class="group-row"><div class="group-row-content"><pre class="code">{snippetMasked(status.snippets[client]?.[1] ?? "")}</pre></div><div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(status!.snippets[client][1])}>{$t("tools.common.copy")}</button></div></div>
      </div>
    </section>

    <section>
      <h3 class="group-title">{status.tools.length} {$t("tools.mcp.tools")}</h3>
      <div class="group">
        {#each status.tools as tool (tool.name)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title mono">{tool.name}</div>
              <div class="group-row-sub">{tool.description}</div>
              {#if tool.inputSchema.properties && Object.keys(tool.inputSchema.properties).length}
                <div class="group-row-sub mono">{Object.keys(tool.inputSchema.properties).map((k) => (tool.inputSchema.required?.includes(k) ? k : `${k}?`)).join(", ")}</div>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .err { color: var(--danger); }
  .wrap { flex-wrap: wrap; justify-content: flex-end; }
  .code { margin: 0; white-space: pre-wrap; font-family: var(--font-mono); font-size: var(--text-xs); max-height: 260px; overflow: auto; }
  .group-title { margin: 0 0 var(--space-2); font-size: var(--text-sm); font-weight: 600; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em; }
</style>
