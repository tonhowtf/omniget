<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open as openShell } from "@tauri-apps/plugin-shell";
  import { save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { settingsHref } from "$lib/settings-navigation";
  import { getSettings, saveSettings } from "./settings-helpers";
  import CookieBucketCard from "#components/cookies/CookieBucketCard.svelte";
  import CookieImportMenu from "#components/cookies/CookieImportMenu.svelte";
  import CookieContentModal from "#components/cookies/CookieContentModal.svelte";
  import CookiePasteModal from "#components/cookies/CookiePasteModal.svelte";
  import BilibiliQrLogin from "#components/cookies/BilibiliQrLogin.svelte";
  import SettingGroup from "./rebrand/SettingGroup.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingsEmptyState from "./rebrand/SettingsEmptyState.svelte";
  import ManagerLink from "./rebrand/ManagerLink.svelte";
  import MoreOptions from "./downloads/MoreOptions.svelte";
  import Toggle from "./downloads/Toggle.svelte";
  import KeyAnchor from "./downloads/KeyAnchor.svelte";
  import DialogContainer from "#components/dialog/DialogContainer.svelte";

  type Account = {
    slug: string;
    alias: string;
    source_url: string | null;
    source_label: string | null;
    captured_at_ms: number;
    cookie_count: number;
    last_used_at_ms: number | null;
  };
  type BucketEntry = { platform_kind: string; accounts: Account[] };
  type Registry = { buckets: Record<string, BucketEntry> };
  type ListResponse = { registry: Registry; cookies_dir: string };
  type ImportResponse = {
    buckets_written: Array<{ domain: string; cookie_count: number; platform_kind: string }>;
  };

  type HealthItem = {
    domain: string;
    slug: string;
    status: string;
    age_days: number;
    expires_in_days: number;
    cookie_count: number;
  };
  type HealthResponse = { items: HealthItem[]; fresh_days: number; expire_days: number };

  let registry = $state<Registry>({ buckets: {} });
  let healthByKey = $state<Record<string, HealthItem>>({});

  function safeErrorText(e: unknown): string {
    if (typeof e === "string" && /^[a-z][\w-]*(\.[\w-]+)+$/.test(e)) {
      const text = $t(e) as string;
      if (text && text !== e) return text;
    }
    return $t("common.error") as string;
  }

  function redactDiagnostic(message: string): string {
    return message
      .replace(/([\w-]+)=([^\s;&"']+)/g, "$1=…")
      .replace(/[A-Za-z0-9_\-%+/]{24,}={0,2}/g, "…")
      .slice(0, 160);
  }

  function healthKey(domain: string, slug: string): string {
    return `${domain}__${slug}`;
  }

  function bucketHealth(domain: string): Record<string, HealthItem> {
    const out: Record<string, HealthItem> = {};
    for (const v of Object.values(healthByKey)) {
      if (v.domain === domain) out[v.slug] = v;
    }
    return out;
  }

  let testing = $state<Record<string, boolean>>({});
  let selectedCookies = $state<Record<string, boolean>>({});

  function cookieTestUrl(domain: string, platformKind: string | undefined): string {
    if (platformKind === "bilibili") return "https://www.bilibili.com/video/BV1xx411c7mu";
    if (platformKind === "youtube" || platformKind === "youtube_music") {
      return "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
    }
    if (platformKind === "twitch") return "https://www.twitch.tv/videos/1";
    return `https://${domain}`;
  }

  async function handleTest(domain: string, slug: string) {
    const bucket = registry.buckets[domain];
    const url = cookieTestUrl(domain, bucket?.platform_kind);
    const key = healthKey(domain, slug);
    testing = { ...testing, [key]: true };
    try {
      const res = await invoke<{ ok: boolean; message: string }>("cookies_test", {
        request: { url, slug },
      });
      if (res.ok) {
        showToast("success", $t("settings.cookies.test_ok") as string);
      } else {
        showToast(
          "error",
          $t("settings.cookies.test_fail", { msg: redactDiagnostic(res.message) }) as string,
        );
      }
    } catch (e) {
      showToast("error", safeErrorText(e));
    } finally {
      testing = { ...testing, [key]: false };
    }
  }

  let settings = $derived(getSettings());
  let managedOnly = $derived(settings?.download.always_use_managed_cookies ?? true);

  async function toggleManagedOnly() {
    await saveSettings({ download: { always_use_managed_cookies: !managedOnly } });
  }

  async function changeFromBrowser(e: Event) {
    const value = (e.currentTarget as HTMLInputElement).value.trim();
    await saveSettings({ advanced: { cookies_from_browser: value } });
  }

  let selectMode = $state(false);
  let cookiesDir = $state("");
  type LoadState = { kind: "loading" } | { kind: "ready" } | { kind: "error" };
  let loadState = $state<LoadState>({ kind: "loading" });
  let hasLoaded = $state(false);
  let pasteOpen = $state(false);

  let viewOpen = $state(false);
  let viewDomain = $state("");
  let viewSlug = $state("");
  let viewContent = $state("");
  let viewPath = $state("");

  let confirmClearOpen = $state(false);
  let confirmDomain = $state("");
  let confirmSlug = $state("");
  let confirmBulkClearOpen = $state(false);

  let addAccountOpen = $state(false);
  let addAccountDomain = $state("");
  let bilibiliQrOpen = $state(false);

  let bucketList = $derived(
    Object.entries(registry.buckets)
      .map(([domain, b]) => ({ domain, ...b }))
      .sort((a, b) => {
        const aTime = a.accounts[0]?.captured_at_ms ?? 0;
        const bTime = b.accounts[0]?.captured_at_ms ?? 0;
        return bTime - aTime;
      }),
  );
  let selectedCookieItems = $derived.by(() => {
    const items: Array<{ domain: string; slug: string; alias: string }> = [];
    for (const [domain, bucket] of Object.entries(registry.buckets)) {
      for (const account of bucket.accounts) {
        if (selectedCookies[healthKey(domain, account.slug)]) {
          items.push({ domain, slug: account.slug, alias: account.alias });
        }
      }
    }
    return items;
  });
  let selectedCount = $derived(selectedCookieItems.length);

  async function reload() {
    loadState = { kind: "loading" };
    try {
      const res = await invoke<ListResponse>("cookies_list");
      registry = res.registry;
      cookiesDir = res.cookies_dir;
      const validSelected: Record<string, boolean> = {};
      for (const [domain, bucket] of Object.entries(res.registry.buckets)) {
        for (const account of bucket.accounts) {
          const key = healthKey(domain, account.slug);
          if (selectedCookies[key]) validSelected[key] = true;
        }
      }
      selectedCookies = validSelected;
      try {
        const h = await invoke<HealthResponse>("cookies_health");
        const map: Record<string, HealthItem> = {};
        for (const item of h.items) map[healthKey(item.domain, item.slug)] = item;
        healthByKey = map;
      } catch {
        healthByKey = {};
      }
      hasLoaded = true;
      loadState = { kind: "ready" };
    } catch {
      loadState = { kind: "error" };
    }
  }

  async function importContent(content: string, sourceUrl: string, alias: string, label: string) {
    try {
      const res = await invoke<ImportResponse>("cookies_import", {
        request: {
          content,
          source_url: sourceUrl || null,
          source_label: label,
          alias: alias || null,
        },
      });
      const total = res.buckets_written.reduce((s, b) => s + b.cookie_count, 0);
      showToast(
        "success",
        ($t("settings.cookies.import_success", {
          buckets: String(res.buckets_written.length),
          cookies: String(total),
        }) as string),
      );
      pasteOpen = false;
      await reload();
    } catch (e) {
      showToast("error", safeErrorText(e));
    }
  }

  async function handleImportFile(path: string) {
    try {
      const res = await invoke<ImportResponse>("cookies_import_file", {
        request: { path, alias: null },
      });
      const total = res.buckets_written.reduce((s, b) => s + b.cookie_count, 0);
      showToast(
        "success",
        ($t("settings.cookies.import_success", {
          buckets: String(res.buckets_written.length),
          cookies: String(total),
        }) as string),
      );
      await reload();
    } catch (e) {
      showToast("error", safeErrorText(e));
    }
  }

  async function handleImportPasteContent(content: string, sourceUrl: string, alias: string) {
    await importContent(content, sourceUrl, alias, $t("settings.downloads.cookies.paste_label") as string);
  }

  async function handleView(domain: string, slug: string) {
    try {
      const res = await invoke<{ content: string; path: string }>("cookies_read", {
        request: { domain, slug },
      });
      viewContent = res.content;
      viewPath = res.path;
      viewDomain = domain;
      viewSlug = slug;
      viewOpen = true;
    } catch (e) {
      showToast("error", safeErrorText(e));
    }
  }

  async function handleExport(domain: string, slug: string) {
    try {
      const saved = await saveDialog({
        title: $t("settings.cookies.export_dialog_title") as string,
        defaultPath: `${domain}-cookies.txt`,
        filters: [{ name: "Netscape Cookies", extensions: ["txt"] }],
      });
      if (saved && typeof saved === "string") {
        await invoke("cookies_export_to", {
          request: { domain, slug, destination_path: saved },
        });
        showToast("success", $t("settings.cookies.export_done") as string);
      }
    } catch (e) {
      showToast("error", safeErrorText(e));
    }
  }

  async function handleRename(domain: string, slug: string, newAlias: string) {
    try {
      await invoke("cookies_rename", { request: { domain, slug, new_alias: newAlias } });
      await reload();
    } catch (e) {
      showToast("error", safeErrorText(e));
    }
  }

  function askClear(domain: string, slug: string) {
    confirmDomain = domain;
    confirmSlug = slug;
    confirmClearOpen = true;
  }

  function toggleCookieSelection(domain: string, slug: string, checked: boolean) {
    const key = healthKey(domain, slug);
    if (checked) {
      selectedCookies = { ...selectedCookies, [key]: true };
      return;
    }
    const next = { ...selectedCookies };
    delete next[key];
    selectedCookies = next;
  }

  function selectAllCookies() {
    const next: Record<string, boolean> = {};
    for (const bucket of bucketList) {
      for (const account of bucket.accounts) {
        next[healthKey(bucket.domain, account.slug)] = true;
      }
    }
    selectedCookies = next;
  }

  function clearCookieSelection() {
    selectedCookies = {};
  }

  function askBulkClear() {
    if (selectedCount === 0) return;
    confirmBulkClearOpen = true;
  }

  function openAddAccount(domain: string) {
    addAccountDomain = domain;
    if (domain === "bilibili.com") {
      bilibiliQrOpen = true;
      return;
    }
    addAccountOpen = true;
  }

  async function handleBilibiliQrSuccess(_slug: string, uname: string) {
    bilibiliQrOpen = false;
    showToast("success", $t("settings.cookies.bilibili.login_qr_success", { uname }) as string);
    await reload();
  }

  let bilibiliWebviewLoading = $state(false);
  async function startBilibiliWebviewLogin() {
    if (bilibiliWebviewLoading) return;
    bilibiliWebviewLoading = true;
    try {
      const res = await invoke<{ slug: string; uname: string; mid: number; is_vip: boolean }>(
        "bilibili_webview_login",
      );
      showToast("success", $t("settings.cookies.bilibili.login_qr_success", { uname: res.uname }) as string);
      await reload();
    } catch (e) {
      const key = typeof e === "string" ? e : "errors.bilibili.network_failed";
      showToast("error", safeErrorText(key));
    } finally {
      bilibiliWebviewLoading = false;
    }
  }

  type BilibiliImportedItem = { url: string; title: string };
  type BilibiliImportResult = { total: number; items: BilibiliImportedItem[] };

  function bilibiliAccountSlug(): string | null {
    const bucket = registry.buckets["bilibili.com"];
    if (!bucket) return null;
    const real = bucket.accounts.find((a) => !a.slug.startsWith("_") && a.cookie_count > 0);
    if (real) return real.slug;
    const imported = bucket.accounts.find((a) => a.slug === "_default" && a.cookie_count > 0);
    return imported?.slug ?? null;
  }

  async function enqueueImportedUrls(items: BilibiliImportedItem[]) {
    const settings = getSettings();
    const outDir = settings?.download?.default_output_dir ?? "";
    for (const it of items) {
      try {
        await invoke("download_from_url", {
          url: it.url,
          outputDir: outDir,
          downloadMode: null,
          quality: null,
          formatId: null,
          referer: "https://www.bilibili.com",
          cookieSlug: null,
          timeRange: null,
          playlistItems: null,
          torrentFiles: null,
          scheduledAt: null,
          stopAt: null,
        });
      } catch (e) {
        console.warn("[bilibili import] enqueue failed", it.url, e);
      }
    }
  }

  async function importBilibiliWatchLater() {
    const slug = bilibiliAccountSlug();
    if (!slug) {
      showToast("error", $t("settings.cookies.bilibili.import_no_account") as string);
      return;
    }
    try {
      const res = await invoke<BilibiliImportResult>("bilibili_import_watch_later", { slug });
      showToast("success", $t("settings.cookies.bilibili.import_done", { count: String(res.items.length) }) as string);
      await enqueueImportedUrls(res.items);
    } catch (e) {
      showToast("error", safeErrorText(e));
    }
  }

  async function importBilibiliHistory() {
    const slug = bilibiliAccountSlug();
    if (!slug) {
      showToast("error", $t("settings.cookies.bilibili.import_no_account") as string);
      return;
    }
    try {
      const res = await invoke<BilibiliImportResult>("bilibili_import_history", { slug });
      showToast("success", $t("settings.cookies.bilibili.import_done", { count: String(res.items.length) }) as string);
      await enqueueImportedUrls(res.items);
    } catch (e) {
      showToast("error", safeErrorText(e));
    }
  }

  async function submitAddAccount(content: string, sourceUrl: string, alias: string) {
    try {
      const res = await invoke<{ slug: string; cookie_count: number }>("cookies_add_account", {
        request: {
          domain: addAccountDomain,
          alias: alias || ($t("settings.downloads.cookies.default_alias") as string),
          content,
          source_url: sourceUrl || null,
        },
      });
      showToast(
        "success",
        ($t("settings.cookies.add_account_success", {
          slug: res.slug,
          count: String(res.cookie_count),
        }) as string),
      );
      addAccountOpen = false;
      await reload();
    } catch (e) {
      showToast("error", safeErrorText(e));
    }
  }

  async function confirmClear() {
    try {
      await invoke("cookies_clear", { request: { domain: confirmDomain, slug: confirmSlug } });
      confirmClearOpen = false;
      await reload();
      showToast("success", $t("settings.cookies.cleared_toast") as string);
    } catch (e) {
      showToast("error", safeErrorText(e));
    }
  }

  async function confirmBulkClear() {
    const items = selectedCookieItems.map(({ domain, slug }) => ({ domain, slug }));
    if (items.length === 0) {
      confirmBulkClearOpen = false;
      return;
    }
    try {
      const res = await invoke<{ ok: boolean; cleared: number }>("cookies_clear_batch", {
        request: { items },
      });
      confirmBulkClearOpen = false;
      clearCookieSelection();
      selectMode = false;
      await reload();
      showToast("success", $t("settings.downloads.cookies.cleared_count", { count: res.cleared }) as string);
    } catch (e) {
      showToast("error", safeErrorText(e));
    }
  }

  async function openCookiesDir() {
    if (!cookiesDir) return;
    try {
      await openShell(cookiesDir);
    } catch (e) {
      console.warn("[cookies] open dir failed", e);
    }
  }

  async function toggleSelectMode() {
    selectMode = !selectMode;
    if (!selectMode) clearCookieSelection();
    await tick();
    document.getElementById(selectMode ? "cookies-bulk-first" : "cookies-select")?.focus();
  }

  onMount(() => {
    void reload();
  });
</script>

<SettingGroup id="cookies" icon="cookie" title={$t("settings.downloads.cookies.title")} description={$t("settings.cookies.intro")}>
  {#snippet actions()}
    <span data-setting-key="settings.cookies.import_trigger">
      <CookieImportMenu onImportFile={handleImportFile} onImportPaste={() => pasteOpen = true} />
    </span>
  {/snippet}

  {#if loadState.kind === "error"}
    <SettingsEmptyState error title={$t("settings.downloads.cookies.load_failed")} hint={$t("settings.downloads.cookies.load_failed_hint")} onRetry={() => void reload()} />
  {/if}

  {#if selectMode && hasLoaded && bucketList.length > 0}
    <div class="bulk-bar" role="group" aria-label={$t("settings.downloads.cookies.select") as string}>
      <span class="bulk-info" role="status">{$t("settings.downloads.cookies.selected", { count: selectedCount })}</span>
      <div class="bulk-actions">
        <button type="button" id="cookies-bulk-first" class="btn btn-secondary btn-sm" onclick={selectAllCookies}>{$t("settings.cookies.select_all")}</button>
        <button type="button" class="btn btn-secondary btn-sm" onclick={clearCookieSelection} disabled={selectedCount === 0}>{$t("common.clear")}</button>
        <button type="button" class="danger-btn" onclick={askBulkClear} disabled={selectedCount === 0}>{$t("settings.downloads.cookies.delete")}</button>
        <button type="button" class="btn btn-secondary btn-sm" onclick={toggleSelectMode}>{$t("settings.downloads.cookies.done")}</button>
      </div>
    </div>
  {/if}

  {#if !hasLoaded}
    {#if loadState.kind === "loading"}
      <div class="loading" aria-busy="true">
        <p class="visually-hidden" role="status">{$t("settings.rebrand.state.loading")}</p>
        {#each Array(3) as _, i (i)}
          <div class="skel" aria-hidden="true"></div>
        {/each}
      </div>
    {/if}
  {:else if bucketList.length === 0}
    {#if loadState.kind === "ready"}
      <SettingsEmptyState title={$t("settings.cookies.empty_title")} hint={$t("settings.cookies.empty_body")} />
    {/if}
  {:else}
    <div class="buckets" aria-busy={loadState.kind === "loading"}>
      {#each bucketList as bucket (bucket.domain)}
        <CookieBucketCard
          {bucket}
          health={bucketHealth(bucket.domain)}
          testing={testing}
          onView={handleView}
          onExport={handleExport}
          onRename={handleRename}
          onClear={askClear}
          onAddAccount={openAddAccount}
          onTest={handleTest}
          selected={selectedCookies}
          onToggleSelection={selectMode ? toggleCookieSelection : undefined}
        />
        {#if bucket.domain === "bilibili.com"}
          {#if bucket.accounts.some((a) => !a.slug.startsWith("_") && a.cookie_count > 0)}
            <div class="bilibili-importers">
              <button type="button" class="btn btn-secondary btn-sm" onclick={importBilibiliWatchLater}>
                {$t("settings.cookies.bilibili.import_watch_later")}
              </button>
              <button type="button" class="btn btn-secondary btn-sm" onclick={importBilibiliHistory}>
                {$t("settings.cookies.bilibili.import_history")}
              </button>
            </div>
          {:else}
            <div class="bilibili-importers">
              <button type="button" class="btn btn-secondary btn-sm" onclick={startBilibiliWebviewLogin} disabled={bilibiliWebviewLoading} aria-busy={bilibiliWebviewLoading}>
                {bilibiliWebviewLoading
                  ? $t("settings.cookies.bilibili.login_webview_loading")
                  : $t("settings.cookies.bilibili.login_webview_btn")}
              </button>
            </div>
          {/if}
        {/if}
      {/each}
    </div>
  {/if}

  <div data-setting-key="settings.cookies.capture_via_extension">
    <ManagerLink href={settingsHref("integrations", "extension")} titleKey="settings.cookies.capture_via_extension" hintKey="settings.cookies.hint_use_extension" />
  </div>

  <MoreOptions id="cookies">
    <SettingRow
      id="cookies-managed-only"
      label={$t("settings.downloads.cookies.managed_only")}
      help={$t("settings.downloads.cookies.managed_only_desc")}
      settingKey="settings.downloads.cookies.managed_only"
      status="download.always_use_managed_cookies"
    >
      {#snippet control(c)}<Toggle id={c.id} describedBy={c.describedBy} on={managedOnly} onclick={toggleManagedOnly} />{/snippet}
    </SettingRow>
    <SettingRow
      id="cookies-from-browser"
      label={$t("settings.downloads.cookies.from_browser")}
      help={$t("settings.downloads.cookies.from_browser_desc")}
      settingKey="settings.downloads.cookies.from_browser"
      status="advanced.cookies_from_browser"
      disabledReason={managedOnly ? ($t("settings.downloads.cookies.from_browser_blocked") as string) : undefined}
    >
      {#snippet control(c)}
        <KeyAnchor keys={["settings.advanced.cookies_from_browser"]} fill>
          <input id={c.id} aria-describedby={c.describedBy} type="text" class="input-text" dir="ltr" spellcheck="false"
            placeholder={$t("settings.downloads.cookies.from_browser_placeholder") as string}
            value={settings?.advanced?.cookies_from_browser ?? ""} onchange={changeFromBrowser} />
        </KeyAnchor>
      {/snippet}
    </SettingRow>
    <SettingRow id="cookies-select" label={$t("settings.downloads.cookies.select")} help={$t("settings.downloads.cookies.select_desc")} settingKey="settings.downloads.cookies.select"
      disabledReason={hasLoaded && bucketList.length === 0 ? ($t("settings.cookies.empty_title") as string) : undefined}>
      {#snippet control(c)}
        <button id={c.id} type="button" class="btn btn-secondary btn-sm" aria-describedby={c.describedBy} aria-pressed={selectMode} disabled={!hasLoaded || bucketList.length === 0} onclick={toggleSelectMode}>
          {selectMode ? $t("settings.downloads.cookies.done") : $t("settings.downloads.cookies.select_action")}
        </button>
      {/snippet}
    </SettingRow>
    <SettingRow id="cookies-folder" label={$t("settings.downloads.cookies.folder")} help={cookiesDir || undefined} settingKey="settings.downloads.cookies.folder">
      {#snippet control(c)}
        <button id={c.id} type="button" class="btn btn-secondary btn-sm" aria-describedby={c.describedBy} onclick={openCookiesDir} disabled={!cookiesDir}>{$t("settings.cookies.open_folder")}</button>
      {/snippet}
    </SettingRow>
  </MoreOptions>
</SettingGroup>

<CookieContentModal
  open={viewOpen}
  domain={viewDomain}
  slug={viewSlug}
  content={viewContent}
  path={viewPath}
  onClose={() => viewOpen = false}
/>

<CookiePasteModal
  open={pasteOpen}
  onSubmit={handleImportPasteContent}
  onClose={() => pasteOpen = false}
/>

<CookiePasteModal
  open={addAccountOpen}
  onSubmit={submitAddAccount}
  onClose={() => addAccountOpen = false}
/>

<BilibiliQrLogin
  open={bilibiliQrOpen}
  onClose={() => (bilibiliQrOpen = false)}
  onSuccess={handleBilibiliQrSuccess}
/>

<DialogContainer bind:isOpen={confirmClearOpen} titleId="cookies-confirm-clear-title">
  <div class="confirm">
    <h3 id="cookies-confirm-clear-title">{$t("settings.cookies.confirm_clear_title")}</h3>
    <p>{$t("settings.cookies.confirm_clear_body", { domain: confirmDomain })}</p>
    <div class="confirm-actions">
      <button type="button" class="ghost-btn" onclick={() => confirmClearOpen = false}>
        {$t("settings.cookies.confirm_clear_cancel")}
      </button>
      <button type="button" class="danger-btn" onclick={confirmClear}>
        {$t("settings.cookies.confirm_clear_action")}
      </button>
    </div>
  </div>
</DialogContainer>

<DialogContainer bind:isOpen={confirmBulkClearOpen} titleId="cookies-confirm-bulk-title">
  <div class="confirm">
    <h3 id="cookies-confirm-bulk-title">{$t("settings.cookies.confirm_clear_title")}</h3>
    <p>{$t("settings.downloads.cookies.delete_selected_body", { count: selectedCount })}</p>
    <div class="confirm-actions">
      <button type="button" class="ghost-btn" onclick={() => confirmBulkClearOpen = false}>
        {$t("settings.cookies.confirm_clear_cancel")}
      </button>
      <button type="button" class="danger-btn" onclick={confirmBulkClear}>
        {$t("settings.downloads.cookies.delete")}
      </button>
    </div>
  </div>
</DialogContainer>

<style>
  .bulk-bar {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    margin-block-end: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--primary);
    border: 1px solid var(--content-border);
    border-radius: var(--radius-md);
  }
  .bulk-info { color: var(--text); font-size: var(--text-sm); }
  .bulk-actions { display: flex; flex-wrap: wrap; gap: var(--space-2); justify-content: flex-end; }
  .ghost-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 7px 14px;
    background: transparent;
    border: none;
    border-radius: var(--radius-full);
    color: var(--secondary);
    font-size: var(--text-base);
    cursor: pointer;
  }
  .ghost-btn:hover:not(:disabled) { color: var(--accent); }
  .ghost-btn:focus-visible, .danger-btn:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .loading { display: flex; flex-direction: column; gap: var(--space-3); }
  .skel {
    height: 92px;
    border-radius: var(--radius-lg);
    background: linear-gradient(
      90deg,
      color-mix(in oklab, var(--button) 35%, transparent) 0%,
      color-mix(in oklab, var(--button) 60%, transparent) 50%,
      color-mix(in oklab, var(--button) 35%, transparent) 100%
    );
    background-size: 200% 100%;
    animation: shimmer 1.5s ease-in-out infinite;
  }
  @keyframes shimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }
  @media (prefers-reduced-motion: reduce) { .skel { animation: none; } }
  :global([data-reduce-motion="true"]) .skel { animation: none; }
  .buckets { display: flex; flex-direction: column; gap: 10px; margin-block-end: var(--space-2); }
  .confirm {
    padding: var(--space-5);
    display: flex; flex-direction: column; gap: var(--space-3);
  }
  .confirm h3 { margin: 0; font-size: var(--text-md); color: var(--secondary); }
  .confirm p { margin: 0; font-size: var(--text-base); color: var(--secondary); line-height: 1.5; }
  .confirm-actions { display: flex; justify-content: flex-end; gap: var(--space-2); margin-top: 6px; }
  .danger-btn {
    padding: 5px 14px;
    background: var(--danger);
    color: var(--on-status);
    border: 0;
    border-radius: var(--radius-full);
    font: inherit;
    font-size: var(--text-base);
    cursor: pointer;
  }
  .danger-btn:hover:not(:disabled) { filter: brightness(1.1); }
  .danger-btn:disabled { opacity: 0.45; cursor: not-allowed; }
  .bilibili-importers { display: flex; gap: var(--space-2); flex-wrap: wrap; margin: calc(-1 * var(--space-1)) 0 var(--space-2); padding-inline-start: var(--space-3); }
</style>
