<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import { roadmapColumns, roadmapErrorText, type Roadmap } from "$lib/roadmap";

  let roadmap = $state<Roadmap | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let request = 0;

  let columns = $derived(roadmap ? roadmapColumns(roadmap) : []);

  async function load() {
    const mine = ++request;
    loading = true;
    error = null;
    try {
      const got = await invoke<Roadmap>("roadmap_get", {});
      if (mine !== request) return;
      roadmap = got;
    } catch (e) {
      if (mine !== request) return;
      error = roadmapErrorText(e);
    } finally {
      if (mine === request) loading = false;
    }
  }

  function openLink(url: string) {
    open(url).catch(() => {});
  }

  onMount(load);
</script>

<div class="roadmap-page" data-agent-readiness={loading ? "loading" : error ? "error" : "ready"}>
  <header class="head">
    <h1>{$t("roadmap.title")}</h1>
    <p class="desc">{$t("roadmap.desc")}</p>
    {#if roadmap}
      <p class="meta">
        <span>{$t("roadmap.updated", { date: roadmap.updated })}</span>
        <button type="button" class="link" onclick={() => openLink(roadmap!.board)}>{$t("roadmap.open_board")}</button>
      </p>
    {/if}
  </header>

  {#if loading}
    <div class="state" role="status">
      <span class="spinner" aria-hidden="true"></span>
      <span>{$t("common.loading")}</span>
    </div>
  {:else if error}
    <div class="state error" role="alert">
      <span><bdi>{$t("roadmap.failed", { msg: error })}</bdi></span>
      <button type="button" class="btn btn-secondary btn-sm" onclick={load}>{$t("roadmap.retry")}</button>
    </div>
  {:else if columns.length === 0}
    <div class="state">{$t("roadmap.empty")}</div>
  {:else}
    {#each columns as col (col.id)}
      <details class="col" open={col.id !== "done" && col.id !== "wontfix"}>
        <summary>
          <span class="col-name" data-col={col.id}>{$t(`roadmap.col_${col.id}`)}</span>
          <span class="col-count">{col.items.length}</span>
        </summary>
        <ul class="tickets">
          {#each col.items as ticket (ticket.key)}
            <li class="ticket">
              <button type="button" class="ticket-main" onclick={() => openLink(ticket.url)}>
                <span class="key" dir="ltr">{ticket.key}</span>
                <span class="title" dir="ltr" lang="en">{ticket.title}</span>
              </button>
              {#if ticket.issue}
                <button type="button" class="link" onclick={() => openLink(ticket.issue!)}>{$t("roadmap.issue")}</button>
              {/if}
            </li>
          {/each}
        </ul>
      </details>
    {/each}
  {/if}
</div>

<style>
  .roadmap-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .head h1 {
    font-size: var(--text-lg);
    font-weight: 600;
    margin: 0 0 var(--space-1);
  }

  .desc,
  .meta {
    font-size: var(--text-sm);
    color: var(--gray);
    margin: 0;
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-top: var(--space-2);
  }

  .link {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    font-size: var(--text-sm);
    color: var(--accent);
    cursor: pointer;
    white-space: nowrap;
  }

  .link:hover {
    text-decoration: underline;
  }

  .state {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-6) 0;
    font-size: var(--text-sm);
    color: var(--gray);
  }

  .spinner {
    width: 20px;
    height: 20px;
    border: 2px solid var(--input-border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
  }

  .col {
    background: var(--button);
    box-shadow: var(--button-box-shadow);
    border-radius: var(--border-radius);
    padding: var(--space-3) var(--space-4);
  }

  summary {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    cursor: pointer;
    font-weight: 600;
  }

  .col-count {
    font-size: var(--text-sm);
    font-weight: 400;
    color: var(--gray);
  }

  .tickets {
    list-style: none;
    margin: var(--space-3) 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .ticket {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .ticket-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    background: none;
    border: 0;
    border-radius: var(--radius-sm);
    padding: var(--space-2);
    font: inherit;
    color: inherit;
    text-align: start;
    cursor: pointer;
  }

  .ticket-main:hover {
    background: var(--fill-1);
  }

  .key {
    flex: none;
    font-size: var(--text-sm);
    color: var(--gray);
    font-variant-numeric: tabular-nums;
  }

  .title {
    min-width: 0;
    overflow-wrap: anywhere;
  }
</style>
