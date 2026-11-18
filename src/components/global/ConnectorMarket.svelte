<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { locale, t } from "$lib/i18n";
  import { errorKey, filterEntries, type InstallProgress, type MarketEntry, type MarketView, type PackageSummary } from "$lib/global/connector-market";
  import { localizedConnectorDescription } from "$lib/global/connector-descriptions";

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
  let shown = $derived(filterEntries(entries, query, focusId));
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
    <button class="btn btn-ghost btn-sm" onclick={pickFile}>{$t("global_dl.market.from_file")}</button>
    <button class="btn btn-ghost btn-sm" onclick={() => load(true)} disabled={loading}>{$t("global_dl.market.refresh")}</button>
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
          <button class="btn btn-primary btn-sm" onclick={installFile} disabled={fileBusy}>
            {#if fileBusy}<span class="spinner" aria-hidden="true"></span>{/if}
            {$t("global_dl.market.install_anyway")}
          </button>
          <button class="btn btn-ghost btn-sm" onclick={closeFile}>{$t("global_dl.close")}</button>
        </div>
      {/if}
      {#if fileError}
        <div class="inline-error" role="alert">
          <p>{$t(errorKey(fileError))}</p>
          {@render detail(fileError)}
          <button class="btn btn-ghost btn-sm" onclick={closeFile}>{$t("global_dl.close")}</button>
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
      {#each [0, 1, 2] as i (i)}
        <li class="tile tile-ghost skeleton"></li>
      {/each}
    </ul>
  {:else if failedEmpty}
    <div class="state" role="alert">
      <h3>{$t("global_dl.market.error_title")}</h3>
      <p>{$t(errorKey(view?.error ?? ""))}</p>
      <button class="btn btn-primary btn-sm" onclick={() => load(true)} disabled={loading}>{$t("global_dl.retry")}</button>
      {@render detail(view?.error ?? "")}
    </div>
  {:else if entries.length === 0}
    <div class="state" role="status">
      <h3>{$t("global_dl.market.empty_title")}</h3>
      <p>{$t("global_dl.market.empty_body")}</p>
    </div>
  {:else}
    {#if entries.length > 6}
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
    {#if shown.length === 0}
      <p class="banner" role="status">{$t("global_dl.market.no_match", { query: query.trim() })}</p>
    {/if}
    <ul class="grid">
      {#each shown as entry (entry.id)}
        {@const running = progress[entry.id]}
        {@const failure = failures[entry.id]}
        {@const installed = entry.state !== "not_installed"}
        <li class="tile" class:tile-focus={entry.id === focusId} use:reveal={entry.id === focusId}>
          <div class="tile-top">
            <span class="avatar" aria-hidden="true">{entry.name.slice(0, 1).toUpperCase()}</span>
            <h3 title={entry.name}>{entry.name}</h3>
            {#if entry.state === "update_available"}
              <span class="pill pill-update">{$t("global_dl.market.update_badge", { version: entry.version })}</span>
            {:else if entry.dev}
              <span class="pill">{$t("global_dl.dev")}</span>
            {:else if installed}
              <span class="pill pill-on">{$t("global_dl.market.installed")}</span>
            {/if}
          </div>
          {#if entry.id === focusId && !installed}<p class="wanted">{$t("global_dl.market.wanted")}</p>{/if}
          {#if entry.description}<p class="desc">{localizedConnectorDescription(entry.id, entry.description, $locale)}</p>{/if}
          <p class="meta">
            {#if entry.author}<span>{$t("global_dl.market.by", { author: entry.author })}</span>{/if}
            <span title={entry.domains.join(", ")}>{entry.domains.slice(0, 3).join(", ")}{entry.domains.length > 3 ? ` +${entry.domains.length - 3}` : ""}</span>
          </p>
          {#if entry.permissions.length}
            <p class="meta">{entry.permissions.map(permissionLabel).join(" · ")}</p>
          {/if}

          {#if running}
            {@const pct = percent(running)}
            <div class="progress" role="status" aria-live="polite">
              <progress max="100" value={pct ?? undefined}></progress>
              <span class="meta">{$t(`global_dl.market.stage_${running.stage === "verify" ? "verify" : "download"}`)}{pct !== null ? ` · ${pct}%` : ""}</span>
            </div>
          {:else if confirmRemove === entry.id}
            <div class="confirm" role="group">
              <p>{$t("global_dl.market.remove_confirm", { name: entry.name })}</p>
              <div class="row">
                <button class="btn btn-destructive btn-sm" onclick={() => remove(entry)} disabled={removing === entry.id}>{$t("global_dl.remove")}</button>
                <button class="btn btn-ghost btn-sm" onclick={() => (confirmRemove = null)}>{$t("global_dl.cancel")}</button>
              </div>
            </div>
          {:else}
            <div class="row actions">
              {#if !entry.compatible && (!installed || entry.state === "update_available")}
                <span class="meta">{$t("global_dl.market.needs_version", { version: entry.min_omniget })}</span>
              {:else if !installed}
                <button class="btn btn-primary btn-sm" onclick={() => install(entry, "install")}>{$t("global_dl.market.install")}</button>
              {:else if entry.state === "update_available"}
                <button class="btn btn-tinted btn-sm" onclick={() => install(entry, "update")}>{$t("global_dl.market.update")}</button>
              {/if}
              {#if installed}
                <button class="btn btn-primary btn-sm" onclick={() => onOpen(entry)}>
                  {entry.login?.url ? $t("global_dl.market.login_open") : $t("global_dl.market.open")}
                </button>
                {#if entry.has_options}
                  <button class="btn btn-ghost btn-sm" onclick={() => toggleOptions(entry)} aria-expanded={optionsFor === entry.id}>{$t("global_dl.options")}</button>
                {/if}
                <button class="btn btn-ghost btn-sm" onclick={() => { confirmRemove = entry.id; }}>{$t("global_dl.remove")}</button>
              {/if}
            </div>
          {/if}

          {#if failure && !running}
            <div class="inline-error" role="alert">
              <p>{$t(errorKey(failure.message))}</p>
              <button class="btn btn-secondary btn-sm" onclick={() => install(entry, failure.action)}>{$t("global_dl.retry")}</button>
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
              <button class="btn btn-primary btn-sm" onclick={saveOptions}>{$t("global_dl.save")}</button>
            </div>
          {/if}
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
            <button class="btn btn-ghost btn-sm" onclick={() => removeInvalid(bad.dir)}>{$t("global_dl.remove")}</button>
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
    align-items: center;
    gap: var(--space-2);
  }
  .market-copy {
    flex: 1;
    min-width: 0;
  }
  h2 {
    font-size: var(--text-md);
    margin: 0;
  }
  h3 {
    font-size: var(--text-base);
    margin: 0;
  }
  .market-copy p {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .grid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--space-3);
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--hairline);
    border-radius: 14px;
    min-width: 0;
  }
  .tile-focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .avatar {
    flex: none;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-md);
    background: var(--fill-2);
    font-weight: 600;
    font-size: var(--text-base);
  }
  .wanted {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--accent-hi);
  }
  .search {
    max-width: 320px;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
    padding: 6px 10px;
    font-size: var(--text-sm);
  }
  .tile-ghost {
    height: 148px;
  }
  .tile-top {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .tile-top h3 {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .desc {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    overflow: hidden;
  }
  .meta {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 2px var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .actions,
  .progress,
  .confirm {
    margin-top: auto;
  }
  .progress {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  progress {
    width: 100%;
    height: 6px;
  }
  .confirm p {
    margin: 0 0 var(--space-2);
    font-size: var(--text-sm);
  }
  .pill {
    font-size: var(--text-xs);
    padding: 2px 10px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .pill-on {
    background: color-mix(in srgb, #34c759 22%, transparent);
    color: var(--text);
  }
  .pill-update {
    background: var(--accent-soft);
    color: var(--accent-hi);
  }
  .banner,
  .panel,
  .state {
    margin: 0;
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--hairline);
    border-radius: 14px;
    background: var(--surface);
    font-size: var(--text-sm);
  }
  .banner p,
  .state p {
    margin: 0;
  }
  .state {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    color: var(--text-dim);
  }
  .state h3 {
    color: var(--text);
  }
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .warn-text {
    margin: 0;
    color: var(--text-dim);
  }
  .facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 2px var(--space-3);
    margin: 0;
  }
  .facts dt {
    color: var(--text-dim);
  }
  .facts dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .inline-error {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: 10px;
    background: color-mix(in srgb, #ff3b30 10%, var(--surface));
    font-size: var(--text-sm);
  }
  .inline-error p {
    margin: 0;
  }
  .details {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .details summary {
    cursor: pointer;
  }
  .details code {
    display: block;
    margin-top: var(--space-1);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .options {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--fill-2);
    border-radius: 10px;
  }
  .options .btn {
    align-self: flex-end;
  }
  .option {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    font-size: var(--text-sm);
  }
  .option input[type="text"],
  .option select {
    min-width: 120px;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--hairline);
    border-radius: 8px;
    padding: 4px 8px;
    font-size: var(--text-xs);
  }
  .invalid {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .invalid li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px 0;
  }
  .invalid-copy {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
</style>
