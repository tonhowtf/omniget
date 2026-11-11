<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/devtools/api";
  import { filterPorts } from "$lib/devtools/helpers";
  import type { DevtoolsError, PortListener, PortTarget } from "$lib/devtools/types";
  import ErrorNotice from "./ErrorNotice.svelte";

  let listeners = $state<PortListener[] | null>(null);
  let query = $state("");
  let busy = $state(false);
  let error = $state<DevtoolsError | null>(null);
  let pending = $state<{ port: number; targets: PortTarget[] } | null>(null);
  let stopping = $state(false);
  let notice = $state<string | null>(null);

  let shown = $derived(filterPorts(listeners ?? [], query));

  async function refresh() {
    busy = true;
    try {
      listeners = await api.portsList();
      error = null;
    } catch (e) {
      error = api.parseError(e);
    } finally {
      busy = false;
    }
  }

  async function ask(listener: PortListener) {
    error = null;
    notice = null;
    try {
      const plan = await api.portsKill({ port: listener.port }, false);
      pending = { port: listener.port, targets: plan.targets };
    } catch (e) {
      error = api.parseError(e);
    }
  }

  async function stop() {
    if (!pending || stopping) return;
    const { port, targets } = pending;
    stopping = true;
    try {
      const done = await api.portsKill({ port }, true);
      const failed = done.failed[0];
      notice = failed
        ? ($t("devtools.ports.stop_failed", { pid: failed.pid, reason: failed.reason }) as string)
        : ($t("devtools.ports.stopped", { process: targets.map((target) => target.process).join(", "), port }) as string);
    } catch (e) {
      error = api.parseError(e);
    } finally {
      stopping = false;
      pending = null;
      await refresh();
    }
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "Escape" && pending && !stopping) pending = null;
  }

  onMount(() => void refresh());
</script>

<svelte:window onkeydown={keydown} />

<div class="dv-panel" aria-busy={busy}>
  <div class="dv-row">
    <label class="dv-field wide">
      {$t("devtools.ports.filter")}
      <input class="input dv-mono" type="search" bind:value={query} placeholder="3000, node, vite" spellcheck="false" autocomplete="off" />
    </label>
    <button type="button" class="btn btn-secondary" disabled={busy} onclick={refresh}>
      {#if busy}<span class="spinner" aria-hidden="true"></span>{/if}
      {$t("devtools.ports.refresh")}
    </button>
  </div>

  {#if notice}
    <p class="dv-meta" role="status">{notice}</p>
  {/if}

  {#if listeners === null}
    {#if !error}<p class="dv-empty">{$t("common.loading")}</p>{/if}
  {:else if listeners.length === 0}
    <p class="dv-empty">{$t("devtools.ports.empty")}</p>
  {:else if shown.length === 0}
    <p class="dv-empty">{$t("devtools.ports.no_match", { query })}</p>
  {:else}
    <div class="dv-scroll">
      <table class="dv-table">
        <thead>
          <tr>
            <th class="num">{$t("devtools.ports.col_port")}</th>
            <th>{$t("devtools.ports.col_process")}</th>
            <th class="num">PID</th>
            <th class="pp-command">{$t("devtools.ports.col_command")}</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each shown as listener (`${listener.port}-${listener.pid}`)}
            <tr>
              <td class="num dv-mono pp-port">{listener.port}</td>
              <td>{listener.process}</td>
              <td class="num dim dv-mono">{listener.pid}</td>
              <td class="dim dv-mono pp-command" title={listener.command ?? ""}>{listener.command ?? ""}</td>
              <td class="num">
                <button type="button" class="btn btn-secondary btn-sm" onclick={() => ask(listener)}>{$t("devtools.ports.stop")}</button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

{#if error}
  <ErrorNotice {error} />
{/if}

{#if pending}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="dialog-backdrop" onclick={(e) => e.target === e.currentTarget && !stopping && (pending = null)}>
    <div class="dialog-card" role="alertdialog" aria-modal="true" aria-labelledby="pp-title" aria-describedby="pp-body">
      <h2 class="dialog-title" id="pp-title">{$t("devtools.ports.confirm_title", { port: pending.port })}</h2>
      <div class="dialog-body" id="pp-body">
        <p>{$t("devtools.ports.confirm_body")}</p>
        <ul class="pp-targets">
          {#each pending.targets as target (target.pid)}
            <li>
              <b>{target.process}</b> <span class="dv-mono">PID {target.pid}</span>
              {#if target.command}<span class="pp-target-command">{target.command}</span>{/if}
            </li>
          {/each}
        </ul>
      </div>
      <div class="dialog-actions">
        <!-- svelte-ignore a11y_autofocus -->
        <button type="button" class="btn btn-secondary" disabled={stopping} onclick={() => (pending = null)} autofocus>{$t("common.cancel")}</button>
        <button type="button" class="btn btn-destructive" disabled={stopping} onclick={stop}>
          {#if stopping}<span class="spinner" aria-hidden="true"></span>{/if}
          {$t("devtools.ports.confirm_action")}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .pp-port {
    font-weight: 700;
  }

  .pp-command {
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (max-width: 560px) {
    .pp-command {
      display: none;
    }
  }

  .dialog-body p {
    margin: 0 0 var(--space-2);
  }

  .pp-targets {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
    color: var(--text);
  }

  .pp-target-command {
    display: block;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    overflow-wrap: anywhere;
  }
</style>
