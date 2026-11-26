<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { ask, open as openDialog } from "@tauri-apps/plugin-dialog";
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { classifyError, type ErrorKind } from "$lib/home/friendly-error";
  import { translateBackendError } from "$lib/error-translate";
  import NavIcon from "#components/shell/NavIcon.svelte";
  import ConnectorMarket from "#components/global/ConnectorMarket.svelte";
  import ConnectorHint from "#components/home/ConnectorHint.svelte";
  import { authProblem, type MarketEntry, type UrlLookup } from "$lib/global/connector-market";

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
  type LoginProfile = { dir: string; name: string; email: string | null };
  type LoginSource = { id: string; name: string; profiles: LoginProfile[]; running: boolean };
  type Imported = { from: string; profile: string } | null;
  type JobOrigin = { tree: TreeNode; outDir: string; maxHeight: number | null };

  const KINDS = new Set(["folder", "video", "audio", "file", "subtitle", "text", "link", "hls", "dash", "document", "image"]);
  const STATUSES = new Set(["queued", "running", "active", "done", "partial", "cancelled", "failed", "skipped", "drm", "handed_off"]);

  let browserOpen = $state(false);
  let browserMode = $state("embedded");
  let browserState = $state("");
  let installed = $state<{ name: string; path: string }[]>([]);
  let tabs = $state<Tab[]>([]);
  let sniffed = $state<Sniffed[]>([]);
  let jobs = $state<Job[]>([]);
  let connectors = $state<Connector[]>([]);
  let outDir = $state("");
  let urlInput = $state("");
  let urlField = $state<HTMLInputElement | null>(null);
  let advancedUrl = $state("");
  let busy = $state("");
  let errorRaw = $state("");
  let retryLast = $state<(() => Promise<void>) | null>(null);

  let resolved = $state<Resolved | null>(null);
  let resolvedUrl = $state("");
  let selected = $state<Set<string>>(new Set());
  let collapsed = $state<Set<string>>(new Set());
  let maxHeight = $state<number | null>(null);
  let showLogs = $state(false);
  let sniffTab = $state<string | null>(null);
  let loginOpen = $state(false);
  let importOpen = $state(false);
  let importSources = $state<LoginSource[]>([]);
  let importSource = $state("");
  let importProfile = $state("");
  let imported = $state<Imported>(null);
  let importResult = $state<Imported>(null);
  let loginNote = $state("");
  let jobOrigins = $state<Record<number, JobOrigin>>({});
  let handledParam = "";

  let marketKey = $state(0);
  let focusHelper = $derived(page.url.searchParams.get("helper") ?? "");

  let visibleSniffed = $derived(sniffTab ? sniffed.filter((s) => s.tab_id === sniffTab) : sniffed);
  let windowVisible = $derived(browserOpen && browserMode === "window");
  let errorKind = $derived<ErrorKind>(errorRaw ? classifyError(errorRaw) : "generic");
  let errorText = $derived(errorRaw ? translateBackendError(errorRaw, (k) => $t(k) as string) : "");
  let downloadable = $derived(resolved ? leafIds(resolved.tree) : []);
  let onlyWholePage = $derived(
    !!resolved && downloadable.length > 0 && allLeaves(resolved.tree).every((n) => n.source?.format === "ytdlp")
  );
  let currentSource = $derived(importSources.find((s) => s.id === importSource));

  async function refresh() {
    const st: any = await invoke("gd_status");
    browserOpen = !!st.browser.open;
    browserMode = st.browser.mode ?? "embedded";
    installed = st.browser.installed ?? [];
    tabs = st.browser.tabs ?? [];
    connectors = st.connectors ?? [];
    jobs = st.jobs ?? [];
    if (!outDir) outDir = st.out_dir ?? "";
    sniffed = await invoke("gd_sniffed", { tabId: null });
  }

  async function run<T>(label: string, fn: () => Promise<T>, retry?: () => Promise<void>): Promise<T | undefined> {
    busy = label;
    errorRaw = "";
    retryLast = null;
    try {
      return await fn();
    } catch (e) {
      errorRaw = typeof e === "string" ? e : ((e as any)?.message ?? String(e));
      retryLast = retry ?? null;
    } finally {
      busy = "";
    }
  }

  function normalizeUrl(u: string) {
    const v = u.trim();
    if (!v) return "";
    return /^[a-z]+:\/\//i.test(v) || v.startsWith("about:") ? v : `https://${v}`;
  }

  function applyResolved(r: Resolved, url: string) {
    resolved = r;
    resolvedUrl = url;
    selected = new Set(leafIds(r.tree));
    const top = (r.tree.children ?? []).filter((c) => c.children?.length);
    collapsed = new Set(top.length > 1 && leafIds(r.tree).length > 40 ? top.map((c) => c.id) : []);
    showLogs = false;
  }

  async function find(reload = false) {
    const url = normalizeUrl(urlInput);
    if (!url) {
      urlField?.focus();
      return;
    }
    resolved = null;
    const r = await run("find", () => invoke<Resolved>("gd_find", { url, reload }), () => find(reload));
    if (r) applyResolved(r, url);
    await refresh();
    if (!r && errorRaw && authProblem(errorRaw) && !loginOpen) {
      const target = await helperLoginUrl(url);
      if (target) await openLogin(target);
    }
  }

  async function helperLoginUrl(url: string): Promise<string | null> {
    try {
      const hit = await invoke<UrlLookup>("gd_connector_for_url", { url });
      return hit.installed?.login_url ?? null;
    } catch {
      return null;
    }
  }

  async function openLogin(target: string) {
    importOpen = false;
    loginNote = "";
    await showWindow(target);
    if (!errorRaw) loginOpen = true;
  }

  async function startBrowser() {
    await run("browser", async () => {
      await invoke("gd_browser_start", { url: normalizeUrl(advancedUrl) || null });
      advancedUrl = "";
      await refresh();
    });
  }

  async function askWhileJobsRun(act: (force: boolean) => Promise<unknown>) {
    try {
      await act(false);
      return true;
    } catch (e) {
      if (!String(e).startsWith("BROWSER_BUSY")) throw e;
    }
    const yes = await ask($t("global_dl.browser_busy_body") as string, {
      title: $t("global_dl.browser_busy_title") as string,
      kind: "warning",
      okLabel: $t("global_dl.browser_busy_confirm") as string,
      cancelLabel: $t("global_dl.browser_busy_keep") as string,
    });
    if (yes) await act(true);
    return yes;
  }

  async function stopBrowser() {
    await run("browser", async () => {
      if (!(await askWhileJobsRun((force) => invoke("gd_browser_stop", { force })))) return;
      loginOpen = false;
      await refresh();
    });
  }

  async function showWindow(url: string | null) {
    await run("window", async () => {
      await askWhileJobsRun((force) => invoke("gd_browser_show", { url, force }));
      await refresh();
    }, () => showWindow(url));
  }

  async function hideWindow() {
    await run("window", async () => {
      if (!(await askWhileJobsRun((force) => invoke("gd_browser_hide", { force })))) return;
      loginOpen = false;
      await refresh();
    });
  }

  async function loginSite() {
    const url = normalizeUrl(urlInput);
    if (!url) {
      urlField?.focus();
      return;
    }
    await openLogin((await helperLoginUrl(url)) ?? url);
  }

  async function openFromMarket(entry: MarketEntry) {
    const login = entry.login?.url;
    if (login && (await listsFrom(entry.id, login))) {
      urlInput = login;
      await find();
    } else if (login) {
      await openLogin(login);
    } else {
      urlField?.focus();
    }
  }

  async function listsFrom(id: string, url: string): Promise<boolean> {
    try {
      const hit = await invoke<UrlLookup>("gd_connector_for_url", { url });
      return hit.installed?.id === id;
    } catch {
      return false;
    }
  }

  async function matchingTabUrl(): Promise<string | null> {
    await refresh();
    for (const tab of [...tabs].reverse()) {
      if (!/^https?:/i.test(tab.url)) continue;
      try {
        const hit = await invoke<UrlLookup>("gd_connector_for_url", { url: tab.url });
        if (hit.installed) return tab.url;
      } catch {}
    }
    return null;
  }

  async function loggedIn() {
    if (!urlInput.trim()) {
      const url = await matchingTabUrl();
      if (!url) {
        loginNote = $t("global_dl.market.login_paste") as string;
        return;
      }
      urlInput = url;
    }
    loginOpen = false;
    loginNote = "";
    await find(true);
  }

  async function toggleImport() {
    if (importOpen) {
      importOpen = false;
      return;
    }
    loginOpen = false;
    importResult = null;
    const r = await run("import", () => invoke<{ sources: LoginSource[]; imported: Imported }>("gd_browser_sources"));
    if (!r) return;
    importSources = r.sources;
    imported = r.imported && (r.imported as any).from ? r.imported : null;
    importSource = importSources[0]?.id ?? "";
    importProfile = importSources[0]?.profiles[0]?.dir ?? "";
    importOpen = true;
  }

  function pickSource(id: string) {
    importSource = id;
    importProfile = importSources.find((s) => s.id === id)?.profiles[0]?.dir ?? "";
  }

  async function runImport() {
    if (!importSource) return;
    const r = await run(
      "import",
      () => invoke<{ from: string; profile: string }>("gd_browser_import", { source: importSource, profile: importProfile || null }),
      runImport
    );
    if (!r) return;
    importResult = r;
    imported = r;
    await refresh();
  }

  async function openAdvancedUrl() {
    const url = normalizeUrl(advancedUrl);
    if (!url) return;
    if (!browserOpen) return startBrowser();
    await run("tab", async () => {
      await invoke("gd_tab", { action: "open", tabId: null, url });
      advancedUrl = "";
    });
  }

  async function tabAction(action: string, tab: Tab) {
    await run("tab", () => invoke("gd_tab", { action, tabId: tab.id, url: null }));
  }

  async function resolveTab(tab: Tab) {
    urlInput = tab.url;
    resolved = null;
    const r = await run("find", () => invoke<Resolved>("gd_resolve", { url: tab.url, tabId: tab.id }), () => resolveTab(tab));
    if (r) applyResolved(r, tab.url);
  }

  function allLeaves(n: TreeNode): TreeNode[] {
    if (!n.children?.length) return n.source && !n.drm ? [n] : [];
    return n.children.flatMap(allLeaves);
  }

  function leafIds(n: TreeNode): string[] {
    return allLeaves(n).map((x) => x.id);
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

  async function startJob(tree: TreeNode, ids: string[] | null, dir: string, height: number | null) {
    await run("download", async () => {
      const job = await invoke<Job>("gd_download", { tree, selected: ids, outDir: dir, maxHeight: height });
      jobOrigins = { ...jobOrigins, [job.id]: { tree, outDir: dir, maxHeight: height } };
      await refresh();
    }, () => startJob(tree, ids, dir, height));
  }

  async function downloadTree() {
    if (!resolved) return;
    await startJob(resolved.tree, [...selected], outDir, maxHeight);
  }

  async function retryItems(j: Job, ids: string[]) {
    const o = jobOrigins[j.id];
    if (!o || !ids.length) return;
    await startJob(o.tree, ids, o.outDir, o.maxHeight);
  }

  async function downloadSniffed(s: Sniffed) {
    await run("download", async () => {
      await invoke("gd_download_sniffed", { id: s.id, outDir });
      await refresh();
    });
  }

  async function openFolder(path: string) {
    await run("folder", async () => {
      try {
        await invoke("open_path_default", { path });
      } catch {
        await invoke("reveal_file", { path });
      }
    });
  }

  async function addConnector() {
    const dir = await openDialog({ directory: true });
    if (typeof dir !== "string") return;
    await run("connector", async () => {
      await invoke("gd_connector_install_dev", { dir });
      await refresh();
      marketKey++;
    });
  }

  async function removeDevConnector(c: Connector) {
    await run("connector", async () => {
      await invoke("gd_connector_remove", { id: c.manifest.id });
      await refresh();
      marketKey++;
    });
  }

  function connectorName(id: string) {
    return connectors.find((c) => c.manifest.id === id)?.manifest.name ?? id;
  }

  function kindLabel(k: string) {
    return $t(`global_dl.kind.${KINDS.has(k) ? k : "file"}`);
  }

  function statusLabel(s: string) {
    return STATUSES.has(s) ? $t(`global_dl.status.${s}`) : s;
  }

  function nodeTitle(n: TreeNode) {
    return n.source?.format === "ytdlp" ? n.title.replace(/\s*\(yt-dlp\)$/, "") : n.title;
  }

  function itemError(raw: string | undefined) {
    return raw ? translateBackendError(raw, (k) => $t(k) as string) : "";
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

  function shortPath(p: string) {
    const parts = p.split(/[\\/]/).filter(Boolean);
    return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : p;
  }

  $effect(() => {
    const param = page.url.searchParams.get("url");
    if (!param || param === handledParam) return;
    handledParam = param;
    untrack(() => {
      urlInput = param;
      marketKey++;
      find();
    });
  });

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
        <button class="twisty" onclick={() => toggleCollapse(n)} aria-label={$t("global_dl.toggle")} aria-expanded={!collapsed.has(n.id)}>
          {collapsed.has(n.id) ? "▸" : "▾"}
        </button>
      {:else}
        <span class="twisty"></span>
      {/if}
      <label class="node-label">
        <input
          type="checkbox"
          checked={state === "all"}
          indeterminate={state === "some"}
          disabled={n.drm || (!folder && !n.source)}
          onchange={() => toggle(n)}
        />
        <span class="kind kind-{n.kind}">{kindLabel(n.kind)}</span>
        <span class="node-title">{nodeTitle(n)}</span>
      </label>
      {#if n.drm}<span class="badge badge-warn" title={$t("global_dl.drm_tooltip")}>{$t("global_dl.drm")}</span>{/if}
      {#if folder && depth === 0 && resolved && leafIds(n).length}
        <button class="btn btn-secondary btn-sm" onclick={() => startJob(resolved!.tree, leafIds(n), outDir, maxHeight)} disabled={busy === "download"}>
          {$t("global_dl.download")}
        </button>
      {/if}
      {#if n.source?.format === "ytdlp"}
        <span class="note">{$t("global_dl.note_handoff")}</span>
      {:else if n.note}
        <span class="note">{n.note}</span>
      {/if}
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

{#snippet emptyTips()}
  <ul class="tips">
    <li>
      <span>{$t("global_dl.empty_login")}</span>
      <button class="btn btn-secondary btn-sm" onclick={loginSite} disabled={!!busy}>{$t("global_dl.login_site")}</button>
    </li>
    <li>
      <span>{$t("global_dl.empty_play")}</span>
      <button class="btn btn-secondary btn-sm" onclick={() => showWindow(resolvedUrl || normalizeUrl(urlInput))} disabled={!!busy}>{$t("global_dl.show_window")}</button>
    </li>
    <li>
      <span>{$t("global_dl.empty_again")}</span>
      <button class="btn btn-secondary btn-sm" onclick={() => find(true)} disabled={!!busy}>{$t("global_dl.find_again")}</button>
    </li>
  </ul>
{/snippet}

<section class="gd">
  <header class="gd-head">
    <NavIcon icon="global" size={48} />
    <div class="gd-head-copy">
      <h1>{$t("global_dl.title")}</h1>
      <p>{$t("global_dl.subtitle")}</p>
    </div>
    <div class="gd-head-actions">
      {#if browserState === "downloading" && !browserOpen}
        <span class="pill" role="status">{$t("global_dl.browser_downloading")}</span>
      {:else if windowVisible}
        <span class="pill pill-on">{$t("global_dl.browser_window_on")}</span>
        <button class="btn btn-secondary btn-sm" onclick={hideWindow} disabled={busy === "window"}>{$t("global_dl.hide_window")}</button>
      {/if}
    </div>
  </header>

  <form class="hero card" onsubmit={(e) => { e.preventDefault(); find(); }}>
    <label for="gd-url" class="hero-label">{$t("global_dl.hero_label")}</label>
    <div class="hero-row">
      <input
        id="gd-url"
        type="text"
        inputmode="url"
        bind:this={urlField}
        bind:value={urlInput}
        placeholder={$t("global_dl.url_placeholder")}
        spellcheck="false"
        autocomplete="off"
      />
      <button class="btn btn-primary btn-lg" type="submit" disabled={!urlInput.trim() || busy === "find"}>
        {#if busy === "find"}<span class="spinner" aria-hidden="true"></span>{/if}
        {$t("global_dl.find")}
      </button>
    </div>
    <div class="hero-secondary">
      <button class="btn btn-ghost btn-sm" type="button" onclick={loginSite} disabled={!urlInput.trim() || busy === "window"}>
        {$t("global_dl.login_site")}
      </button>
      <button class="btn btn-ghost btn-sm" type="button" onclick={toggleImport} aria-expanded={importOpen} disabled={busy === "import" && !importOpen}>
        {$t("global_dl.import_logins")}
      </button>
    </div>
    {#if !browserOpen && installed.length === 0}
      <p class="hint">{$t("global_dl.no_browser_hint")}</p>
    {/if}
  </form>

  {#if !resolved && busy !== "find"}
    <ConnectorHint url={urlInput} offerOnly />
  {/if}

  {#if loginOpen}
    <section class="card callout" aria-live="polite">
      <h2>{$t("global_dl.login_title")}</h2>
      <p>{$t("global_dl.login_hint")}</p>
      {#if loginNote}<p role="status">{loginNote}</p>{/if}
      <div class="row">
        <button class="btn btn-primary" onclick={loggedIn} disabled={busy === "find"}>{$t("global_dl.login_done")}</button>
        <button class="btn btn-ghost" onclick={hideWindow} disabled={busy === "window"}>{$t("global_dl.hide_window")}</button>
      </div>
    </section>
  {/if}

  {#if importOpen}
    <section class="card callout">
      <h2>{$t("global_dl.import_title")}</h2>
      <p>{$t("global_dl.import_hint")}</p>
      {#if importSources.length === 0}
        <p class="empty">{$t("global_dl.import_none")}</p>
      {:else}
        <div class="row">
          <label class="field">
            <span>{$t("global_dl.import_browser")}</span>
            <select value={importSource} onchange={(e) => pickSource((e.currentTarget as HTMLSelectElement).value)}>
              {#each importSources as s (s.id)}
                <option value={s.id}>{s.name}</option>
              {/each}
            </select>
          </label>
          {#if (currentSource?.profiles.length ?? 0) > 1}
            <label class="field">
              <span>{$t("global_dl.import_profile")}</span>
              <select bind:value={importProfile}>
                {#each currentSource?.profiles ?? [] as p (p.dir)}
                  <option value={p.dir}>{p.email ? `${p.name} (${p.email})` : p.name}</option>
                {/each}
              </select>
            </label>
          {/if}
          <button class="btn btn-primary" onclick={runImport} disabled={busy === "import"}>
            {#if busy === "import"}<span class="spinner" aria-hidden="true"></span>{/if}
            {$t("global_dl.import_button")}
          </button>
          <button class="btn btn-ghost" onclick={() => (importOpen = false)}>{$t("global_dl.close")}</button>
        </div>
        {#if currentSource?.running}
          <p class="hint">{$t("global_dl.import_running", { name: currentSource.name })}</p>
        {/if}
      {/if}
      {#if importResult}
        <p class="ok" role="status">{$t("global_dl.import_done", { from: importResult.from, profile: importResult.profile })}</p>
        {#if urlInput.trim()}
          <button class="btn btn-secondary btn-sm" onclick={() => find(true)}>{$t("global_dl.find_again")}</button>
        {/if}
      {:else if imported}
        <p class="hint">{$t("global_dl.import_last", { from: imported.from, profile: imported.profile })}</p>
      {/if}
    </section>
  {/if}

  {#if errorRaw}
    <section class="card error" role="alert">
      <h2>{$t(`omnibox.err.${errorKind}.title`)}</h2>
      <p>{errorKind === "generic" ? $t("global_dl.error_body") : errorKind === "login" ? $t("global_dl.error_login") : $t(`omnibox.err.${errorKind}.body`)}</p>
      <div class="row">
        {#if retryLast}
          <button class="btn btn-primary btn-sm" onclick={() => retryLast?.()}>{$t("global_dl.retry")}</button>
        {/if}
        {#if errorKind === "login"}
          <button class="btn btn-secondary btn-sm" onclick={loginSite}>{$t("global_dl.login_site")}</button>
        {/if}
        <button class="btn btn-ghost btn-sm" onclick={() => (errorRaw = "")}>{$t("global_dl.close")}</button>
      </div>
      <details class="details">
        <summary>{$t("global_dl.error_details")}</summary>
        <code>{errorText !== errorRaw ? `${errorText}\n${errorRaw}` : errorRaw}</code>
      </details>
    </section>
  {/if}

  {#if busy === "find"}
    <section class="card loading" role="status" aria-live="polite">
      <span class="spinner" aria-hidden="true"></span>
      <span>{$t("global_dl.finding")}</span>
    </section>
  {:else if resolved}
    <section class="card card-tree">
      <header class="card-head">
        <div class="tree-head">
          <h2 title={resolved.tree.title}>{resolved.tree.title}</h2>
          <span class="note">
            {resolved.connector ? $t("global_dl.via_connector", { name: connectorName(resolved.connector) }) : $t("global_dl.via_page")}
            · {$t("global_dl.items_count", { count: downloadable.length })}
            {#if resolved.drm} · <span title={$t("global_dl.drm_tooltip")}>{$t("global_dl.drm_count", { count: resolved.drm })}</span>{/if}
          </span>
        </div>
        {#if downloadable.length > 1}
          <button class="btn btn-ghost btn-sm" onclick={() => (selected = new Set(downloadable))}>{$t("global_dl.select_all")}</button>
          <button class="btn btn-ghost btn-sm" onclick={() => (selected = new Set())}>{$t("global_dl.select_none")}</button>
        {/if}
      </header>
      {#if downloadable.length === 0}
        <h3 class="sub">{$t("global_dl.empty_title")}</h3>
        <p class="hint">{$t("global_dl.empty_try")}</p>
        {@render emptyTips()}
      {:else}
        {#if onlyWholePage}
          <details class="soft">
            <summary>{$t("global_dl.only_page")}</summary>
            {@render emptyTips()}
          </details>
        {/if}
        <ul class="tree">
          {#each resolved.tree.children ?? [] as c (c.id)}
            {@render treeNode(c, 0)}
          {/each}
        </ul>
        <footer class="tree-foot">
          <div class="field-inline">
            <span class="note">{$t("global_dl.folder")}</span>
            <button class="btn btn-ghost btn-sm path" onclick={pickOutDir} title={outDir} aria-label={`${$t("global_dl.folder_change")}: ${outDir}`}>{shortPath(outDir)}</button>
          </div>
          <label class="field-inline">
            <span class="note">{$t("global_dl.quality")}</span>
            <select bind:value={maxHeight}>
              <option value={null}>{$t("global_dl.best_quality")}</option>
              <option value={1080}>1080p</option>
              <option value={720}>720p</option>
              <option value={480}>480p</option>
            </select>
          </label>
          <button class="btn btn-primary" onclick={downloadTree} disabled={selected.size === 0 || busy === "download"}>
            {$t("global_dl.download_selected", { count: selected.size })}
          </button>
        </footer>
      {/if}
      {#if resolved.logs.length || resolved.ms}
        <button class="btn btn-ghost btn-sm tech" onclick={() => (showLogs = !showLogs)} aria-expanded={showLogs}>{$t("global_dl.logs")}</button>
        {#if showLogs}
          <pre class="logs">{[`${resolved.ms} ms`, ...resolved.logs].join("\n")}</pre>
        {/if}
      {/if}
    </section>
  {:else if !jobs.length}
    <section class="card steps">
      <h2>{$t("global_dl.steps_title")}</h2>
      <ol>
        <li><span class="step-n" aria-hidden="true">1</span><span>{$t("global_dl.step_paste")}</span></li>
        <li><span class="step-n" aria-hidden="true">2</span><span>{$t("global_dl.step_login")}</span></li>
        <li><span class="step-n" aria-hidden="true">3</span><span>{$t("global_dl.step_pick")}</span></li>
      </ol>
    </section>
  {/if}

  {#if jobs.length}
    <section class="card">
      <header class="card-head">
        <h2>{$t("global_dl.jobs")}</h2>
        <button class="btn btn-ghost btn-sm" onclick={async () => { await invoke("gd_jobs_clear"); await refresh(); }}>{$t("global_dl.clear_finished")}</button>
      </header>
      <ul class="jobs">
        {#each [...jobs].reverse() as j (j.id)}
          {@const finished = j.done + j.skipped + j.drm + j.failed + j.handed_off}
          {@const failedItems = j.items.filter((i) => i.status === "failed")}
          <li class="job">
            <div class="job-row">
              <span class="tab-title" title={j.title}>{j.title}</span>
              <span class="note">{$t("global_dl.job_progress", { finished, total: j.total })}{j.bytes ? ` · ${bytes(j.bytes)}` : ""}</span>
              <span class="pill" class:pill-on={j.status === "done"} class:pill-fail={j.status === "failed" || j.status === "partial"}>{statusLabel(j.status)}</span>
              {#if j.status === "running"}
                <button class="btn btn-ghost btn-sm" onclick={() => invoke("gd_job_cancel", { id: j.id })}>{$t("global_dl.cancel")}</button>
              {/if}
            </div>
            <progress max={j.total} value={finished} aria-label={j.title}></progress>
            <div class="note">
              {$t("global_dl.job_summary", { done: j.done, skipped: j.skipped, handed: j.handed_off, drm: j.drm, failed: j.failed })}
            </div>
            {#each j.items.filter((i) => i.status === "active") as a (a.id)}
              <div class="note">↓ {a.title} {bytes(a.bytes)}</div>
            {/each}
            {#each failedItems as f (f.id)}
              <div class="fail">
                <span><strong>{f.title}</strong>: {itemError(f.error)}</span>
                {#if jobOrigins[j.id] && j.status !== "running"}
                  <button class="btn btn-ghost btn-sm" onclick={() => retryItems(j, [f.id])}>{$t("global_dl.retry")}</button>
                {/if}
              </div>
            {/each}
            {#if j.status !== "running"}
              <div class="row">
                {#if j.done + j.skipped > 0}
                  <button class="btn btn-secondary btn-sm" onclick={() => openFolder(j.out_dir)} title={j.out_dir}>{$t("global_dl.open_folder")}</button>
                {/if}
                {#if j.handed_off > 0}
                  <a class="btn btn-secondary btn-sm" href="/downloads">{$t("global_dl.see_in_downloads")}</a>
                {/if}
                {#if failedItems.length > 1 && jobOrigins[j.id]}
                  <button class="btn btn-ghost btn-sm" onclick={() => retryItems(j, failedItems.map((f) => f.id))}>{$t("global_dl.retry_failed")}</button>
                {/if}
              </div>
              {#if j.handed_off > 0}
                <p class="note">{$t("global_dl.handed_note", { count: j.handed_off })}</p>
              {/if}
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  {#key marketKey}
    <ConnectorMarket onOpen={openFromMarket} focusId={focusHelper} />
  {/key}

  <details class="advanced card">
    <summary>
      <span class="adv-title">{$t("global_dl.advanced")}</span>
      <span class="note">{$t("global_dl.advanced_hint")}</span>
    </summary>

    <div class="adv-body">
      <div class="row">
        <span class="pill" class:pill-on={browserOpen}>
          {browserOpen ? $t("global_dl.browser_open") : $t("global_dl.browser_closed")}
        </span>
        {#if browserOpen}
          <button class="btn btn-secondary btn-sm" onclick={stopBrowser} disabled={busy === "browser"}>{$t("global_dl.close_browser")}</button>
        {:else}
          <button class="btn btn-secondary btn-sm" onclick={startBrowser} disabled={busy === "browser"}>
            {busy === "browser" ? $t("global_dl.starting") : $t("global_dl.open_browser")}
          </button>
        {/if}
        {#if windowVisible}
          <button class="btn btn-ghost btn-sm" onclick={hideWindow} disabled={busy === "window"}>{$t("global_dl.hide_window")}</button>
        {:else}
          <button class="btn btn-ghost btn-sm" onclick={() => showWindow(null)} disabled={busy === "window"}>{$t("global_dl.show_window")}</button>
        {/if}
      </div>
      {#if !browserOpen && installed.length > 0}
        <p class="hint">{$t("global_dl.browser_will_use", { name: installed[0]?.name ?? "" })}</p>
      {/if}

      <form class="urlbar" onsubmit={(e) => { e.preventDefault(); openAdvancedUrl(); }}>
        <label class="sr-only" for="gd-adv-url">{$t("global_dl.open_tab")}</label>
        <input id="gd-adv-url" type="text" bind:value={advancedUrl} placeholder={$t("global_dl.adv_url_placeholder")} spellcheck="false" autocomplete="off" />
        <button class="btn btn-secondary" type="submit" disabled={!advancedUrl.trim()}>{$t("global_dl.open_tab")}</button>
      </form>

      <div class="gd-grid">
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
                    <span class="tab-url" title={tab.url}>{tab.url}</span>
                  </div>
                  <div class="tab-actions">
                    <button class="btn btn-secondary btn-sm" onclick={() => resolveTab(tab)} disabled={busy === "find" || tab.url.startsWith("about:") || tab.url.startsWith("chrome")}>
                      {$t("global_dl.find")}
                    </button>
                    <button class="btn btn-ghost btn-sm" onclick={() => (sniffTab = sniffTab === tab.id ? null : tab.id)} aria-pressed={sniffTab === tab.id}>
                      {$t("global_dl.media")} {sniffed.filter((s) => s.tab_id === tab.id).length}
                    </button>
                    <button class="btn btn-ghost btn-sm" onclick={() => tabAction("focus", tab)} aria-label={$t("global_dl.focus")} title={$t("global_dl.focus")}>↗</button>
                    <button class="btn btn-ghost btn-sm" onclick={() => tabAction("close", tab)} aria-label={$t("global_dl.close_tab")} title={$t("global_dl.close_tab")}>✕</button>
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
                  <span class="kind kind-{s.kind}">{kindLabel(s.kind)}</span>
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
          <p class="note">{$t("global_dl.connectors_hint")}</p>
          {#if connectors.some((c) => c.dev)}
            <ul class="connectors">
              {#each connectors.filter((c) => c.dev) as c (c.manifest.id)}
                <li>
                  <div class="tab-copy">
                    <span class="tab-title">{c.manifest.name} <span class="note">{c.manifest.version} · {$t("global_dl.dev")}</span></span>
                    <span class="tab-url" title={c.dir}>{c.dir}</span>
                  </div>
                  <button class="btn btn-ghost btn-sm" onclick={() => removeDevConnector(c)}>{$t("global_dl.remove")}</button>
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      </div>
    </div>
  </details>
</section>

<style>
  .gd {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-5);
    max-width: 1100px;
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
    white-space: nowrap;
  }
  .pill-fail {
    color: var(--danger);
  }

  .pill-on {
    background: color-mix(in srgb, #34c759 22%, transparent);
    color: var(--text);
  }
  .hero {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5);
  }
  .hero-label {
    font-size: var(--text-md);
    font-weight: 600;
  }
  .hero-row {
    display: flex;
    gap: var(--space-2);
  }
  .hero-row input {
    flex: 1;
    min-width: 0;
    height: var(--control-h-lg);
    padding: 0 14px;
    border-radius: var(--radius-md);
    border: 1px solid var(--hairline);
    background: var(--surface);
    color: var(--text);
    font-size: var(--text-md);
  }
  .hero-row input:focus-visible,
  .urlbar input:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .hero-secondary {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .callout h2,
  .error h2,
  .steps h2 {
    font-size: var(--text-base);
    margin: 0 0 var(--space-1);
  }
  .callout p,
  .error p {
    margin: 0 0 var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .field-inline {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 0;
  }
  select {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--hairline);
    border-radius: 8px;
    padding: 4px 8px;
    font-size: var(--text-sm);
  }
  .ok {
    margin: var(--space-3) 0 var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
  }
  .error {
    background: color-mix(in srgb, #ff3b30 10%, var(--surface));
  }
  .error p {
    color: var(--text);
  }
  .details {
    margin-top: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .details code {
    display: block;
    margin-top: var(--space-1);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .loading {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    font-size: var(--text-sm);
    padding: var(--space-4);
  }
  .steps ol {
    list-style: none;
    margin: var(--space-2) 0 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--space-3);
  }
  @media (max-width: 760px) {
    .steps ol {
      grid-template-columns: 1fr;
    }
  }
  .steps li {
    display: flex;
    gap: var(--space-2);
    align-items: flex-start;
    font-size: var(--text-sm);
  }
  .step-n {
    flex-shrink: 0;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-full);
    background: var(--accent-soft);
    color: var(--accent-hi);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-weight: 600;
    font-size: var(--text-xs);
  }
  .hint {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .sub {
    font-size: var(--text-sm);
    margin: var(--space-2) 0 var(--space-1);
  }
  .tips {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: var(--space-2) 0;
  }
  .tips li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    font-size: var(--text-sm);
  }
  .soft {
    font-size: var(--text-sm);
    color: var(--text-dim);
    margin-bottom: var(--space-2);
  }
  .soft summary,
  .advanced summary {
    cursor: pointer;
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
  .advanced summary {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    list-style: none;
  }
  .advanced summary::before {
    content: "▸";
    color: var(--text-dim);
  }
  .advanced[open] summary::before {
    content: "▾";
  }
  .advanced summary:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .adv-title {
    font-weight: 600;
    font-size: var(--text-base);
  }
  .adv-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .gd-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: var(--space-3);
    align-items: start;
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
  .kind-dash,
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
  .node-label {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    cursor: pointer;
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
    cursor: help;
  }
  .tree-foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    padding-top: var(--space-3);
    border-top: 1px solid var(--hairline);
    margin-top: var(--space-2);
  }
  .tree-foot .btn-primary {
    margin-left: auto;
  }
  .path {
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tech {
    align-self: flex-start;
    margin-top: var(--space-2);
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
  .fail {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: #ff453a;
  }
</style>
