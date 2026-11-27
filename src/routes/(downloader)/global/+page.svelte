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
  import SpotlightCard from "#components/global/SpotlightCard.svelte";
  import CourseCard from "#components/global/CourseCard.svelte";
  import CourseCurriculum from "#components/global/CourseCurriculum.svelte";
  import { authProblem, type MarketEntry, type UrlLookup } from "$lib/global/connector-market";
  import {
    collectDownloadable,
    isCourseLibrary,
    type GdTreeNode,
  } from "$lib/global/platform-visuals";

  type Tab = { id: string; url: string; title: string };
  type Sniffed = {
    id: number; tab_id: string; page_title: string; url: string;
    kind: string; mime: string; size: number | null;
  };
  type Resolved = { connector: string | null; tree: GdTreeNode; logs: string[]; items: number; drm: number; ms: number };
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
  type JobOrigin = { tree: GdTreeNode; outDir: string; maxHeight: number | null };

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
  let activeCourseId = $state<string | null>(null);
  let courseFilter = $state("");
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
    !!resolved && downloadable.length > 0 && allLeaves(resolved.tree).every((n) => n.source?.format === "ytdlp"),
  );
  let libraryMode = $derived(!!resolved && isCourseLibrary(resolved.tree));
  let libraryCourses = $derived.by(() => {
    if (!resolved || !libraryMode) return [];
    const list = (resolved.tree.children ?? []).filter((c) => !!c.children?.length);
    const q = courseFilter.trim().toLowerCase();
    return q ? list.filter((c) => c.title.toLowerCase().includes(q)) : list;
  });
  let focusedCourse = $derived.by<GdTreeNode | null>(() => {
    if (!resolved) return null;
    if (!libraryMode) return resolved.tree;
    if (!activeCourseId) return null;
    return (resolved.tree.children ?? []).find((c) => c.id === activeCourseId) ?? null;
  });
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
    activeCourseId = null;
    courseFilter = "";
    selected = new Set(leafIds(r.tree));
    const top = (r.tree.children ?? []).filter((c) => c.children?.length);
    collapsed = new Set(top.length > 1 && leafIds(r.tree).length > 40 ? top.map((c) => c.id) : []);
    showLogs = false;
  }

  function inspectCourse(course: GdTreeNode) {
    activeCourseId = course.id;
    const ids = leafIds(course);
    selected = new Set(ids);
    const modules = (course.children ?? []).filter((c) => c.children?.length);
    collapsed = new Set(modules.length > 2 && ids.length > 30 ? modules.slice(1).map((c) => c.id) : []);
  }

  async function find(reload = false) {
    const url = normalizeUrl(urlInput);
    if (!url) {
      urlField?.focus();
      return;
    }
    resolved = null;
    activeCourseId = null;
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
      runImport,
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

  function allLeaves(n: GdTreeNode): GdTreeNode[] {
    return collectDownloadable(n);
  }

  function leafIds(n: GdTreeNode): string[] {
    return allLeaves(n).map((x) => x.id);
  }

  function checkState(n: GdTreeNode): "all" | "some" | "none" {
    const ids = leafIds(n);
    if (!ids.length) return "none";
    const c = ids.filter((i) => selected.has(i)).length;
    return c === 0 ? "none" : c === ids.length ? "all" : "some";
  }

  function toggle(n: GdTreeNode) {
    const ids = leafIds(n);
    const next = new Set(selected);
    const on = checkState(n) !== "all";
    for (const i of ids) (on ? next.add(i) : next.delete(i));
    selected = next;
  }

  function toggleCollapse(n: GdTreeNode) {
    const next = new Set(collapsed);
    next.has(n.id) ? next.delete(n.id) : next.add(n.id);
    collapsed = next;
  }

  async function pickOutDir() {
    const dir = await openDialog({ directory: true, defaultPath: outDir || undefined });
    if (typeof dir === "string") outDir = dir;
  }

  async function startJob(tree: GdTreeNode, ids: string[] | null, dir: string, height: number | null) {
    await run("download", async () => {
      const job = await invoke<Job>("gd_download", { tree, selected: ids, outDir: dir, maxHeight: height });
      jobOrigins = { ...jobOrigins, [job.id]: { tree, outDir: dir, maxHeight: height } };
      await refresh();
    }, () => startJob(tree, ids, dir, height));
  }

  async function downloadCourse(course: GdTreeNode) {
    if (!resolved) return;
    await startJob(resolved.tree, leafIds(course), outDir, maxHeight);
  }

  async function downloadFocused() {
    if (!resolved || !focusedCourse) return;
    const scopeIds = leafIds(focusedCourse).filter((id) => selected.has(id));
    if (!scopeIds.length) return;
    await startJob(resolved.tree, scopeIds, outDir, maxHeight);
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

{#snippet emptyTips()}
  <ul class="tips">
    <li>
      <span>{$t("global_dl.empty_login")}</span>
      <button type="button" class="btn btn-secondary btn-sm" onclick={loginSite} disabled={!!busy}>{$t("global_dl.login_site")}</button>
    </li>
    <li>
      <span>{$t("global_dl.empty_play")}</span>
      <button type="button" class="btn btn-secondary btn-sm" onclick={() => showWindow(resolvedUrl || normalizeUrl(urlInput))} disabled={!!busy}>{$t("global_dl.show_window")}</button>
    </li>
    <li>
      <span>{$t("global_dl.empty_again")}</span>
      <button type="button" class="btn btn-secondary btn-sm" onclick={() => find(true)} disabled={!!busy}>{$t("global_dl.find_again")}</button>
    </li>
  </ul>
{/snippet}

<section class="gd">
  <header class="gd-head">
    <NavIcon icon="global" size={44} />
    <div class="gd-head-copy">
      <h1>{$t("global_dl.title")}</h1>
      <p>{$t("global_dl.subtitle")}</p>
    </div>
    <div class="gd-head-actions">
      {#if browserState === "downloading" && !browserOpen}
        <span class="pill" role="status">{$t("global_dl.browser_downloading")}</span>
      {:else if windowVisible}
        <span class="pill pill-on">{$t("global_dl.browser_window_on")}</span>
        <button type="button" class="btn btn-secondary btn-sm" onclick={hideWindow} disabled={busy === "window"}>{$t("global_dl.hide_window")}</button>
      {/if}
    </div>
  </header>

  <SpotlightCard accent="var(--accent)" class="hero-card">
    <form class="hero" onsubmit={(e) => { e.preventDefault(); find(); }}>
      <div class="hero-row">
        <div class="input-wrap">
          <svg class="input-ic" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M8.5 11.5a4 4 0 0 0 5.66 0l2-2a4 4 0 0 0-5.66-5.66l-1.1 1.1" />
            <path d="M11.5 8.5a4 4 0 0 0-5.66 0l-2 2a4 4 0 0 0 5.66 5.66l1.1-1.1" />
          </svg>
          <input
            id="gd-url"
            type="text"
            inputmode="url"
            aria-label={$t("global_dl.hero_label")}
            bind:this={urlField}
            bind:value={urlInput}
            placeholder={$t("global_dl.url_placeholder")}
            spellcheck="false"
            autocomplete="off"
          />
        </div>
        <button class="btn btn-primary btn-lg hero-btn" type="submit" disabled={!urlInput.trim() || busy === "find"}>
          {#if busy === "find"}<span class="spinner" aria-hidden="true"></span>{/if}
          {$t("global_dl.find")}
        </button>
      </div>

      <div class="hero-actions">
        <button class="chip-btn" type="button" onclick={loginSite} disabled={!urlInput.trim() || busy === "window"}>
          <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <circle cx="8" cy="5.5" r="2.5" />
            <path d="M3.5 13.5c.8-2.3 2.5-3.5 4.5-3.5s3.7 1.2 4.5 3.5" />
          </svg>
          {$t("global_dl.login_site")}
        </button>
        <button class="chip-btn" type="button" onclick={toggleImport} aria-expanded={importOpen} disabled={busy === "import" && !importOpen}>
          <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M8 2.5v7M5 6.5L8 9.5l3-3M3 12.5h10" />
          </svg>
          {$t("global_dl.import_logins")}
        </button>
      </div>

      {#if !browserOpen && installed.length === 0}
        <p class="hint">{$t("global_dl.no_browser_hint")}</p>
      {/if}
    </form>
  </SpotlightCard>

  {#if !resolved && busy !== "find"}
    <ConnectorHint url={urlInput} offerOnly />
  {/if}

  {#if loginOpen}
    <section class="card callout" aria-live="polite">
      <h2>{$t("global_dl.login_title")}</h2>
      <p>{$t("global_dl.login_hint")}</p>
      {#if loginNote}<p role="status">{loginNote}</p>{/if}
      <div class="row">
        <button type="button" class="btn btn-primary" onclick={loggedIn} disabled={busy === "find"}>{$t("global_dl.login_done")}</button>
        <button type="button" class="btn btn-ghost" onclick={hideWindow} disabled={busy === "window"}>{$t("global_dl.hide_window")}</button>
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
          <button type="button" class="btn btn-primary" onclick={runImport} disabled={busy === "import"}>
            {#if busy === "import"}<span class="spinner" aria-hidden="true"></span>{/if}
            {$t("global_dl.import_button")}
          </button>
          <button type="button" class="btn btn-ghost" onclick={() => (importOpen = false)}>{$t("global_dl.close")}</button>
        </div>
        {#if currentSource?.running}
          <p class="hint">{$t("global_dl.import_running", { name: currentSource.name })}</p>
        {/if}
      {/if}
      {#if importResult}
        <p class="ok" role="status">{$t("global_dl.import_done", { from: importResult.from, profile: importResult.profile })}</p>
        {#if urlInput.trim()}
          <button type="button" class="btn btn-secondary btn-sm" onclick={() => find(true)}>{$t("global_dl.find_again")}</button>
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
          <button type="button" class="btn btn-primary btn-sm" onclick={() => retryLast?.()}>{$t("global_dl.retry")}</button>
        {/if}
        {#if errorKind === "login"}
          <button type="button" class="btn btn-secondary btn-sm" onclick={loginSite}>{$t("global_dl.login_site")}</button>
        {/if}
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => (errorRaw = "")}>{$t("global_dl.close")}</button>
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
    {#if downloadable.length === 0}
      <section class="card">
        <h2 class="sub">{$t("global_dl.empty_title")}</h2>
        <p class="hint">{$t("global_dl.empty_try")}</p>
        {@render emptyTips()}
      </section>
    {:else if libraryMode && !focusedCourse}
      <section class="library">
        <header class="library-head">
          <div class="library-copy">
            <h2>{resolved.tree.title}</h2>
            <span class="note">
              {resolved.connector ? $t("global_dl.via_connector", { name: connectorName(resolved.connector) }) : $t("global_dl.via_page")}
              · {$t("global_dl.courses_count", { count: (resolved.tree.children ?? []).filter((c) => !!c.children?.length).length })}
            </span>
          </div>
          <div class="library-actions">
            {#if (resolved.tree.children ?? []).length > 4}
              <input
                class="course-search"
                type="search"
                bind:value={courseFilter}
                placeholder={$t("global_dl.search_courses")}
                aria-label={$t("global_dl.search_courses")}
                spellcheck="false"
              />
            {/if}
            <button
              type="button"
              class="btn btn-secondary btn-sm"
              onclick={() => startJob(resolved!.tree, downloadable, outDir, maxHeight)}
              disabled={busy === "download"}
            >
              {$t("global_dl.download_all", { count: downloadable.length })}
            </button>
          </div>
        </header>

        <div class="courses-grid">
          {#each libraryCourses as course (course.id)}
            <CourseCard
              {course}
              connectorId={resolved.connector}
              busy={busy === "download"}
              onInspect={inspectCourse}
              onDownload={downloadCourse}
            />
          {/each}
        </div>
      </section>
    {:else if focusedCourse}
      {#if onlyWholePage}
        <details class="card soft">
          <summary>{$t("global_dl.only_page")}</summary>
          {@render emptyTips()}
        </details>
      {/if}
      <CourseCurriculum
        course={focusedCourse}
        connectorId={resolved.connector}
        connectorLabel={resolved.connector ? connectorName(resolved.connector) : ""}
        {selected}
        {collapsed}
        {outDir}
        bind:maxHeight
        busy={busy === "download"}
        showBack={libraryMode}
        onBack={() => (activeCourseId = null)}
        onToggleNode={toggle}
        onToggleCollapse={toggleCollapse}
        onSelectAll={() => (selected = new Set(leafIds(focusedCourse!)))}
        onSelectNone={() => (selected = new Set())}
        onPickOutDir={pickOutDir}
        onDownload={downloadFocused}
      />
    {/if}

    {#if resolved.logs.length || resolved.ms}
      <div class="logs-wrap">
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => (showLogs = !showLogs)} aria-expanded={showLogs}>
          {$t("global_dl.logs")}
        </button>
        {#if showLogs}
          <pre class="logs">{[`${resolved.ms} ms`, ...resolved.logs].join("\n")}</pre>
        {/if}
      </div>
    {/if}
  {:else if !jobs.length}
    <div class="how-strip" aria-label={$t("global_dl.steps_title")}>
      <div class="how-card">
        <span class="how-ic" aria-hidden="true">
          <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="4" width="14" height="12" rx="2" />
            <path d="M7 9h6M7 12h4" />
          </svg>
        </span>
        <strong>{$t("global_dl.step_pick_platform")}</strong>
      </div>
      <div class="how-card">
        <span class="how-ic" aria-hidden="true">
          <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="10" cy="7" r="3" />
            <path d="M4.5 16c1-2.8 3-4 5.5-4s4.5 1.2 5.5 4" />
          </svg>
        </span>
        <strong>{$t("global_dl.step_sign_in_short")}</strong>
      </div>
      <div class="how-card">
        <span class="how-ic" aria-hidden="true">
          <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
            <path d="M10 3.5v9M6.5 9.5L10 13l3.5-3.5M4 16h12" />
          </svg>
        </span>
        <strong>{$t("global_dl.step_download_short")}</strong>
      </div>
    </div>
  {/if}

  {#if jobs.length}
    <section class="card">
      <header class="card-head">
        <h2>{$t("global_dl.jobs")}</h2>
        <button type="button" class="btn btn-ghost btn-sm" onclick={async () => { await invoke("gd_jobs_clear"); await refresh(); }}>
          {$t("global_dl.clear_finished")}
        </button>
      </header>
      <ul class="jobs">
        {#each [...jobs].reverse() as j (j.id)}
          {@const finished = j.done + j.skipped + j.drm + j.failed + j.handed_off}
          {@const pct = j.total ? Math.round((finished / j.total) * 100) : 0}
          {@const failedItems = j.items.filter((i) => i.status === "failed")}
          <li class="job">
            <div class="job-row">
              <span class="tab-title" title={j.title}>{j.title}</span>
              <span class="note">{$t("global_dl.job_progress", { finished, total: j.total })}{j.bytes ? ` · ${bytes(j.bytes)}` : ""}</span>
              <span class="pill" class:pill-on={j.status === "done"} class:pill-fail={j.status === "failed" || j.status === "partial"}>
                {statusLabel(j.status)}
              </span>
              {#if j.status === "running"}
                <button type="button" class="btn btn-ghost btn-sm" onclick={() => invoke("gd_job_cancel", { id: j.id })}>
                  {$t("global_dl.cancel")}
                </button>
              {/if}
            </div>
            <div class="job-bar" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100} aria-label={j.title}>
              <div class="job-bar-fill" class:job-bar-active={j.status === "running"} style:width="{pct}%"></div>
            </div>
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
                  <button type="button" class="btn btn-ghost btn-sm" onclick={() => retryItems(j, [f.id])}>{$t("global_dl.retry")}</button>
                {/if}
              </div>
            {/each}
            {#if j.status !== "running"}
              <div class="row">
                {#if j.done + j.skipped > 0}
                  <button type="button" class="btn btn-secondary btn-sm" onclick={() => openFolder(j.out_dir)} title={j.out_dir}>
                    {$t("global_dl.open_folder")}
                  </button>
                {/if}
                {#if j.handed_off > 0}
                  <a class="btn btn-secondary btn-sm" href="/downloads">{$t("global_dl.see_in_downloads")}</a>
                {/if}
                {#if failedItems.length > 1 && jobOrigins[j.id]}
                  <button type="button" class="btn btn-ghost btn-sm" onclick={() => retryItems(j, failedItems.map((f) => f.id))}>
                    {$t("global_dl.retry_failed")}
                  </button>
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
          <button type="button" class="btn btn-secondary btn-sm" onclick={stopBrowser} disabled={busy === "browser"}>{$t("global_dl.close_browser")}</button>
        {:else}
          <button type="button" class="btn btn-secondary btn-sm" onclick={startBrowser} disabled={busy === "browser"}>
            {busy === "browser" ? $t("global_dl.starting") : $t("global_dl.open_browser")}
          </button>
        {/if}
        {#if windowVisible}
          <button type="button" class="btn btn-ghost btn-sm" onclick={hideWindow} disabled={busy === "window"}>{$t("global_dl.hide_window")}</button>
        {:else}
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => showWindow(null)} disabled={busy === "window"}>{$t("global_dl.show_window")}</button>
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
                    <button type="button" class="btn btn-secondary btn-sm" onclick={() => resolveTab(tab)} disabled={busy === "find" || tab.url.startsWith("about:") || tab.url.startsWith("chrome")}>
                      {$t("global_dl.find")}
                    </button>
                    <button type="button" class="btn btn-ghost btn-sm" onclick={() => (sniffTab = sniffTab === tab.id ? null : tab.id)} aria-pressed={sniffTab === tab.id}>
                      {$t("global_dl.media")} {sniffed.filter((s) => s.tab_id === tab.id).length}
                    </button>
                    <button type="button" class="btn btn-ghost btn-sm" onclick={() => tabAction("focus", tab)} aria-label={$t("global_dl.focus")} title={$t("global_dl.focus")}>↗</button>
                    <button type="button" class="btn btn-ghost btn-sm" onclick={() => tabAction("close", tab)} aria-label={$t("global_dl.close_tab")} title={$t("global_dl.close_tab")}>✕</button>
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
              <button type="button" class="btn btn-ghost btn-sm" onclick={() => (sniffTab = null)}>{$t("global_dl.all_tabs")}</button>
            {/if}
            <button type="button" class="btn btn-ghost btn-sm" onclick={async () => { await invoke("gd_sniffed_clear", { tabId: sniffTab }); sniffed = await invoke("gd_sniffed", { tabId: null }); }}>
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
                  <button type="button" class="btn btn-secondary btn-sm" onclick={() => downloadSniffed(s)}>{$t("global_dl.download")}</button>
                </li>
              {/each}
            </ul>
          {/if}
        </section>

        <section class="card">
          <header class="card-head">
            <h2>{$t("global_dl.connectors")}</h2>
            <button type="button" class="btn btn-ghost btn-sm" onclick={addConnector}>{$t("global_dl.add_connector")}</button>
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
                  <button type="button" class="btn btn-ghost btn-sm" onclick={() => removeDevConnector(c)}>{$t("global_dl.remove")}</button>
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
    max-width: 1120px;
  }

  .gd-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .gd-head-copy {
    flex: 1;
    min-width: 0;
  }

  .gd-head h1 {
    font-family: var(--font-display);
    font-size: var(--text-xl, 22px);
    letter-spacing: var(--track-tight);
    margin: 0;
  }

  .gd-head p {
    margin: 2px 0 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .gd-head-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .hero {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
  }

  .hero-row {
    display: flex;
    gap: var(--space-2);
  }

  .input-wrap {
    flex: 1;
    min-width: 0;
    position: relative;
    display: flex;
    align-items: center;
  }

  .input-ic {
    position: absolute;
    left: 12px;
    width: 16px;
    height: 16px;
    color: var(--text-dim);
    pointer-events: none;
  }

  .input-wrap input {
    width: 100%;
    height: var(--control-h-lg, 38px);
    padding: 0 14px 0 36px;
    border-radius: var(--radius-md, 8px);
    border: 1px solid var(--hairline);
    background: var(--bg);
    color: var(--text);
    font-size: var(--text-base);
    transition: border-color 140ms ease;
  }

  .input-wrap input:focus-visible,
  .urlbar input:focus-visible,
  .course-search:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
    border-color: var(--accent);
  }

  .hero-btn {
    flex-shrink: 0;
    padding: 0 18px;
  }

  .hero-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .chip-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 10px;
    border-radius: var(--radius-full, 9999px);
    border: 1px solid var(--hairline);
    background: color-mix(in srgb, var(--fill-2) 60%, transparent);
    color: var(--text-dim);
    font-size: var(--text-xs);
    cursor: pointer;
    transition: background 120ms ease, color 120ms ease;
  }

  .chip-btn:hover:not(:disabled) {
    background: var(--fill-2);
    color: var(--text);
  }

  .chip-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .chip-btn svg {
    width: 13px;
    height: 13px;
  }

  .how-strip {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--space-3);
  }

  @media (max-width: 720px) {
    .how-strip {
      grid-template-columns: 1fr;
    }
  }

  .how-card {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-lg, 12px);
    background: color-mix(in srgb, var(--surface) 75%, transparent);
    border: 1px solid var(--hairline);
  }

  .how-card strong {
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text-dim);
    line-height: 1.4;
  }

  .how-ic {
    width: 34px;
    height: 34px;
    border-radius: 9px;
    display: grid;
    place-items: center;
    flex-shrink: 0;
    background: var(--accent-soft, color-mix(in srgb, var(--accent) 14%, transparent));
    color: var(--accent-hi, var(--accent));
  }

  .how-ic svg {
    width: 17px;
    height: 17px;
  }

  .library {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .library-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .library-copy h2 {
    margin: 0;
    font-size: var(--text-lg, 17px);
    font-weight: 600;
  }

  .library-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .course-search {
    width: 200px;
    height: var(--control-h, 28px);
    padding: 0 10px;
    border-radius: var(--radius-sm, 6px);
    border: 1px solid var(--hairline);
    background: var(--surface);
    color: var(--text);
    font-size: var(--text-xs);
  }

  .courses-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: var(--space-3);
  }

  .logs-wrap {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }

  .pill {
    font-size: var(--text-xs);
    padding: 2px 10px;
    border-radius: var(--radius-full, 9999px);
    background: var(--fill-2);
    color: var(--text-dim);
    white-space: nowrap;
  }

  .pill-fail {
    color: var(--danger, #ff453a);
  }

  .pill-on {
    background: color-mix(in srgb, #34c759 22%, transparent);
    color: var(--text);
  }

  .callout h2,
  .error h2 {
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

  .hint {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-xs);
  }

  .sub {
    font-size: var(--text-base);
    margin: 0 0 var(--space-1);
  }

  .tips {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: var(--space-2) 0 0;
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
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: var(--space-3);
    align-items: start;
  }

  .card {
    background: var(--surface);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-lg, 12px);
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

  .logs {
    width: 100%;
    max-height: 160px;
    overflow: auto;
    font-size: 11px;
    background: var(--fill-2);
    padding: var(--space-2);
    border-radius: 8px;
    white-space: pre-wrap;
    margin: 0;
  }

  .job {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: var(--space-3) 0;
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
    font-weight: 600;
  }

  .job-bar {
    width: 100%;
    height: 6px;
    border-radius: var(--radius-full, 9999px);
    background: var(--fill-2);
    overflow: hidden;
  }

  .job-bar-fill {
    height: 100%;
    border-radius: var(--radius-full, 9999px);
    background: var(--accent);
    transition: width 220ms ease;
  }

  .job-bar-active {
    background: linear-gradient(90deg, var(--accent), color-mix(in srgb, var(--accent) 60%, #fff), var(--accent));
    background-size: 200% 100%;
    animation: job-flow 1.4s linear infinite;
  }

  @keyframes job-flow {
    from { background-position: 200% 0; }
    to { background-position: -200% 0; }
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
