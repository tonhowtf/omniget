<script lang="ts">
  import { page } from "$app/stores";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { listen } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import CourseCard from "$components/hotmart/CourseCard.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getDownloads } from "$lib/stores/download-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { t } from "$lib/i18n";

  let downloads = $derived(getDownloads());

  type PlatformConfig = {
    id: string;
    name: string;
    color: string;
    icon: string;
    login_methods: LoginMethod[];
    commands: { check_session: string; logout: string; list: string; refresh: string; download: string; cancel?: string; search?: string };
    features: { captcha_event?: string; has_search?: boolean; download_arg_name?: string; list_returns_key?: string; item_subtitle_field?: string; session_display?: string; string_ids?: boolean };
  };

  type LoginMethod = {
    method_type: string;
    command: string;
    extra_fields: { key: string; label: string; placeholder: string; field_type: string }[];
  };

  let platformId = $derived($page.params.platform);
  let config = $state<PlatformConfig | null>(null);
  let configError = $state("");

  let email = $state("");
  let password = $state("");
  let token = $state("");
  let cookiesText = $state("");
  let extraFields: Record<string, string> = $state({});
  let loginTabIndex = $state(0);
  let loading = $state(false);
  let error = $state("");
  let captchaWarning = $state(false);

  let checking = $state(true);
  let loggedIn = $state(false);
  let sessionEmail = $state("");

  let items: any[] = $state([]);
  let loadingItems = $state(false);
  let itemsError = $state("");

  let searchQuery = $state("");

  const ITEMS_PER_PAGE = 12;
  let currentPage = $state(1);

  let totalPages = $derived(Math.max(1, Math.ceil(items.length / ITEMS_PER_PAGE)));
  let paginatedItems = $derived(
    items.slice((currentPage - 1) * ITEMS_PER_PAGE, currentPage * ITEMS_PER_PAGE)
  );

  let pageNumbers = $derived((): number[] => {
    const pages: number[] = [];
    if (totalPages <= 7) {
      for (let i = 1; i <= totalPages; i++) pages.push(i);
    } else {
      pages.push(1);
      if (currentPage > 3) pages.push(-1);
      const start = Math.max(2, currentPage - 1);
      const end = Math.min(totalPages - 1, currentPage + 1);
      for (let i = start; i <= end; i++) pages.push(i);
      if (currentPage < totalPages - 2) pages.push(-1);
      pages.push(totalPages);
    }
    return pages;
  });

  let currentMethod = $derived(config?.login_methods[loginTabIndex] ?? null);

  function hashCode(s: string): number {
    let hash = 0;
    for (let i = 0; i < s.length; i++) {
      hash = ((hash << 5) - hash + s.charCodeAt(i)) | 0;
    }
    return Math.abs(hash);
  }

  function getItemId(item: any): number {
    if (typeof item.id === "number") return item.id;
    return hashCode(String(item.id));
  }

  function getItemName(item: any): string {
    return item.name || item.title || "—";
  }

  function getItemSubtitle(item: any): string {
    if (!config?.features.item_subtitle_field) return "—";
    const field = config.features.item_subtitle_field;
    if (field === "price") {
      const p = item.price;
      if (p === null || p === undefined) return "—";
      if (p === 0) return $t("hotmart.free");
      return `R$ ${Number(p).toFixed(2).replace(".", ",")}`;
    }
    if (field === "num_published_lectures") {
      const n = item.num_published_lectures;
      return n ? `${n} lectures` : "—";
    }
    if (field === "num_courses") {
      const n = item.num_courses;
      return n ? `${n} courses` : "—";
    }
    const val = item[field];
    return val ? String(val) : "—";
  }

  function getItemImage(item: any): string | undefined {
    return item.image_url ?? item.preview_url ?? undefined;
  }

  function goToPage(p: number) {
    if (p >= 1 && p <= totalPages) currentPage = p;
  }

  $effect(() => {
    const id = platformId;
    if (!id) return;
    loadConfig(id);
  });

  async function loadConfig(id: string) {
    checking = true;
    configError = "";
    console.log(`[courses/${id}] loading config...`);
    try {
      config = await pluginInvoke<PlatformConfig>("courses", "get_platform_config", { platform: id });
      console.log(`[courses/${id}] config loaded:`, JSON.stringify(config));
      if (config.features.captcha_event) {
        const evt = config.features.captcha_event;
        const unlisten = listen(evt, () => { captchaWarning = true; });
        return () => { unlisten.then((fn) => fn()); };
      }
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "";
      console.error(`[courses/${id}] get_platform_config FAILED:`, e, "| raw:", JSON.stringify(e));
      if (msg.includes("Unknown command") || msg.includes("not found") || msg.includes("No handler")) {
        configError = $t("courses.update_plugin_for_platform");
      } else {
        configError = msg || "Failed to load platform config";
      }
      checking = false;
      return;
    }
    checkSession();
  }

  async function checkSession() {
    if (!config) return;
    try {
      const result = await pluginInvoke<string>("courses", config.commands.check_session);
      sessionEmail = result;
      loggedIn = true;
      loadItems();
    } catch {
      loggedIn = false;
    } finally {
      checking = false;
    }
  }

  async function handleLogin() {
    if (!config || !currentMethod) return;
    error = "";
    captchaWarning = false;
    loading = true;

    try {
      const args: Record<string, any> = {};

      if (currentMethod.method_type === "email_password") {
        args.email = email;
        args.password = password;
      } else if (currentMethod.method_type === "email_only") {
        args.email = email;
      } else if (currentMethod.method_type === "token") {
        args.token = token;
      } else if (currentMethod.method_type === "cookies") {
        if (currentMethod.command.includes("udemy")) {
          args.cookieJson = cookiesText;
        } else {
          args.cookies = cookiesText;
        }
      }

      for (const field of currentMethod.extra_fields) {
        args[field.key] = extraFields[field.key] ?? "";
      }

      const result = await pluginInvoke<string>("courses", currentMethod.command, args);
      sessionEmail = result || email || "OK";
      loggedIn = true;
      loadItems();
    } catch (e: any) {
      error = typeof e === "string" ? e : e.message ?? $t("common.error");
    } finally {
      loading = false;
    }
  }

  async function handleLogout() {
    if (!config) return;
    try {
      await pluginInvoke("courses", config.commands.logout);
    } catch {}
    loggedIn = false;
    sessionEmail = "";
    items = [];
    itemsError = "";
    currentPage = 1;
    searchQuery = "";
  }

  async function loadItems() {
    if (!config) return;
    loadingItems = true;
    itemsError = "";
    try {
      items = await pluginInvoke("courses", config.commands.list);
      currentPage = 1;
    } catch (e: any) {
      itemsError = typeof e === "string" ? e : e.message ?? $t("common.error");
    } finally {
      loadingItems = false;
    }
  }

  async function handleSearch() {
    if (!config?.commands.search || !searchQuery.trim()) return;
    loadingItems = true;
    itemsError = "";
    try {
      items = await pluginInvoke("courses", config.commands.search, { query: searchQuery.trim() });
      currentPage = 1;
    } catch (e: any) {
      itemsError = typeof e === "string" ? e : e.message ?? $t("common.error");
    } finally {
      loadingItems = false;
    }
  }

  function getDownloadStatus(item: any): "idle" | "downloading" | "complete" | "error" {
    const id = getItemId(item);
    const dl = downloads.get(id);
    if (!dl) return "idle";
    const s = dl.status;
    if (s === "queued" || s === "paused" || s === "seeding") return "downloading";
    return s;
  }

  function getDownloadPercent(item: any): number {
    return downloads.get(getItemId(item))?.percent ?? 0;
  }

  async function downloadItem(item: any) {
    if (!config) return;
    const status = getDownloadStatus(item);
    if (status === "downloading") {
      showToast("info", $t("toast.download_already_active"));
      return;
    }
    if (status === "complete") return;

    const appSettings = getSettings();
    let outputDir: string | null = null;

    if (appSettings?.download.always_ask_path) {
      outputDir = await open({ directory: true, title: $t("hotmart.choose_folder") }) as string | null;
      if (!outputDir) return;
    } else {
      outputDir = appSettings?.download.default_output_dir ?? null;
      if (!outputDir) {
        outputDir = await open({ directory: true, title: $t("hotmart.choose_folder") }) as string | null;
        if (!outputDir) return;
      }
    }

    const argName = config.features.download_arg_name ?? "courseJson";
    try {
      await pluginInvoke("courses", config.commands.download, {
        [argName]: JSON.stringify(item),
        outputDir,
      });
      showToast("info", $t("toast.download_preparing"));
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  let refreshing = $state(false);

  async function refreshItems() {
    if (!config) return;
    refreshing = true;
    loadingItems = true;
    itemsError = "";
    try {
      items = await pluginInvoke("courses", config.commands.refresh);
      currentPage = 1;
    } catch (e: any) {
      itemsError = typeof e === "string" ? e : e.message ?? $t("common.error");
    } finally {
      loadingItems = false;
      refreshing = false;
    }
  }

  function handleFileUpload(event: Event) {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      if (currentMethod?.method_type === "cookies") {
        cookiesText = reader.result as string;
      } else {
        token = reader.result as string;
      }
    };
    reader.readAsText(file);
    input.value = "";
  }

  let sessionDisplay = $derived(() => {
    if (!config) return sessionEmail;
    if (config.features.session_display === "platform_name") return config.name;
    if (config.features.session_display === "site_url") return sessionEmail;
    return sessionEmail || "—";
  });
</script>

<a href="/courses" class="back-link">
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M19 12H5" /><path d="M12 19l-7-7 7-7" />
  </svg>
  {$t("courses.back")}
</a>

{#if configError}
  <div class="page-center">
    <p class="error-msg">{configError}</p>
    <a href="/courses" class="button">{$t("courses.back")}</a>
  </div>
{:else if checking}
  <div class="page-center">
    <span class="spinner"></span>
    <span class="spinner-text">{$t("hotmart.checking_session")}</span>
  </div>
{:else if loggedIn && config}
  <div class="page-logged">
    <div class="session-bar">
      <span class="session-info">
        {$t("hotmart.logged_as", { email: sessionDisplay() })}
      </span>
      <div class="session-actions">
        {#if config.features.has_search}
          <div class="search-bar">
            <input
              class="search-input"
              type="text"
              placeholder={$t("courses.search_placeholder")}
              bind:value={searchQuery}
              onkeydown={(e) => { if (e.key === "Enter") handleSearch(); }}
            />
            <button class="button" onclick={handleSearch} disabled={!searchQuery.trim()}>
              {$t("courses.search_btn") ?? "Search"}
            </button>
          </div>
        {/if}
        <button class="button" onclick={refreshItems} disabled={refreshing} aria-label={$t("hotmart.refresh")}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class:spinning={refreshing}>
            <path d="M21 2v6h-6" /><path d="M3 12a9 9 0 0115-6.7L21 8" /><path d="M3 22v-6h6" /><path d="M21 12a9 9 0 01-15 6.7L3 16" />
          </svg>
        </button>
        <button class="button" onclick={handleLogout}>{$t("hotmart.logout")}</button>
      </div>
    </div>

    {#if loadingItems}
      <div class="spinner-section">
        <span class="spinner"></span>
        <span class="spinner-text">{$t("hotmart.loading_courses")}</span>
      </div>
    {:else if itemsError}
      <div class="error-section">
        <p class="error-msg">{itemsError}</p>
        <button class="button" onclick={loadItems}>{$t("common.retry")}</button>
      </div>
    {:else if items.length === 0}
      <p class="empty-text">{$t("hotmart.no_courses")}</p>
    {:else}
      <div class="courses-header">
        <h2>{config.name}</h2>
        <span class="subtext">{items.length === 1 ? $t("hotmart.course_count_one", { count: items.length }) : $t("hotmart.course_count", { count: items.length })}</span>
      </div>

      <div class="courses-grid">
        {#each paginatedItems as item (getItemId(item))}
          <CourseCard
            name={getItemName(item)}
            price={getItemSubtitle(item)}
            imageUrl={getItemImage(item)}
            downloadStatus={getDownloadStatus(item)}
            downloadPercent={getDownloadPercent(item)}
            onDownload={() => downloadItem(item)}
          />
        {/each}
      </div>

      {#if totalPages > 1}
        <div class="pagination">
          <span class="pagination-info">
            {$t("hotmart.page_of", { current: currentPage, total: totalPages })} &middot; {items.length === 1 ? $t("hotmart.course_count_one", { count: items.length }) : $t("hotmart.course_count", { count: items.length })}
          </span>
          <div class="pagination-controls">
            <button class="button pagination-btn" disabled={currentPage <= 1} onclick={() => goToPage(currentPage - 1)}>&lt;</button>
            {#each pageNumbers() as pg}
              {#if pg === -1}
                <span class="pagination-ellipsis">&hellip;</span>
              {:else}
                <button class="button pagination-btn" class:active={pg === currentPage} onclick={() => goToPage(pg)}>{pg}</button>
              {/if}
            {/each}
            <button class="button pagination-btn" disabled={currentPage >= totalPages} onclick={() => goToPage(currentPage + 1)}>&gt;</button>
          </div>
        </div>
      {/if}
    {/if}
  </div>
{:else if config}
  <div class="page-center">
    <div class="login-card">
      <h2>{config.name}</h2>

      {#if config.login_methods.length > 1}
        <div class="login-tabs">
          {#each config.login_methods as method, i}
            <button
              class="login-tab button"
              class:active={loginTabIndex === i}
              onclick={() => { loginTabIndex = i; error = ""; }}
            >
              {#if method.method_type === "email_password"}Email & Password
              {:else if method.method_type === "email_only"}Email
              {:else if method.method_type === "token"}Token
              {:else if method.method_type === "cookies"}Cookies
              {:else}{method.method_type}{/if}
            </button>
          {/each}
        </div>
      {/if}

      {#if currentMethod}
        <form class="form" onsubmit={(e) => { e.preventDefault(); handleLogin(); }}>
          {#each currentMethod.extra_fields as field}
            <label class="field">
              <span class="field-label">{field.label}</span>
              <input
                type={field.field_type === "url" ? "url" : "text"}
                placeholder={field.placeholder}
                bind:value={extraFields[field.key]}
                class="input"
                disabled={loading}
                required
              />
            </label>
          {/each}

          {#if currentMethod.method_type === "email_password"}
            <label class="field">
              <span class="field-label">{$t("hotmart.email_label")}</span>
              <input type="email" placeholder={$t("hotmart.email_placeholder")} bind:value={email} class="input" disabled={loading} required />
            </label>
            <label class="field">
              <span class="field-label">{$t("hotmart.password_label")}</span>
              <input type="password" placeholder={$t("hotmart.password_placeholder")} bind:value={password} class="input" disabled={loading} required />
            </label>
          {:else if currentMethod.method_type === "email_only"}
            <label class="field">
              <span class="field-label">{$t("hotmart.email_label")}</span>
              <input type="email" placeholder={$t("hotmart.email_placeholder")} bind:value={email} class="input" disabled={loading} required />
            </label>
          {:else if currentMethod.method_type === "token"}
            <label class="field">
              <span class="field-label">Token</span>
              <textarea placeholder="Paste token or cookie string..." bind:value={token} class="input token-input" disabled={loading} required></textarea>
            </label>
            <label class="file-upload-label">
              <input type="file" accept=".json,.txt" onchange={handleFileUpload} class="file-input" />
              <span class="button file-btn">{$t("udemy.upload_file") ?? "Upload file"}</span>
            </label>
          {:else if currentMethod.method_type === "cookies"}
            <label class="field">
              <span class="field-label">Cookies JSON</span>
              <textarea placeholder="Paste cookies JSON..." bind:value={cookiesText} class="input token-input" disabled={loading} required></textarea>
            </label>
            <label class="file-upload-label">
              <input type="file" accept=".json,.txt" onchange={handleFileUpload} class="file-input" />
              <span class="button file-btn">{$t("udemy.upload_file") ?? "Upload file"}</span>
            </label>
          {/if}

          {#if error}
            <p class="error-msg">{error}</p>
          {/if}

          {#if captchaWarning}
            <p class="captcha-warning">{$t("hotmart.captcha_detected")}</p>
          {/if}

          <button type="submit" class="button" disabled={loading}>
            {#if loading}
              {$t("hotmart.authenticating")}
            {:else}
              {$t("hotmart.login")}
            {/if}
          </button>
        </form>
      {/if}
    </div>
  </div>
{/if}

<style>
  .back-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    margin-bottom: var(--padding);
  }

  @media (hover: hover) {
    .back-link:hover { color: var(--secondary); }
  }

  .page-center {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: calc(100vh - var(--padding) * 4 - 40px);
    gap: var(--padding);
  }

  .page-logged {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--padding) * 1.5);
    padding: calc(var(--padding) * 1.5);
    width: 100%;
  }

  .page-logged > :global(*) {
    width: 100%;
    max-width: 1200px;
  }

  .session-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--padding);
  }

  .session-info {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .session-actions {
    display: flex;
    gap: calc(var(--padding) / 2);
    align-items: center;
    flex-wrap: wrap;
  }

  .session-bar :global(.button) {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 12.5px;
  }

  .search-bar {
    display: flex;
    gap: calc(var(--padding) / 2);
    align-items: center;
  }

  .search-bar .search-input {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 12.5px;
    background: var(--input-bg);
    border: 1px solid var(--input-border);
    border-radius: var(--border-radius);
    color: var(--secondary);
    outline: none;
    min-width: 180px;
  }

  .search-bar .search-input::placeholder { color: var(--gray); }
  .search-bar .search-input:focus-visible { border-color: var(--secondary); }

  .spinning {
    animation: spin 0.6s linear infinite;
  }

  .courses-header {
    display: flex;
    align-items: baseline;
    gap: var(--padding);
  }

  .courses-header h2 { margin-block: 0; }

  .subtext {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .courses-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--padding);
  }

  @media (max-width: 1000px) { .courses-grid { grid-template-columns: repeat(3, 1fr); } }
  @media (max-width: 750px) { .courses-grid { grid-template-columns: repeat(2, 1fr); } }
  @media (max-width: 535px) { .courses-grid { grid-template-columns: 1fr; } }

  .pagination {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    padding-top: var(--padding);
  }

  .pagination-info {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .pagination-controls {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 3);
  }

  .pagination-btn {
    min-width: 36px;
    height: 36px;
    padding: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 14.5px;
  }

  .pagination-ellipsis {
    min-width: 36px;
    text-align: center;
    color: var(--gray);
    font-size: 14.5px;
  }

  .login-card {
    width: 100%;
    max-width: 400px;
    background: var(--button-elevated);
    border-radius: var(--border-radius);
    padding: calc(var(--padding) * 2);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
  }

  .login-card h2 { margin-block: 0; }

  .login-tabs {
    display: flex;
    gap: 0;
  }

  .login-tab {
    flex: 1;
    border-radius: 0;
    font-size: 12.5px;
    padding: calc(var(--padding) * 0.75) var(--padding);
  }

  .login-tab:first-child {
    border-radius: var(--border-radius) 0 0 var(--border-radius);
  }

  .login-tab:last-child {
    border-radius: 0 var(--border-radius) var(--border-radius) 0;
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 2);
  }

  .field-label {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .input {
    width: 100%;
    padding: var(--padding);
    font-size: 14.5px;
    background: var(--button);
    border-radius: var(--border-radius);
    color: var(--secondary);
    border: 1px solid var(--input-border);
  }

  .input::placeholder { color: var(--gray); }
  .input:focus-visible { border-color: var(--secondary); outline: none; }
  .input:disabled { opacity: 0.5; cursor: default; }

  .token-input {
    min-height: 80px;
    resize: vertical;
    font-family: var(--font-mono);
    font-size: 12.5px;
  }

  .file-upload-label {
    display: flex;
    cursor: pointer;
  }

  .file-input {
    display: none;
  }

  .file-btn {
    font-size: 12.5px;
    cursor: pointer;
  }

  .error-msg {
    color: var(--red);
    font-size: 12.5px;
    font-weight: 500;
  }

  .captcha-warning {
    color: var(--orange);
    font-size: 12.5px;
    font-weight: 500;
  }

  .spinner-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    padding: calc(var(--padding) * 4) 0;
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 2px solid var(--input-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin { to { transform: rotate(360deg); } }

  .spinner-text {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .error-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    padding: calc(var(--padding) * 2) 0;
  }

  .empty-text {
    color: var(--gray);
    font-size: 14.5px;
    text-align: center;
    padding: calc(var(--padding) * 4) 0;
  }
</style>
