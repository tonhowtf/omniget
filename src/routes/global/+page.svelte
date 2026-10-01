<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import NavIcon from "$components/shell/NavIcon.svelte";

  type Tab = { id: string; url: string; title: string };
  type Sniffed = {
    id: number; tab_id: string; page_title: string; url: string;
    kind: string; mime: string; size: number | null;
  };
  type TreeNode = {
    id: string; title: string; kind: string; drm?: boolean; note?: string;
    children?: TreeNode[]; source?: { format: string; url?: string };
  };
  type Resolved = { connector: string | null; tree: TreeNode; logs: string[]; items: number; drm: number; ms: number };
  type JobItem = { id: string; title: string; status: string; bytes: number; error?: string };
  type Job = {
    id: number; title: string; out_dir: string; status: string; total: number;
    done: number; skipped: number; drm: number; failed: number; handed_off: number;
    bytes: number; items: JobItem[];
  };
  type Connector = { manifest: { id: string; name: string; version: string; domains: string[] }; dir: string; dev: boolean };

  let browserOpen = $state(false);
  let browserState = $state("");
  let browserExe = $state("");
  let installed = $state<{ name: string; path: string }[]>([]);
  let tabs = $state<Tab[]>([]);
  let sniffed = $state<Sniffed[]>([]);
  let jobs = $state<Job[]>([]);
  let connectors = $state<Connector[]>([]);
  let outDir = $state("");
  let urlInput = $state("");
  let busy = $state("");
  let error = $state("");

  let resolved = $state<Resolved | null>(null);
  let resolvedUrl = $state("");
  let selected = $state<Set<string>>(new Set());
  let collapsed = $state<Set<string>>(new Set());
  let maxHeight = $state<number | null>(null);
  let showLogs = $state(false);
  let sniffTab = $state<string | null>(null);

  type OptionDef = { id: string; label?: string; type?: string; default?: unknown; choices?: string[]; help?: string };
  let optionsFor = $state<string | null>(null);
  let optionDefs = $state<OptionDef[]>([]);
  let optionValues = $state<Record<string, any>>({});

  let visibleSniffed = $derived(sniffTab ? sniffed.filter((s) => s.tab_id === sniffTab) : sniffed);

  async function refresh() {
    const st: any = await invoke("gd_status");
    browserOpen = !!st.browser.open;
    browserExe = st.browser.exe ?? "";
    installed = st.browser.installed ?? [];
    tabs = st.browser.tabs ?? [];
    connectors = st.connectors ?? [];
    jobs = st.jobs ?? [];
    if (!outDir) outDir = st.out_dir ?? "";
    sniffed = await invoke("gd_sniffed", { tabId: null });
  }

  async function run<T>(label: string, fn: () => Promise<T>): Promise<T | undefined> {
    busy = label;
    error = "";
    try {
      return await fn();
    } catch (e) {
      error = String(e);
    } finally {
      busy = "";
    }
  }

  function normalizeUrl(u: string) {
    const v = u.trim();
    if (!v) return "";
    return /^[a-z]+:\/\//i.test(v) || v.startsWith("about:") ? v : `https://${v}`;
  }

  async function startBrowser() {
    await run("browser", async () => {
      await invoke("gd_browser_start", { url: normalizeUrl(urlInput) || null });
      urlInput = "";
      await refresh();
    });
  }

  async function stopBrowser() {
    await run("browser", async () => {
      await invoke("gd_browser_stop");
      await refresh();
    });
  }

  async function openUrl() {
    const url = normalizeUrl(urlInput);
    if (!url) return;
    if (!browserOpen) return startBrowser();
    await run("tab", async () => {
      await invoke("gd_tab", { action: "open", tabId: null, url });
      urlInput = "";
    });
  }

  async function tabAction(action: string, tab: Tab) {
    await run("tab", () => invoke("gd_tab", { action, tabId: tab.id, url: null }));
  }

  async function resolveTab(tab: Tab) {
    await resolveUrl(tab.url, tab.id);
  }

  async function resolveUrl(url: string, tabId: string | null = null) {
    const r = await run("resolve", () => invoke<Resolved>("gd_resolve", { url, tabId }));
    if (!r) return;
    resolved = r;
    resolvedUrl = url;
    const all = new Set<string>();
    const walk = (n: TreeNode) => {
      if (!n.children?.length && n.source && !n.drm) all.add(n.id);
      n.children?.forEach(walk);
    };
    walk(r.tree);
    selected = all;
    collapsed = new Set();
    showLogs = false;
  }

  function leafIds(n: TreeNode): string[] {
    if (!n.children?.length) return n.source && !n.drm ? [n.id] : [];
    return n.children.flatMap(leafIds);
  }

  function checkState(n: TreeNode): "all" | "some" | "none" {
    const ids = leafIds(n);
    if (!ids.length) return "none";
    const c = ids.filter((i) => selected.has(i)).length;
    return c === 0 ? "none" : c === ids.length ? "all" : "some";
  }

  function toggle(n: TreeNode) {
    const ids = leafIds(n);
    const next = new Set(selected);
    const on = checkState(n) !== "all";
    for (const i of ids) (on ? next.add(i) : next.delete(i));
    selected = next;
  }

  function toggleCollapse(n: TreeNode) {
    const next = new Set(collapsed);
    next.has(n.id) ? next.delete(n.id) : next.add(n.id);
    collapsed = next;
  }

  async function pickOutDir() {
    const dir = await openDialog({ directory: true, defaultPath: outDir || undefined });
    if (typeof dir === "string") outDir = dir;
  }

  async function downloadTree() {
    if (!resolved) return;
    await run("download", async () => {
      await invoke("gd_download", {
        tree: resolved!.tree,
        selected: [...selected],
        outDir,
        maxHeight,
      });
      await refresh();
    });
  }

  async function downloadSniffed(s: Sniffed) {
    await run("download", async () => {
      await invoke("gd_download_sniffed", { id: s.id, outDir });
      await refresh();
    });
  }

  async function addConnector() {
    const dir = await openDialog({ directory: true });
    if (typeof dir !== "string") return;
    await run("connector", async () => {
      await invoke("gd_connector_install_dev", { dir });
      await refresh();
    });
  }

  async function toggleOptions(c: Connector) {
    if (optionsFor === c.manifest.id) {
      optionsFor = null;
      return;
    }
    const r = await run("connector", () => invoke<{ defs: OptionDef[]; values: Record<string, any> }>("gd_connector_options", { id: c.manifest.id }));
    if (!r) return;
    optionDefs = r.defs;
    optionValues = { ...r.values };
    optionsFor = c.manifest.id;
  }

  async function saveOptions() {
    if (!optionsFor) return;
    const id = optionsFor;
    const r = await run("connector", () => invoke<{ values: Record<string, any> }>("gd_connector_options_set", { id, values: optionValues }));
    if (r) optionsFor = null;
  }

  async function removeConnector(c: Connector) {
    await run("connector", async () => {
      await invoke("gd_connector_remove", { id: c.manifest.id });
      await refresh();
    });
  }

  function bytes(n: number | null | undefined) {
    if (!n) return "";
    const u = ["B", "KB", "MB", "GB", "TB"];
    let i = 0;
    let v = n;
    while (v >= 1024 && i < u.length - 1) {
      v /= 1024;
      i++;
    }
    return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${u[i]}`;
  }

  function host(u: string) {
    try {
      return new URL(u).host;
    } catch {
      return u;
    }
  }

  function shortUrl(u: string) {
    try {
      const x = new URL(u);
      const last = x.pathname.split("/").filter(Boolean).pop() ?? "";
      return `${x.host}/…/${last}`;
    } catch {
      return u;
    }
  }

  onMount(() => {
    refresh();
    const un = listen<any>("global-dl:event", (e) => {
      const ev = e.payload;
      if (ev.kind === "browser") {
        browserState = ev.state;
        if (ev.state === "open" || ev.state === "closed") refresh();
      } else if (ev.kind === "tabs") {
        tabs = ev.tabs;
        browserOpen = true;
      } else if (ev.kind === "sniffed") {
        sniffed = [...sniffed, ev.item].slice(-600);
        if (ev.item.kind === "hls" || ev.item.kind === "dash") {
          invoke<Sniffed[]>("gd_sniffed", { tabId: null }).then((list) => (sniffed = list));
        }
      } else if (ev.kind === "sniffed_removed") {
        const gone = new Set<number>(ev.ids);
        sniffed = sniffed.filter((s) => !gone.has(s.id));
      } else if (ev.kind === "job") {
        const i = jobs.findIndex((j) => j.id === ev.job.id);
        jobs = i < 0 ? [...jobs, ev.job] : jobs.map((j) => (j.id === ev.job.id ? ev.job : j));
      }
    });
    return () => {
      un.then((f) => f());
    };
  });
</script>

{#snippet treeNode(n: TreeNode, depth: number)}
  {@const folder = !!n.children?.length}
  {@const state = checkState(n)}
  <li class="node" class:drm={n.drm}>
    <div class="node-row" style:padding-left="{depth * 18}px">
      {#if folder}
        <button class="twisty" onclick={() => toggleCollapse(n)} aria-label={$t("global_dl.toggle")}>
          {collapsed.has(n.id) ? "▸" : "▾"}
        </button>
      {:else}
        <span class="twisty"></span>
      {/if}
      <input
        type="checkbox"
        checked={state === "all"}
        indeterminate={state === "some"}
        disabled={n.drm || (!folder && !n.source)}
        onchange={() => toggle(n)}
      />
      <span class="kind kind-{n.kind}">{n.kind}</span>
      <span class="node-title">{n.title}</span>
      {#if n.drm}<span class="badge badge-warn">DRM</span>{/if}
      {#if n.note}<span class="note">{n.note}</span>{/if}
    </div>
    {#if folder && !collapsed.has(n.id)}
      <ul>
        {#each n.children ?? [] as c (c.id)}
          {@render treeNode(c, depth + 1)}
        {/each}
      </ul>
    {/if}
  </li>
{/snippet}

<section class="gd">
  <header class="gd-head">
    <NavIcon icon="global" size={48} />
    <div class="gd-head-copy">
      <h1>{$t("global_dl.title")}</h1>
      <p>{$t("global_dl.subtitle")}</p>
    </div>
    <div class="gd-head-actions">
      <span class="pill" class:pill-on={browserOpen}>
        {browserOpen ? $t("global_dl.browser_open") : browserState === "downloading" ? $t("global_dl.browser_downloading") : $t("global_dl.browser_closed")}
      </span>
      {#if browserOpen}
        <button class="btn btn-secondary" onclick={stopBrowser} disabled={busy === "browser"}>{$t("global_dl.close_browser")}</button>
      {:else}
        <button class="btn btn-primary" onclick={startBrowser} disabled={busy === "browser"}>
          {busy === "browser" ? $t("global_dl.starting") : $t("global_dl.open_browser")}
        </button>
      {/if}
    </div>
  </header>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <form class="urlbar" onsubmit={(e) => { e.preventDefault(); openUrl(); }}>
    <input type="text" bind:value={urlInput} placeholder={$t("global_dl.url_placeholder")} spellcheck="false" autocomplete="off" />
    <button class="btn btn-secondary" type="submit" disabled={!urlInput.trim()}>{$t("global_dl.open_tab")}</button>
    <button class="btn btn-secondary" type="button" disabled={!urlInput.trim() || busy === "resolve"} onclick={() => resolveUrl(normalizeUrl(urlInput))}>
      {$t("global_dl.resolve")}
    </button>
  </form>

  {#if !browserOpen && installed.length === 0}
    <p class="hint">{$t("global_dl.no_browser_hint")}</p>
  {:else if !browserOpen}
    <p class="hint">{$t("global_dl.browser_will_use", { name: installed[0]?.name ?? "" })}</p>
  {/if}

  <div class="gd-grid">
    <div class="col">
      <section class="card">
        <header class="card-head">
          <h2>{$t("global_dl.tabs")}</h2>
          <span class="count">{tabs.length}</span>
        </header>
        {#if tabs.length === 0}
          <p class="empty">{$t("global_dl.no_tabs")}</p>
        {:else}
          <ul class="tabs">
            {#each tabs as tab (tab.id)}
              <li class="tab">
                <div class="tab-copy">
                  <span class="tab-title">{tab.title || host(tab.url)}</span>
                  <span class="tab-url">{tab.url}</span>
                </div>
                <div class="tab-actions">
                  <button class="btn btn-primary btn-sm" onclick={() => resolveTab(tab)} disabled={busy === "resolve" || tab.url.startsWith("about:") || tab.url.startsWith("chrome")}>
                    {busy === "resolve" ? "…" : $t("global_dl.resolve")}
                  </button>
                  <button class="btn btn-ghost btn-sm" onclick={() => (sniffTab = sniffTab === tab.id ? null : tab.id)}>
                    {$t("global_dl.media")} {sniffed.filter((s) => s.tab_id === tab.id).length}
                  </button>
                  <button class="btn btn-ghost btn-sm" onclick={() => tabAction("focus", tab)} aria-label={$t("global_dl.focus")}>↗</button>
                  <button class="btn btn-ghost btn-sm" onclick={() => tabAction("close", tab)} aria-label={$t("global_dl.close_tab")}>✕</button>
                </div>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <section class="card">
        <header class="card-head">
          <h2>{$t("global_dl.sniffed")}</h2>
          {#if sniffTab}
            <button class="btn btn-ghost btn-sm" onclick={() => (sniffTab = null)}>{$t("global_dl.all_tabs")}</button>
          {/if}
          <button class="btn btn-ghost btn-sm" onclick={async () => { await invoke("gd_sniffed_clear", { tabId: sniffTab }); sniffed = await invoke("gd_sniffed", { tabId: null }); }}>
            {$t("global_dl.clear")}
          </button>
        </header>
        {#if visibleSniffed.length === 0}
          <p class="empty">{$t("global_dl.no_media")}</p>
        {:else}
          <ul class="sniffed">
            {#each [...visibleSniffed].reverse() as s (s.id)}
              <li>
                <span class="kind kind-{s.kind}">{s.kind}</span>
                <span class="sniff-url" title={s.url}>{shortUrl(s.url)}</span>
                <span class="size">{bytes(s.size)}</span>
                <button class="btn btn-secondary btn-sm" onclick={() => downloadSniffed(s)}>{$t("global_dl.download")}</button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <section class="card">
        <header class="card-head">
          <h2>{$t("global_dl.connectors")}</h2>
          <button class="btn btn-ghost btn-sm" onclick={addConnector}>{$t("global_dl.add_connector")}</button>
        </header>
        {#if connectors.length === 0}
          <p class="empty">{$t("global_dl.no_connectors")}</p>
        {:else}
          <ul class="connectors">
            {#each connectors as c (c.manifest.id)}
              <li>
                <div class="tab-copy">
                  <span class="tab-title">{c.manifest.name} <span class="note">{c.manifest.version}{c.dev ? " · dev" : ""}</span></span>
                  <span class="tab-url">{c.manifest.domains.join(", ")}</span>
                </div>
                {#if (c.manifest as any).options?.length}
                  <button class="btn btn-ghost btn-sm" onclick={() => toggleOptions(c)}>{$t("global_dl.options")}</button>
                {/if}
                <button class="btn btn-ghost btn-sm" onclick={() => removeConnector(c)}>{$t("global_dl.remove")}</button>
              </li>
              {#if optionsFor === c.manifest.id}
                <li class="options">
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
                </li>
              {/if}
            {/each}
          </ul>
        {/if}
      </section>
    </div>

    <div class="col">
      <section class="card card-tree">
        {#if !resolved}
          <p class="empty">{$t("global_dl.no_tree")}</p>
        {:else}
          <header class="card-head">
            <div class="tree-head">
              <h2>{resolved.tree.title}</h2>
              <span class="note">
                {resolved.connector ? $t("global_dl.via_connector", { name: resolved.connector }) : $t("global_dl.via_sniffer")}
                · {$t("global_dl.items_count", { count: resolved.items })}
                {#if resolved.drm} · {$t("global_dl.drm_count", { count: resolved.drm })}{/if}
                · {resolved.ms} ms
              </span>
            </div>
            {#if resolved.logs.length}
              <button class="btn btn-ghost btn-sm" onclick={() => (showLogs = !showLogs)}>{$t("global_dl.logs")}</button>
            {/if}
          </header>
          {#if showLogs}
            <pre class="logs">{resolved.logs.join("\n")}</pre>
          {/if}
          <ul class="tree">
            {#each resolved.tree.children ?? [] as c (c.id)}
              {@render treeNode(c, 0)}
            {/each}
          </ul>
          <footer class="tree-foot">
            <button class="btn btn-ghost btn-sm" onclick={pickOutDir} title={outDir}>{$t("global_dl.folder")}: {outDir.split(/[\\/]/).pop()}</button>
            <select bind:value={maxHeight}>
              <option value={null}>{$t("global_dl.best_quality")}</option>
              <option value={1080}>1080p</option>
              <option value={720}>720p</option>
              <option value={480}>480p</option>
            </select>
            <button class="btn btn-primary" onclick={downloadTree} disabled={selected.size === 0 || busy === "download"}>
              {$t("global_dl.download_selected", { count: selected.size })}
            </button>
          </footer>
        {/if}
      </section>

      {#if jobs.length}
        <section class="card">
          <header class="card-head">
            <h2>{$t("global_dl.jobs")}</h2>
            <button class="btn btn-ghost btn-sm" onclick={async () => { await invoke("gd_jobs_clear"); await refresh(); }}>{$t("global_dl.clear_finished")}</button>
          </header>
          <ul class="jobs">
            {#each [...jobs].reverse() as j (j.id)}
              {@const finished = j.done + j.skipped + j.drm + j.failed + j.handed_off}
              <li class="job">
                <div class="job-row">
                  <span class="tab-title">{j.title}</span>
                  <span class="note">{finished}/{j.total} · {bytes(j.bytes)}</span>
                  {#if j.status === "running"}
                    <button class="btn btn-ghost btn-sm" onclick={() => invoke("gd_job_cancel", { id: j.id })}>{$t("global_dl.cancel")}</button>
                  {:else}
                    <span class="pill">{j.status}</span>
                  {/if}
                </div>
                <progress max={j.total} value={finished}></progress>
                <div class="note">
                  {$t("global_dl.job_summary", { done: j.done, skipped: j.skipped, drm: j.drm, failed: j.failed })}
                </div>
                {#each j.items.filter((i) => i.status === "failed") as f (f.id)}
                  <div class="fail">{f.title}: {f.error}</div>
                {/each}
                {#each j.items.filter((i) => i.status === "active") as a (a.id)}
                  <div class="note">↓ {a.title} {bytes(a.bytes)}</div>
                {/each}
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    </div>
  </div>
</section>

<style>
  .gd {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-5);
    max-width: 1400px;
  }
  .gd-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }
  .gd-head-copy {
    flex: 1;
    min-width: 0;
  }
  .gd-head h1 {
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    letter-spacing: var(--track-tight);
    margin: 0;
  }
  .gd-head p {
    margin: var(--space-1) 0 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .gd-head-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .pill {
    font-size: var(--text-xs);
    padding: 2px 10px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-dim);
  }
  .pill-on {
    background: color-mix(in srgb, #34c759 22%, transparent);
    color: var(--text);
  }
  .error {
    margin: 0;
    padding: var(--space-3);
    border-radius: 10px;
    background: color-mix(in srgb, #ff3b30 14%, transparent);
    font-size: var(--text-sm);
    white-space: pre-wrap;
  }
  .hint {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .urlbar {
    display: flex;
    gap: var(--space-2);
  }
  .urlbar input {
    flex: 1;
    min-width: 0;
    padding: 8px 12px;
    border-radius: 10px;
    border: 1px solid var(--hairline);
    background: var(--surface);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .gd-grid {
    display: grid;
    grid-template-columns: minmax(300px, 5fr) minmax(360px, 7fr);
    gap: var(--space-4);
    align-items: start;
  }
  @media (max-width: 900px) {
    .gd-grid {
      grid-template-columns: 1fr;
    }
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--hairline);
    border-radius: 14px;
    padding: var(--space-3) var(--space-4);
    min-width: 0;
  }
  .card-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }
  .card-head h2 {
    font-size: var(--text-base);
    margin: 0;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count,
  .note,
  .size {
    color: var(--text-dim);
    font-size: var(--text-xs);
  }
  .empty {
    color: var(--text-muted);
    font-size: var(--text-sm);
    margin: var(--space-2) 0;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .tabs li,
  .connectors li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 0;
    border-top: 1px solid var(--hairline);
  }
  .tabs li:first-child,
  .connectors li:first-child {
    border-top: none;
  }
  .tab-copy {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .tab-title {
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tab-url {
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tab-actions {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }
  .sniffed {
    max-height: 320px;
    overflow: auto;
  }
  .sniffed li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px 0;
    font-size: var(--text-xs);
  }
  .sniff-url {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: ui-monospace, monospace;
  }
  .kind {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 6px;
    border-radius: 6px;
    background: var(--fill-2);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .kind-hls,
  .kind-video {
    background: color-mix(in srgb, #0a84ff 18%, transparent);
  }
  .kind-subtitle {
    background: color-mix(in srgb, #bf5af2 18%, transparent);
  }
  .kind-file,
  .kind-document {
    background: color-mix(in srgb, #ff9f0a 18%, transparent);
  }
  .card-tree {
    max-height: 70vh;
    display: flex;
    flex-direction: column;
  }
  .tree-head {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .tree {
    overflow: auto;
    flex: 1;
    min-height: 0;
  }
  .node-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 0;
    font-size: var(--text-sm);
  }
  .node.drm > .node-row {
    opacity: 0.55;
  }
  .twisty {
    width: 16px;
    flex-shrink: 0;
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
  }
  .node-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .badge-warn {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 6px;
    background: color-mix(in srgb, #ff453a 22%, transparent);
  }
  .tree-foot {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding-top: var(--space-3);
    border-top: 1px solid var(--hairline);
    margin-top: var(--space-2);
  }
  .tree-foot .btn-primary {
    margin-left: auto;
  }
  .tree-foot select {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--hairline);
    border-radius: 8px;
    padding: 4px 8px;
    font-size: var(--text-xs);
  }
  .logs {
    max-height: 160px;
    overflow: auto;
    font-size: 11px;
    background: var(--fill-2);
    padding: var(--space-2);
    border-radius: 8px;
    white-space: pre-wrap;
  }
  .job {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: var(--space-2) 0;
    border-top: 1px solid var(--hairline);
  }
  .job:first-child {
    border-top: none;
  }
  .job-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .job-row .tab-title {
    flex: 1;
  }
  progress {
    width: 100%;
    height: 6px;
  }
  .connectors li.options {
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--fill-2);
    border-radius: 10px;
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
    min-width: 140px;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--hairline);
    border-radius: 8px;
    padding: 4px 8px;
    font-size: var(--text-xs);
  }
  .options .btn {
    align-self: flex-end;
  }
  .fail {
    font-size: var(--text-xs);
    color: #ff453a;
  }
</style>
