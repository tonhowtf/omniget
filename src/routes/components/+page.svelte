<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import * as api from "$lib/components/api";
  import { customSource, docsUrl, itemUrl, parseItem, parseRegistry } from "$lib/components/registry";
  import { search, typeCounts } from "$lib/components/search";
  import { buildCommand, detectPackageManager, parseWrittenFiles, stripAnsi } from "$lib/components/command";
  import type { ItemSummary, OutputLine, ProjectInfo, RegistrySource } from "$lib/components/types";
  import NavIcon from "#components/shell/NavIcon.svelte";
  import StatusIcon from "#components/converter/StatusIcon.svelte";
  import RegistryManager, { type SourceStatus } from "#components/registry-ui/RegistryManager.svelte";
  import ItemDetail, { type DetailState, type RunState } from "#components/registry-ui/ItemDetail.svelte";
  import TokensPanel from "#components/registry-ui/TokensPanel.svelte";

  type Tab = "browse" | "registries" | "tokens";

  const CUSTOM_KEY = "omniget.components.custom-registries";
  const PROJECT_KEY = "omniget.components.project";
  const PAGE = 120;
  const MANAGER_LABEL = { npm: "npm", pnpm: "pnpm", yarn: "Yarn", bun: "Bun" } as const;

  let tab = $state<Tab>("browse");
  let builtins = $state<RegistrySource[]>([]);
  let custom = $state<RegistrySource[]>([]);
  let sources = $derived([...builtins, ...custom]);
  let statuses = $state<Record<string, SourceStatus>>({});
  let itemsBySource = $state<Record<string, ItemSummary[]>>({});
  let booting = $state(true);
  let bootError = $state<string | null>(null);

  let query = $state("");
  let registryFilter = $state<string | null>(null);
  let typeFilter = $state<string | null>(null);
  let limit = $state(PAGE);
  let selectedKey = $state<string | null>(null);
  let detail = $state<DetailState>({ state: "loading" });
  let detailToken = 0;

  let project = $state<ProjectInfo | null>(null);
  let projectError = $state<string | null>(null);
  let projectBusy = $state(false);
  let overwrite = $state(false);
  let run = $state<RunState | null>(null);
  let runId: string | null = null;

  let allItems = $derived(sources.flatMap((s) => itemsBySource[s.id] ?? []));
  let order = $derived(sources.map((s) => s.id));
  let results = $derived(search(allItems, { query, registryId: registryFilter, type: typeFilter }, order));
  let shown = $derived(results.slice(0, limit));
  let types = $derived(typeCounts(allItems, registryFilter));
  let selected = $derived(allItems.find((i) => i.key === selectedKey) ?? null);
  let selectedSource = $derived(selected ? (sources.find((s) => s.id === selected.registryId) ?? null) : null);
  let anyLoading = $derived(sources.some((s) => statuses[s.id]?.state === "loading"));
  let failedSources = $derived(sources.filter((s) => statuses[s.id]?.state === "error"));
  let offlineSources = $derived(sources.filter((s) => statuses[s.id]?.offline));
  let detection = $derived(detectPackageManager(project?.lockfiles ?? [], !!project?.yarn_classic));
  let selectedUrl = $derived(selected && selectedSource ? itemUrl(selectedSource, selected.name) : null);
  let addCommand = $derived(selectedUrl ? buildCommand(detection.runner, { kind: "add", targets: [selectedUrl], overwrite }) : null);
  let initCommand = $derived(buildCommand(detection.runner, { kind: "init" }));
  let previewUrl = $derived(selected && selectedSource ? docsUrl(selectedSource, selected) : null);

  let liveText = $derived.by(() => {
    if (run?.state === "running") return $t(run.kind === "init" ? "components.init_running" : "components.add_running", { name: run.name }) as string;
    if (booting || anyLoading) return $t("components.loading_registries") as string;
    return $t("components.results", { count: results.length }) as string;
  });

  function saveCustom() {
    try {
      localStorage.setItem(CUSTOM_KEY, JSON.stringify(custom.map((c) => c.index_url)));
    } catch {
      return;
    }
  }

  async function loadSource(source: RegistrySource, force = false) {
    statuses[source.id] = { state: "loading", count: statuses[source.id]?.count ?? 0, fromCache: false, stale: false, offline: false, error: null, detail: null, fetchedAt: null };
    try {
      const res = await api.fetchJson(source.index_url, force);
      const parsed = parseRegistry(res.body, source.id);
      itemsBySource[source.id] = parsed.items;
      if (source.custom && parsed.name) {
        const idx = custom.findIndex((c) => c.id === source.id);
        if (idx >= 0 && custom[idx].name !== parsed.name) custom[idx] = { ...custom[idx], name: parsed.name, homepage: parsed.homepage ?? custom[idx].homepage };
      }
      statuses[source.id] = {
        state: "ready",
        count: parsed.items.length,
        fromCache: res.from_cache,
        stale: res.stale,
        offline: !!res.offline_error,
        error: res.offline_error?.code ?? null,
        detail: res.offline_error?.detail ?? null,
        fetchedAt: res.fetched_at,
      };
    } catch (e) {
      const err = e instanceof Error && e.message === "shape" ? { code: "shape", detail: null } : api.errorCode(e);
      itemsBySource[source.id] = [];
      statuses[source.id] = { state: "error", count: 0, fromCache: false, stale: false, offline: false, error: err.code, detail: err.detail, fetchedAt: null };
    }
  }

  function addSource(source: RegistrySource) {
    custom = [...custom, source];
    saveCustom();
    void loadSource(source);
  }

  function removeSource(id: string) {
    custom = custom.filter((c) => c.id !== id);
    delete itemsBySource[id];
    delete statuses[id];
    if (registryFilter === id) registryFilter = null;
    if (selected?.registryId === id) selectedKey = null;
    saveCustom();
  }

  function reloadSource(id: string) {
    const source = sources.find((s) => s.id === id);
    if (source) void loadSource(source, true);
  }

  async function loadDetail(item: ItemSummary, source: RegistrySource, force = false) {
    const token = ++detailToken;
    detail = { state: "loading" };
    const url = itemUrl(source, item.name);
    if (!url) {
      detail = { state: "error", code: "url_invalid", detail: null };
      return;
    }
    try {
      const res = await api.fetchJson(url, force);
      if (token !== detailToken) return;
      detail = { state: "ready", data: parseItem(res.body), offline: !!res.offline_error };
    } catch (e) {
      if (token !== detailToken) return;
      const err = e instanceof Error && e.message === "shape" ? { code: "shape", detail: null } : api.errorCode(e);
      detail = { state: "error", code: err.code, detail: err.detail };
    }
  }

  function select(item: ItemSummary) {
    if (selectedKey === item.key) return;
    selectedKey = item.key;
    overwrite = false;
    if (run && run.state !== "running") run = null;
    const source = sources.find((s) => s.id === item.registryId);
    if (source) void loadDetail(item, source);
  }

  function onListKey(event: KeyboardEvent) {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const idx = shown.findIndex((i) => i.key === selectedKey);
    const next = event.key === "ArrowDown" ? Math.min(shown.length - 1, idx + 1) : Math.max(0, idx - 1);
    const item = shown[next];
    if (!item) return;
    select(item);
    document.getElementById(`cr-item-${next}`)?.focus();
  }

  async function inspect(path: string) {
    projectBusy = true;
    projectError = null;
    try {
      project = await api.inspectProject(path);
      try {
        localStorage.setItem(PROJECT_KEY, path);
      } catch {
        projectError = null;
      }
    } catch (e) {
      project = null;
      projectError = api.errorCode(e).code;
    } finally {
      projectBusy = false;
    }
  }

  async function chooseProject() {
    const picked = await openDialog({ directory: true, multiple: false }).catch(() => null);
    if (typeof picked === "string") await inspect(picked);
  }

  function newId(): string {
    return typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : `cr-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  }

  async function execute(kind: "add" | "init") {
    if (!project || run?.state === "running") return;
    const cmd = kind === "add" ? addCommand : initCommand;
    if (!cmd) return;
    const id = newId();
    runId = id;
    run = { kind, itemKey: kind === "add" ? selectedKey : null, name: kind === "add" ? (selected?.name ?? "") : "", state: "running", display: cmd.display, lines: [], outcome: null, written: null };
    const current = run;
    try {
      const outcome = await api.run(id, project.path, cmd.program, cmd.args);
      if (outcome.output?.length) current.lines = outcome.output.map((l) => ({ run_id: id, stream: l.stream, line: stripAnsi(l.line) }));
      current.outcome = outcome;
      current.state = outcome.cancelled ? "cancelled" : outcome.ok ? "done" : "failed";
      current.written = parseWrittenFiles(current.lines.map((l) => l.line));
    } catch (e) {
      current.outcome = { ok: false, code: null, cancelled: false, error: api.errorCode(e).code };
      current.state = "failed";
    } finally {
      if (runId === id) runId = null;
      if (project) void inspect(project.path);
    }
  }

  function onOutput(line: OutputLine) {
    if (!run || line.run_id !== runId) return;
    const text = stripAnsi(line.line);
    if (run.lines.length >= 2000) run.lines.shift();
    run.lines.push({ ...line, line: text });
  }

  async function cancelRun() {
    if (runId) await api.cancel(runId).catch(() => {});
  }

  function retry() {
    if (run && run.state !== "running") {
      const kind = run.kind;
      run = null;
      if (detail.state === "error" && selected && selectedSource) void loadDetail(selected, selectedSource, true);
      else void execute(kind);
    } else if (selected && selectedSource) void loadDetail(selected, selectedSource, true);
  }

  function openLink(url: string) {
    if (/^https:\/\//.test(url)) void openExternal(url).catch(() => {});
  }

  function setRegistryFilter(value: string) {
    registryFilter = value || null;
    if (typeFilter && !typeCounts(allItems, registryFilter).some(([ty]) => ty === typeFilter)) typeFilter = null;
    limit = PAGE;
  }

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void api
      .onOutput(onOutput)
      .then((fn) => (disposed ? fn() : (unlisten = fn)))
      .catch(() => {});

    void (async () => {
      try {
        builtins = await api.registries();
      } catch (e) {
        bootError = api.errorCode(e).code;
      }
      let saved: string[] = [];
      try {
        saved = JSON.parse(localStorage.getItem(CUSTOM_KEY) ?? "[]");
      } catch {
        saved = [];
      }
      custom = (Array.isArray(saved) ? saved : []).map((u) => (typeof u === "string" ? customSource(u) : null)).filter((s): s is RegistrySource => !!s);
      booting = false;
      await Promise.all(sources.map((s) => loadSource(s)));
    })();

    let savedProject: string | null = null;
    try {
      savedProject = localStorage.getItem(PROJECT_KEY);
    } catch {
      savedProject = null;
    }
    if (savedProject) void inspect(savedProject).then(() => (projectError = null));

    return () => {
      disposed = true;
      unlisten?.();
      if (runId) void api.cancel(runId).catch(() => {});
    };
  });
</script>

<section class="cr" aria-busy={booting || anyLoading || run?.state === "running"}>
  <header class="cr-head">
    <NavIcon icon="components" size={56} />
    <div class="cr-head-copy">
      <h1>{$t("components.title")}</h1>
      <p class="cr-subtitle">{$t("components.subtitle")}</p>
    </div>
  </header>

  <p class="cr-live" role="status" aria-live="polite">{liveText}</p>

  <div class="cr-project" class:set={!!project}>
    {#if projectBusy}
      <StatusIcon name="running" size={16} />
      <p class="cr-project-copy">{$t("components.project_checking")}</p>
    {:else if project}
      <StatusIcon name={project.has_components_json && !project.components_json_error ? "done" : "warning"} size={16} />
      <div class="cr-project-copy">
        <p class="cr-project-name" title={project.path}>{project.name || project.path}</p>
        <p class="cr-project-meta">
          {#if !project.has_package_json}
            {$t("components.project_no_package")}
          {:else if project.components_json_error}
            {$t("components.project_bad_config")}
          {:else if project.has_components_json}
            {$t("components.project_ready", { manager: MANAGER_LABEL[detection.manager] })}
          {:else}
            {$t("components.project_no_config", { manager: MANAGER_LABEL[detection.manager] })}
          {/if}
        </p>
      </div>
      <button type="button" class="btn btn-ghost btn-sm" disabled={run?.state === "running"} onclick={chooseProject}>{$t("components.change_project")}</button>
    {:else}
      <StatusIcon name="idle" size={16} />
      <div class="cr-project-copy">
        <p class="cr-project-name">{$t("components.project_none")}</p>
        <p class="cr-project-meta">
          {projectError ? $t("components.err.project_missing") : $t("components.project_none_body")}
        </p>
      </div>
      <button type="button" class="btn btn-secondary btn-sm" onclick={chooseProject}>{$t("components.choose_project")}</button>
    {/if}
  </div>

  <div class="mac-segmented cr-tabs" role="tablist" aria-label={$t("components.sections")}>
    {#each [["browse", "components.tab_browse"], ["registries", "components.tab_registries"], ["tokens", "components.tab_tokens"]] as [id, key] (id)}
      <button
        type="button"
        role="tab"
        id="cr-tab-{id}"
        class="mac-segmented-btn"
        class:active={tab === id}
        aria-selected={tab === id}
        aria-controls="cr-panel-{id}"
        onclick={() => (tab = id as Tab)}
      >
        {$t(key)}
        {#if id === "registries" && failedSources.length}<span class="badge badge-neutral">{failedSources.length}</span>{/if}
      </button>
    {/each}
  </div>

  {#if tab === "browse"}
    <div id="cr-panel-browse" role="tabpanel" aria-labelledby="cr-tab-browse" class="cr-panel">
      {#if bootError}
        <div class="cr-notice" role="alert">
          <StatusIcon name="failed" />
          <p>{$t("components.boot_failed")}</p>
        </div>
      {/if}
      {#if !anyLoading && failedSources.length > 0 && failedSources.length < sources.length}
        <div class="cr-notice" role="status">
          <StatusIcon name="warning" />
          <p>{$t("components.partial_failed", { names: failedSources.map((s) => s.name).join(", ") })}</p>
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => (tab = "registries")}>{$t("components.see_registries")}</button>
        </div>
      {/if}
      {#if offlineSources.length > 0}
        <div class="cr-notice" role="status">
          <StatusIcon name="warning" />
          <p>{$t("components.offline_notice", { names: offlineSources.map((s) => s.name).join(", ") })}</p>
        </div>
      {/if}

      <div class="cr-filters">
        <input
          class="input input-search cr-search"
          type="search"
          spellcheck="false"
          placeholder={$t("components.search_placeholder")}
          aria-label={$t("components.search_label")}
          bind:value={query}
          oninput={() => (limit = PAGE)}
        />
        <select class="input cr-select" aria-label={$t("components.filter_registry")} value={registryFilter ?? ""} onchange={(e) => setRegistryFilter((e.currentTarget as HTMLSelectElement).value)}>
          <option value="">{$t("components.all_registries")}</option>
          {#each sources as s (s.id)}
            <option value={s.id}>{s.name}</option>
          {/each}
        </select>
        <select class="input cr-select" aria-label={$t("components.filter_type")} value={typeFilter ?? ""} onchange={(e) => ((typeFilter = (e.currentTarget as HTMLSelectElement).value || null), (limit = PAGE))}>
          <option value="">{$t("components.all_types")}</option>
          {#each types as [ty, count] (ty)}
            <option value={ty}>{ty} ({count})</option>
          {/each}
        </select>
      </div>

      {#if booting || (anyLoading && allItems.length === 0)}
        <div class="cr-empty">
          <span class="spinner" aria-hidden="true"></span>
          <p>{$t("components.loading_registries")}</p>
        </div>
      {:else if allItems.length === 0}
        <div class="cr-empty" role="alert">
          <StatusIcon name="failed" size={24} />
          <h2>{$t("components.all_failed_title")}</h2>
          <p>{$t("components.all_failed_body")}</p>
          <button type="button" class="btn btn-primary" onclick={() => sources.forEach((s) => loadSource(s, true))}>{$t("components.reload_registries")}</button>
        </div>
      {:else}
        <div class="cr-split">
          <div class="cr-list-wrap">
            <p class="cr-count" aria-hidden="true">
              {$t("components.results", { count: results.length })}{#if anyLoading} · {$t("components.registry_loading")}{/if}
            </p>
            {#if results.length === 0}
              <div class="cr-noresult">
                <p>{$t("components.no_results", { query })}</p>
                <button type="button" class="btn btn-ghost btn-sm" onclick={() => ((query = ""), (registryFilter = null), (typeFilter = null))}>{$t("components.clear_filters")}</button>
              </div>
            {:else}
              <ul class="cr-list" role="listbox" aria-label={$t("components.results_label")} tabindex="-1" onkeydown={onListKey}>
                {#each shown as item, i (item.key)}
                  {@const src = sources.find((s) => s.id === item.registryId)}
                  <li
                    id="cr-item-{i}"
                    class="cr-row"
                    class:active={item.key === selectedKey}
                    role="option"
                    aria-selected={item.key === selectedKey}
                    tabindex={item.key === selectedKey || (!selectedKey && i === 0) ? 0 : -1}
                    onclick={() => select(item)}
                    onkeydown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), select(item))}
                  >
                    <div class="cr-row-top">
                      <span class="cr-row-title">{item.title}</span>
                      <span class="cr-row-type">{item.type}</span>
                    </div>
                    {#if item.description}<p class="cr-row-desc">{item.description}</p>{/if}
                    <p class="cr-row-meta">
                      <span>{src?.name ?? item.registryId}</span>
                      <span>· {$t("components.deps_count", { count: item.dependencies.length + item.registryDependencies.length })}</span>
                    </p>
                  </li>
                {/each}
              </ul>
              {#if results.length > shown.length}
                <button type="button" class="btn btn-ghost btn-sm cr-more" onclick={() => (limit += PAGE)}>
                  {$t("components.show_more", { count: Math.min(PAGE, results.length - shown.length) })}
                </button>
              {/if}
            {/if}
          </div>

          <div class="cr-detail-wrap">
            {#if selected}
              <ItemDetail
                item={selected}
                source={selectedSource}
                {detail}
                {project}
                managerLabel={MANAGER_LABEL[detection.manager]}
                ambiguous={detection.ambiguous}
                {addCommand}
                {initCommand}
                {previewUrl}
                {run}
                yarnClassic={detection.manager === "yarn" && detection.runner !== "yarn"}
                bind:overwrite
                onadd={() => execute("add")}
                oninit={() => execute("init")}
                oncancel={cancelRun}
                onchooseproject={chooseProject}
                onopen={openLink}
                onretry={retry}
                ondismiss={() => (run = null)}
              />
            {:else}
              <div class="cr-pick">
                <h2>{$t("components.pick_title")}</h2>
                <p>{$t("components.pick_body")}</p>
              </div>
            {/if}
          </div>
        </div>
      {/if}
    </div>
  {:else if tab === "registries"}
    <div id="cr-panel-registries" role="tabpanel" aria-labelledby="cr-tab-registries" class="cr-panel">
      <p class="cr-panel-intro">{$t("components.registries_intro")}</p>
      <RegistryManager {sources} {statuses} onadd={addSource} onremove={removeSource} onreload={reloadSource} />
    </div>
  {/if}
  <div id="cr-panel-tokens" role="tabpanel" aria-labelledby="cr-tab-tokens" class="cr-panel" hidden={tab !== "tokens"}>
    <TokensPanel {project} onchooseproject={chooseProject} />
  </div>
</section>

<style>
  .cr {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .cr-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .cr-head-copy {
    min-width: 0;
  }

  .cr-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .cr-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .cr-live {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }

  .cr-project {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cr-project-copy {
    flex: 1;
    min-width: 0;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .cr-project-name {
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
  }

  .cr-project-meta {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .cr-tabs {
    align-self: flex-start;
  }

  .cr-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .cr-panel[hidden] {
    display: none;
  }

  .cr-panel-intro {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .cr-notice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cr-notice p {
    flex: 1;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .cr-filters {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .cr-search {
    flex: 1;
    min-width: 220px;
  }

  .cr-select {
    width: auto;
    min-width: 150px;
  }

  .cr-empty,
  .cr-pick {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    min-height: 260px;
    padding: var(--space-6) var(--space-5);
    border-radius: var(--radius-xl);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    text-align: center;
  }

  .cr-empty h2,
  .cr-pick h2 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .cr-empty p,
  .cr-pick p {
    margin: 0;
    max-width: 420px;
    font-size: var(--text-base);
    color: var(--text-dim);
  }

  .cr-split {
    display: grid;
    grid-template-columns: minmax(260px, 340px) minmax(0, 1fr);
    gap: var(--space-4);
    align-items: start;
  }

  .cr-list-wrap {
    position: sticky;
    top: var(--space-2);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .cr-count {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .cr-list {
    max-height: calc(100vh - 300px);
    min-height: 240px;
    margin: 0;
    padding: var(--space-1);
    overflow-y: auto;
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cr-row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    cursor: pointer;
  }

  .cr-row:hover {
    background: var(--fill-2);
  }

  .cr-row.active {
    background: var(--accent-soft);
  }

  .cr-row:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }

  .cr-row-top {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .cr-row-title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
  }

  .cr-row-type {
    flex-shrink: 0;
    font-size: var(--text-caption);
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
    color: var(--text-dim);
  }

  .cr-row-desc {
    display: -webkit-box;
    margin: 0;
    overflow: hidden;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .cr-row-meta {
    display: flex;
    gap: var(--space-1);
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .cr-more {
    align-self: center;
  }

  .cr-noresult {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cr-noresult p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .cr-detail-wrap {
    min-width: 0;
  }

  @media (max-width: 760px) {
    .cr-split {
      grid-template-columns: 1fr;
    }

    .cr-list-wrap {
      position: static;
    }

    .cr-list {
      max-height: 360px;
    }
  }
</style>
