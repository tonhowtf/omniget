<script lang="ts">
  // "Agentes ACP" do registro oficial: instalar pela distribuição declarada
  // (npx/uvx registra o comando; binário baixa com sha256) e adicionar o
  // comando pronto ao roster.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { acpInstall, acpRegistry, errorCode, type AcpAgent, type AcpLaunch, type AcpRegistry } from "$lib/central/clitools";

  let { onstart }: { onstart?: (agentId: string, title: string) => void } = $props();

  let reg = $state<AcpRegistry | null>(null);
  let error = $state("");
  let loading = $state(false);
  let busy = $state<string | null>(null);
  let query = $state("");

  async function load(force = false) {
    loading = true;
    error = "";
    try {
      reg = await acpRegistry(force);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  let agents = $derived(
    (reg?.agents ?? []).filter((a) => {
      const q = query.trim().toLowerCase();
      return !q || a.name.toLowerCase().includes(q) || a.id.includes(q) || (a.description ?? "").toLowerCase().includes(q);
    }),
  );

  function launchLine(l: AcpLaunch): string {
    return [l.command, ...l.args].join(" ");
  }

  async function install(a: AcpAgent, method: string | null = null) {
    busy = a.id;
    onstart?.(a.id, a.name);
    try {
      let launch: AcpLaunch;
      try {
        launch = await acpInstall(a.id, method, false);
      } catch (e) {
        if (errorCode(e) !== "CLITOOLS_UNVERIFIED" || !confirm($t("llm.central.tools.acp_unverified_confirm", { name: a.name }) as string)) throw e;
        launch = await acpInstall(a.id, method, true);
      }
      showToast("success", $t("llm.central.tools.acp_installed", { name: a.name }) as string);
      for (const w of launch.warnings) showToast("info", w);
      await load(false);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = null;
    }
  }

  async function addToRoster(a: AcpAgent) {
    const l = a.installed;
    if (!l) return;
    try {
      await invoke("llm_acp_agent_create", { name: a.name, command: l.command, args: l.args });
      showToast("success", $t("llm.central.tools.acp_added", { name: a.name }) as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  onMount(() => {
    void load(false);
  });
</script>

<section class="acp">
  <header class="acp-head">
    <div>
      <h2 class="section-title">{$t("llm.central.tools.acp_title")}</h2>
      <p class="hint">{$t("llm.central.tools.acp_hint")}</p>
    </div>
    <input class="input input-search" type="search" placeholder={$t("llm.central.tools.search") as string} bind:value={query} />
    <button class="button" type="button" disabled={loading} onclick={() => load(true)}>{$t("llm.central.tools.refresh")}</button>
  </header>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
  {#if reg?.stale}<p class="hint">{$t("llm.central.tools.acp_stale")} {reg.error}</p>{/if}

  <ul class="agents">
    {#each agents as a (a.id)}
      <li class="surface-card agent">
        <div class="info">
          <strong>{a.name}</strong>
          <span class="dim">{a.version}</span>
          {#each a.methods as m}<span class="tag">{m}</span>{/each}
          {#if a.binary_here && !a.binary_verified}<span class="tag tag-warning">{$t("llm.central.tools.no_sha")}</span>{/if}
          {#if a.license}<span class="dim">{a.license}</span>{/if}
        </div>
        {#if a.description}<p class="desc">{a.description}</p>{/if}
        {#if a.installed}
          <code class="launch">{launchLine(a.installed)}</code>
        {/if}
        <div class="row-actions">
          <button class="button" class:active={!a.installed} type="button" disabled={busy !== null} onclick={() => install(a)}>
            {a.installed ? $t("llm.central.tools.reinstall") : $t("llm.central.tools.install")}
          </button>
          {#if a.installed}
            <button class="button" type="button" onclick={() => addToRoster(a)}>{$t("llm.central.tools.acp_add")}</button>
          {/if}
          {#if a.website || a.repository}
            <a class="docs" href={a.website ?? a.repository ?? "#"} target="_blank" rel="noreferrer">{$t("llm.central.tools.docs")}</a>
          {/if}
          {#if busy === a.id}<span class="tag tag-info">{$t("llm.central.tools.running")}</span>{/if}
        </div>
      </li>
    {/each}
  </ul>
</section>

<style>
  .acp {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .acp-head {
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .acp-head > div {
    flex: 1;
    min-width: 220px;
  }
  .acp-head .input {
    width: 220px;
  }
  .hint,
  .dim {
    margin: 0;
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .err {
    color: var(--danger);
    font-size: var(--text-sm);
  }
  .agents {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: var(--space-3);
  }
  .agent {
    padding: var(--space-3) var(--space-4);
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .info {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .desc {
    margin: 0;
    font-size: var(--text-caption);
    color: var(--text-muted);
  }
  .launch {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    background: var(--fill-1);
    border-radius: var(--radius-sm);
    padding: 4px 6px;
    word-break: break-all;
    user-select: text;
  }
  .row-actions {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    flex-wrap: wrap;
    margin-top: auto;
  }
  .docs {
    font-size: var(--text-caption);
    color: var(--text-muted);
  }
</style>
