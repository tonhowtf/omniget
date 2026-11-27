<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { locale, t } from "$lib/i18n";
  import {
    descriptionFor,
    errorKey,
    filterEntries,
    type InstallProgress,
    type MarketEntry,
    type MarketView,
    type PackageSummary,
  } from "$lib/global/connector-market";
  import { platformVisual } from "$lib/global/platform-visuals";
  import SpotlightCard from "./SpotlightCard.svelte";

  type OptionDef = { id: string; label?: string; type?: string; default?: unknown; choices?: string[]; help?: string };
  type Failure = { message: string; action: "install" | "update" };

  let { onOpen, focusId = "" }: { onOpen: (entry: MarketEntry) => void; focusId?: string } = $props();

  let view = $state<MarketView | null>(null);
  let loading = $state(true);
  let progress = $state<Record<string, InstallProgress>>({});
  let failures = $state<Record<string, Failure>>({});
  let confirmRemove = $state<string | null>(null);
  let removing = $state<string | null>(null);
  let optionsFor = $state<string | null>(null);
  let optionDefs = $state<OptionDef[]>([]);
  let optionValues = $state<Record<string, any>>({});
  let filePath = $state<string | null>(null);
  let fileSummary = $state<PackageSummary | null>(null);
  let fileError = $state("");
  let fileBusy = $state(false);

  let query = $state("");
  let entries = $derived(view?.entries ?? []);
  let shown = $derived(filterEntries(entries, query, focusId, (entry) => descriptionFor(entry, $locale)));
  let offline = $derived(!!view && view.source === "cache" && !!view.error);
  let failedEmpty = $derived(!!view && !entries.length && !!view.error);
  let savedAt = $derived(view?.fetched_at ? new Date(view.fetched_at * 1000).toLocaleString() : "");

  async function load(refresh = false) {
    loading = true;
    try {
      view = await invoke<MarketView>("gd_market_list", { refresh });
    } catch (e) {
      view = { entries: [], invalid: [], source: "none", fetched_at: null, error: String(e), rejected: [] };
    } finally {
      loading = false;
    }
  }

  async function install(entry: MarketEntry, action: "install" | "update") {
    const id = entry.id;
    const { [id]: _gone, ...rest } = failures;
    failures = rest;
    confirmRemove = null;
    progress = { ...progress, [id]: { stage: "download", received: 0, total: null } };
    try {
      await invoke(action === "update" ? "gd_market_update" : "gd_market_install", { id });
      await load();
      const fresh = action === "install" ? entries.find((e) => e.id === id) : null;
      if (fresh && fresh.state !== "not_installed") onOpen(fresh);
    } catch (e) {
      failures = { ...failures, [id]: { message: typeof e === "string" ? e : String(e), action } };
    } finally {
      const { [id]: _done, ...left } = progress;
      progress = left;
    }
  }

  async function remove(entry: MarketEntry) {
    removing = entry.id;
    try {
      await invoke("gd_connector_remove", { id: entry.id });
      confirmRemove = null;
      if (optionsFor === entry.id) optionsFor = null;
      await load();
    } catch (e) {
      failures = { ...failures, [entry.id]: { message: String(e), action: "install" } };
    } finally {
      removing = null;
    }
  }

  async function removeInvalid(dir: string) {
    try {
      await invoke("gd_connector_remove_invalid", { dir });
    } finally {
      await load();
    }
  }

  async function toggleOptions(entry: MarketEntry) {
    if (optionsFor === entry.id) {
      optionsFor = null;
      return;
    }
    const r = await invoke<{ defs: OptionDef[]; values: Record<string, any> }>("gd_connector_options", { id: entry.id });
    optionDefs = r.defs;
    optionValues = { ...r.values };
    optionsFor = entry.id;
  }

  async function saveOptions() {
    if (!optionsFor) return;
    await invoke("gd_connector_options_set", { id: optionsFor, values: optionValues });
    optionsFor = null;
  }

  async function pickFile() {
    fileError = "";
    fileSummary = null;
    const path = await openDialog({ filters: [{ name: "OmniGet helper", extensions: ["omniconnector"] }] });
    if (typeof path !== "string") return;
    filePath = path;
    try {
      fileSummary = await invoke<PackageSummary>("gd_connector_inspect_file", { path });
    } catch (e) {
      fileError = typeof e === "string" ? e : String(e);
    }
  }

  async function installFile() {
    if (!filePath) return;
    fileBusy = true;
    try {
      await invoke("gd_connector_install_file", { path: filePath });
      closeFile();
      await load();
    } catch (e) {
      fileError = typeof e === "string" ? e : String(e);
    } finally {
      fileBusy = false;
    }
  }

  function closeFile() {
    filePath = null;
    fileSummary = null;
    fileError = "";
  }

  function permissionLabel(p: string) {
    return p === "cookies" || p === "page_eval" ? $t(`global_dl.market.permission_${p}`) : p;
  }

  function percent(p: InstallProgress) {
    return p.total ? Math.min(100, Math.round((p.received / p.total) * 100)) : null;
  }

  function reveal(node: HTMLElement, on: boolean) {
    if (on) node.scrollIntoView({ block: "center" });
  }

  onMount(() => {
    load();
    const un = listen<any>("global-dl:event", (e) => {
      const ev = e.payload;
      if (ev.kind !== "connector_install" || !progress[ev.id]) return;
      progress = { ...progress, [ev.id]: { stage: ev.stage, received: ev.received ?? 0, total: ev.total ?? null } };
    });
    return () => {
      un.then((f) => f());
    };
  });
</script>

{#snippet detail(raw: string)}
  <details class="details">
    <summary>{$t("global_dl.error_details")}</summary>
    <code>{raw}</code>
  </details>
{/snippet}

<section class="market" id="site-helpers" aria-labelledby="market-title">
  <header class="market-head">
    <div class="market-copy">
      <h2 id="market-title">{$t("global_dl.market.title")}</h2>
      <p>{$t("global_dl.market.subtitle")}</p>
    </div>
    <div class="market-controls">
      {#if entries.length > 4}
        <input
          class="search"
          type="search"
          bind:value={query}
          placeholder={$t("global_dl.market.search")}
          aria-label={$t("global_dl.market.search")}
          spellcheck="false"
          autocomplete="off"
        />
      {/if}
      <button type="button" class="btn btn-ghost btn-sm" onclick={pickFile}>{$t("global_dl.market.from_file")}</button>
      <button type="button" class="btn btn-ghost btn-sm" onclick={() => load(true)} disabled={loading}>{$t("global_dl.market.refresh")}</button>
    </div>
  </header>

  {#if filePath}
    <div class="panel" role="group" aria-label={$t("global_dl.market.file_title")}>
      {#if fileSummary}
        <h3>{$t("global_dl.market.file_title")}</h3>
        <p class="warn-text">{$t("global_dl.market.file_warning")}</p>
        <dl class="facts">
          <dt>{$t("global_dl.market.name")}</dt><dd>{fileSummary.name} {fileSummary.version}</dd>
          <dt>{$t("global_dl.market.author")}</dt><dd>{fileSummary.author || $t("global_dl.market.author_unknown")}</dd>
          <dt>{$t("global_dl.market.domains")}</dt><dd>{fileSummary.domains.join(", ")}</dd>
          {#if fileSummary.permissions.length}
            <dt>{$t("global_dl.market.permissions")}</dt><dd>{fileSummary.permissions.map(permissionLabel).join(" · ")}</dd>
          {/if}
          {#if fileSummary.replaces_version}
            <dt>{$t("global_dl.market.replaces")}</dt><dd>{fileSummary.replaces_version}</dd>
          {/if}
        </dl>
        <div class="row">
          <button type="button" class="btn btn-primary btn-sm" onclick={installFile} disabled={fileBusy}>
            {#if fileBusy}<span class="spinner" aria-hidden="true"></span>{/if}
            {$t("global_dl.market.install_anyway")}
          </button>
          <button type="button" class="btn btn-ghost btn-sm" onclick={closeFile}>{$t("global_dl.close")}</button>
        </div>
      {/if}
      {#if fileError}
        <div class="inline-error" role="alert">
          <p>{$t(errorKey(fileError))}</p>
          {@render detail(fileError)}
          <button type="button" class="btn btn-ghost btn-sm" onclick={closeFile}>{$t("global_dl.close")}</button>
        </div>
      {/if}
    </div>
  {/if}

  {#if offline}
    <p class="banner" role="status">{$t("global_dl.market.offline", { date: savedAt })}</p>
  {:else if view?.error && !failedEmpty}
    <div class="banner" role="alert">
      <p>{$t(errorKey(view.error))}</p>
      {@render detail(view.error)}
    </div>
  {/if}

  {#if loading && !view}
    <ul class="grid" aria-busy="true" aria-label={$t("global_dl.market.loading")}>
      {#each [0, 1, 2, 3] as i (i)}
        <li class="tile-skeleton"></li>
      {/each}
    </ul>
  {:else if failedEmpty}
    <div class="state" role="alert">
      <h3>{$t("global_dl.market.error_title")}</h3>
      <p>{$t(errorKey(view?.error ?? ""))}</p>
      <button type="button" class="btn btn-primary btn-sm" onclick={() => load(true)} disabled={loading}>{$t("global_dl.retry")}</button>
      {@render detail(view?.error ?? "")}
    </div>
  {:else if entries.length === 0}
    <div class="state" role="status">
      <h3>{$t("global_dl.market.empty_title")}</h3>
      <p>{$t("global_dl.market.empty_body")}</p>
    </div>
  {:else}
    {#if shown.length === 0}
      <p class="banner" role="status">{$t("global_dl.market.no_match", { query: query.trim() })}</p>
    {/if}
    <ul class="grid">
      {#each shown as entry (entry.id)}
        {@const running = progress[entry.id]}
        {@const failure = failures[entry.id]}
        {@const installed = entry.state !== "not_installed"}
        {@const description = descriptionFor(entry, $locale)}
        {@const visual = platformVisual(entry.id)}
        <li use:reveal={entry.id === focusId}>
          <SpotlightCard accent={visual.color} highlighted={entry.id === focusId} class="platform-card">
            <div class="tile-banner" style:background={visual.gradient}>
              <div class="platform-emblem" style:--brand={visual.color} aria-hidden="true">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
                  {@html visual.iconPaths}
                </svg>
              </div>
              <div class="tile-status">
                {#if entry.state === "update_available"}
                  <span class="pill pill-update">{$t("global_dl.market.update_badge", { version: entry.version })}</span>
                {:else if entry.dev}
                  <span class="pill">{$t("global_dl.dev")}</span>
                {:else if installed}
                  <span class="pill pill-on">{$t("global_dl.market.installed")}</span>
                {/if}
              </div>
            </div>

            <div class="tile-body">
              <div class="tile-head">
                <h3 title={entry.name}>{entry.name}</h3>
                {#if entry.domains[0]}
                  <span class="domain">{entry.domains[0]}</span>
                {/if}
              </div>

              {#if entry.id === focusId && !installed}
                <p class="wanted">{$t("global_dl.market.wanted")}</p>
              {/if}

              {#if description}
                <p class="desc" title={description}>{description}</p>
              {/if}

              {#if running}
                {@const pct = percent(running)}
                <div class="progress" role="status" aria-live="polite">
                  <progress max="100" value={pct ?? undefined}></progress>
                  <span class="meta">
                    {$t(`global_dl.market.stage_${running.stage === "verify" ? "verify" : "download"}`)}{pct !== null ? ` · ${pct}%` : ""}
                  </span>
                </div>
              {:else if confirmRemove === entry.id}
                <div class="confirm" role="group">
                  <p>{$t("global_dl.market.remove_confirm", { name: entry.name })}</p>
                  <div class="row">
                    <button type="button" class="btn btn-destructive btn-sm" onclick={() => remove(entry)} disabled={removing === entry.id}>
                      {$t("global_dl.remove")}
                    </button>
                    <button type="button" class="btn btn-ghost btn-sm" onclick={() => (confirmRemove = null)}>
                      {$t("global_dl.cancel")}
                    </button>
                  </div>
                </div>
              {:else}
                <div class="actions">
                  {#if !entry.compatible && (!installed || entry.state === "update_available")}
                    <span class="meta">{$t("global_dl.market.needs_version", { version: entry.min_omniget })}</span>
                  {:else if !installed}
                    <button type="button" class="btn btn-primary btn-sm action-fill" onclick={() => install(entry, "install")}>
                      {$t("global_dl.market.install")}
                    </button>
                  {:else}
                    <button type="button" class="btn btn-primary btn-sm action-fill" onclick={() => onOpen(entry)}>
                      {entry.login?.url ? $t("global_dl.market.login_open") : $t("global_dl.market.open")}
                    </button>
                    {#if entry.state === "update_available"}
                      <button type="button" class="btn btn-tinted btn-sm" onclick={() => install(entry, "update")}>
                        {$t("global_dl.market.update")}
                      </button>
                    {/if}
                    {#if entry.has_options}
                      <button
                        type="button"
                        class="btn btn-ghost btn-sm icon-btn"
                        onclick={() => toggleOptions(entry)}
                        aria-expanded={optionsFor === entry.id}
                        aria-label={$t("global_dl.options")}
                        title={$t("global_dl.options")}
                      >
                        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                          <circle cx="8" cy="8" r="2.2" />
                          <path d="M13.2 8a5.2 5.2 0 0 0-.1-.9l1.4-1.1-1.3-2.2-1.7.7a5 5 0 0 0-1.5-.9L9.7 2H7.1l-.3 1.6a5 5 0 0 0-1.5.9l-1.7-.7-1.3 2.2 1.4 1.1a5.2 5.2 0 0 0 0 1.8l-1.4 1.1 1.3 2.2 1.7-.7a5 5 0 0 0 1.5.9l.3 1.6h2.6l.3-1.6a5 5 0 0 0 1.5-.9l1.7.7 1.3-2.2-1.4-1.1c.1-.3.1-.6.1-.9z" />
                        </svg>
                      </button>
                    {/if}
                    <button
                      type="button"
                      class="btn btn-ghost btn-sm icon-btn"
                      onclick={() => { confirmRemove = entry.id; }}
                      aria-label={$t("global_dl.remove")}
                      title={$t("global_dl.remove")}
                    >
                      <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                        <path d="M3 4.5h10M6 4.5V3h4v1.5M5 4.5l.6 8.5h4.8l.6-8.5" />
                      </svg>
                    </button>
                  {/if}
                </div>
              {/if}

              {#if failure && !running}
                <div class="inline-error" role="alert">
                  <p>{$t(errorKey(failure.message))}</p>
                  <button type="button" class="btn btn-secondary btn-sm" onclick={() => install(entry, failure.action)}>
                    {$t("global_dl.retry")}
                  </button>
                  {@render detail(failure.message)}
                </div>
              {/if}

              {#if optionsFor === entry.id}
                <div class="options">
                  {#each optionDefs as o (o.id)}
                    <label class="option">
                      <span>{o.label ?? o.id}</span>
                      {#if o.type === "boolean"}
                        <input type="checkbox" bind:checked={optionValues[o.id]} />
                      {:else if o.type === "select"}
                        <select bind:value={optionValues[o.id]}>
                          {#each o.choices ?? [] as ch (ch)}
                            <option value={ch}>{ch}</option>
                          {/each}
                        </select>
                      {:else}
                        <input type="text" bind:value={optionValues[o.id]} placeholder={o.help ?? ""} />
                      {/if}
                    </label>
                  {/each}
                  <button type="button" class="btn btn-primary btn-sm" onclick={saveOptions}>{$t("global_dl.save")}</button>
                </div>
              {/if}
            </div>
          </SpotlightCard>
        </li>
      {/each}
    </ul>
  {/if}

  {#if view?.invalid?.length}
    <div class="panel" role="alert">
      <h3>{$t("global_dl.market.invalid_title")}</h3>
      <ul class="invalid">
        {#each view.invalid as bad (bad.dir)}
          <li>
            <div class="invalid-copy">
              <strong>{bad.id}</strong>
              <span class="meta">{bad.error}</span>
            </div>
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => removeInvalid(bad.dir)}>{$t("global_dl.remove")}</button>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
</section>

<style>
  .market {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .market-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .market-copy {
    min-width: 180px;
  }

  .market-controls {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  h2 {
    font-size: var(--text-md);
    font-weight: 600;
    margin: 0;
  }

  h3 {
    font-size: var(--text-base);
    font-weight: 600;
    margin: 0;
  }

  .market-copy p {
    margin: 2px 0 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .search {
    width: 190px;
    height: var(--control-h, 28px);
    padding: 0 10px;
    border-radius: var(--radius-sm, 6px);
    border: 1px solid var(--hairline);
    background: var(--surface);
    color: var(--text);
    font-size: var(--text-xs);
  }

  .search:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .grid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
    gap: var(--space-3);
  }

  .tile-skeleton {
    height: 180px;
    border-radius: var(--radius-lg, 12px);
    border: 1px solid var(--hairline);
    background: linear-gradient(90deg, var(--surface) 25%, var(--fill-2) 50%, var(--surface) 75%);
    background-size: 200% 100%;
    animation: shimmer 1.4s ease-in-out infinite;
  }

  @keyframes shimmer {
    from { background-position: 200% 0; }
    to { background-position: -200% 0; }
  }

  .tile-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3);
    border-bottom: 1px solid var(--hairline);
  }

  .platform-emblem {
    width: 40px;
    height: 40px;
    border-radius: 11px;
    display: grid;
    place-items: center;
    color: var(--brand);
    background: color-mix(in srgb, var(--brand) 16%, var(--surface));
    border: 1px solid color-mix(in srgb, var(--brand) 36%, transparent);
    box-shadow: 0 4px 14px color-mix(in srgb, var(--brand) 18%, transparent);
  }

  .platform-emblem svg {
    width: 20px;
    height: 20px;
  }

  .tile-status {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .tile-body {
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex: 1;
  }

  .tile-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }

  .tile-head h3 {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .domain {
    font-size: 11px;
    color: var(--text-dim);
    flex-shrink: 0;
  }

  .desc {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    line-height: 1.45;
    display: -webkit-box;
    line-clamp: 2;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .wanted {
    margin: 0;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--accent-hi, var(--accent));
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: auto;
    padding-top: 4px;
  }

  .action-fill {
    flex: 1;
  }

  .icon-btn {
    width: 28px;
    height: 28px;
    padding: 0;
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }

  .icon-btn svg {
    width: 14px;
    height: 14px;
  }

  .pill {
    font-size: 11px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: var(--radius-full, 9999px);
    background: var(--fill-2);
    color: var(--text-dim);
    white-space: nowrap;
  }

  .pill-on {
    background: color-mix(in srgb, #34c759 22%, var(--surface));
    color: var(--text);
  }

  .pill-update {
    background: color-mix(in srgb, var(--accent) 24%, var(--surface));
    color: var(--text);
  }

  .meta {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .progress {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: auto;
  }

  progress {
    width: 100%;
    height: 6px;
  }

  .confirm {
    margin-top: auto;
    padding: var(--space-2);
    border-radius: 8px;
    background: color-mix(in srgb, #ff453a 10%, var(--surface));
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .confirm p {
    margin: 0;
    font-size: var(--text-xs);
  }

  .options {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-top: var(--space-2);
    border-top: 1px solid var(--hairline);
  }

  .option {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    font-size: var(--text-xs);
  }

  .option input[type="text"],
  .option select {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--hairline);
    border-radius: 6px;
    padding: 3px 8px;
    font-size: var(--text-xs);
  }

  .panel,
  .state {
    background: var(--surface);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-lg, 12px);
    padding: var(--space-3) var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .warn-text {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--warning, #ffd60a);
  }

  .facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px var(--space-3);
    margin: 0;
    font-size: var(--text-xs);
  }

  .facts dt {
    color: var(--text-dim);
  }

  .facts dd {
    margin: 0;
  }

  .banner {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: 8px;
    background: var(--fill-2);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .inline-error {
    padding: var(--space-2);
    border-radius: 8px;
    background: color-mix(in srgb, #ff453a 10%, var(--surface));
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--text-xs);
  }

  .inline-error p {
    margin: 0;
  }

  .details {
    font-size: 11px;
    color: var(--text-dim);
  }

  .details code {
    display: block;
    margin-top: 4px;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .invalid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .invalid li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .invalid-copy {
    display: flex;
    flex-direction: column;
  }
</style>
