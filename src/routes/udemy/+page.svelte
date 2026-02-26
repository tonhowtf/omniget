<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import CourseCard from "$components/hotmart/CourseCard.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getDownloads } from "$lib/stores/download-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { t } from "$lib/i18n";

  let downloads = $derived(getDownloads());

  type UdemyCourse = {
    id: number;
    title: string;
    published_title: string;
    url: string | null;
    image_url: string | null;
    num_published_lectures: number | null;
  };

  let email = $state("");
  let loading = $state(false);
  let error = $state("");
  let waitingCode = $state(false);

  let checking = $state(true);
  let loggedIn = $state(false);
  let sessionEmail = $state("");

  let loginMode = $state<"email" | "cookies">("email");
  let cookieJson = $state("");
  let portalName = $state("");

  let courses: UdemyCourse[] = $state([]);
  let loadingCourses = $state(false);
  let coursesError = $state("");

  const ITEMS_PER_PAGE = 12;
  let currentPage = $state(1);

  let totalPages = $derived(Math.max(1, Math.ceil(courses.length / ITEMS_PER_PAGE)));
  let paginatedCourses = $derived(
    courses.slice((currentPage - 1) * ITEMS_PER_PAGE, currentPage * ITEMS_PER_PAGE)
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

  function formatLectures(course: UdemyCourse): string {
    if (course.num_published_lectures === null) return "—";
    return $t('udemy.lectures', { count: course.num_published_lectures });
  }

  function goToPage(page: number) {
    if (page >= 1 && page <= totalPages) {
      currentPage = page;
    }
  }

  $effect(() => {
    checkSession();
    const unlistenCode = listen("udemy-auth-waiting-code", () => {
      waitingCode = true;
    });
    return () => { unlistenCode.then((fn) => fn()); };
  });

  async function checkSession() {
    try {
      const result = await invoke<string>("udemy_check_session");
      sessionEmail = result;
      loggedIn = true;
      portalName = await invoke<string>("udemy_get_portal");
      loadCourses();
    } catch {
      loggedIn = false;
    } finally {
      checking = false;
    }
  }

  async function handleLogin() {
    error = "";
    waitingCode = false;
    loading = true;
    try {
      const result = await invoke<string>("udemy_login", { email });
      sessionEmail = result || email;
      loggedIn = true;
      waitingCode = false;
      portalName = "www";
      loadCourses();
    } catch (e: any) {
      error = typeof e === "string" ? e : e.message ?? $t('udemy.unknown_error');
      waitingCode = false;
    } finally {
      loading = false;
    }
  }

  async function handleCookieLogin() {
    if (!cookieJson.trim()) return;
    error = "";
    loading = true;
    try {
      const result = await invoke<string>("udemy_login_cookies", { cookieJson });
      sessionEmail = result;
      loggedIn = true;
      portalName = await invoke<string>("udemy_get_portal");
      loadCourses();
    } catch (e: any) {
      error = typeof e === "string" ? e : e.message ?? $t('udemy.unknown_error');
    } finally {
      loading = false;
    }
  }

  let fileInput: HTMLInputElement;

  function loadCookieFile() {
    fileInput.click();
  }

  function onFileSelected(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      cookieJson = reader.result as string;
    };
    reader.readAsText(file);
    input.value = "";
  }

  async function handleLogout() {
    try {
      await invoke("udemy_logout");
    } catch {
    }
    loggedIn = false;
    sessionEmail = "";
    portalName = "";
    courses = [];
    coursesError = "";
    currentPage = 1;
  }

  async function loadCourses() {
    loadingCourses = true;
    coursesError = "";
    try {
      courses = await invoke("udemy_list_courses");
      currentPage = 1;
    } catch (e: any) {
      coursesError = typeof e === "string" ? e : e.message ?? $t('udemy.courses_error');
    } finally {
      loadingCourses = false;
    }
  }

  function getCourseDownloadStatus(courseId: number): "idle" | "downloading" | "complete" | "error" {
    const item = downloads.get(courseId);
    if (!item) return "idle";
    const s = item.status;
    if (s === "queued" || s === "paused") return "downloading";
    return s;
  }

  function getCourseDownloadPercent(courseId: number): number {
    return downloads.get(courseId)?.percent ?? 0;
  }

  async function downloadCourse(course: UdemyCourse) {
    const status = getCourseDownloadStatus(course.id);
    if (status === "downloading") {
      showToast("info", $t("toast.download_already_active"));
      return;
    }
    if (status === "complete") return;

    const appSettings = getSettings();
    let outputDir: string | null = null;

    if (appSettings?.download.always_ask_path) {
      outputDir = await open({ directory: true, title: $t("udemy.choose_folder") }) as string | null;
      if (!outputDir) return;
    } else {
      outputDir = appSettings?.download.default_output_dir ?? null;
      if (!outputDir) {
        outputDir = await open({ directory: true, title: $t("udemy.choose_folder") }) as string | null;
        if (!outputDir) return;
      }
    }

    try {
      await invoke("start_udemy_course_download", {
        courseJson: JSON.stringify(course),
        outputDir,
      });
      showToast("info", $t("toast.download_preparing"));
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  let refreshing = $state(false);

  async function refreshCourses() {
    refreshing = true;
    loadingCourses = true;
    coursesError = "";
    try {
      courses = await invoke("udemy_refresh_courses");
      currentPage = 1;
    } catch (e: any) {
      coursesError = typeof e === "string" ? e : e.message ?? $t('udemy.courses_error');
    } finally {
      loadingCourses = false;
      refreshing = false;
    }
  }
</script>

{#if checking}
  <div class="page-center">
    <span class="spinner"></span>
    <span class="spinner-text">{$t('udemy.checking_session')}</span>
  </div>
{:else if loggedIn}
  <div class="page-logged">
    <div class="session-bar">
      <span class="session-info">
        {#if portalName && portalName !== "www"}
          {$t('udemy.logged_as_enterprise', { email: sessionEmail || "—", portal: portalName })}
        {:else}
          {$t('udemy.logged_as', { email: sessionEmail || "—" })}
        {/if}
      </span>
      <div class="session-actions">
        <button
          class="button"
          onclick={refreshCourses}
          disabled={refreshing}
          aria-label={$t('udemy.refresh')}
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class:spinning={refreshing}>
            <path d="M21 2v6h-6" />
            <path d="M3 12a9 9 0 0115-6.7L21 8" />
            <path d="M3 22v-6h6" />
            <path d="M21 12a9 9 0 01-15 6.7L3 16" />
          </svg>
        </button>
        <button class="button" onclick={handleLogout}>{$t('udemy.logout')}</button>
      </div>
    </div>

    {#if loadingCourses}
      <div class="spinner-section">
        <span class="spinner"></span>
        <span class="spinner-text">{$t('udemy.loading_courses')}</span>
      </div>
    {:else if coursesError}
      <div class="error-section">
        <p class="error-msg">{coursesError}</p>
        <button class="button" onclick={loadCourses}>{$t('common.retry')}</button>
      </div>
    {:else if courses.length === 0}
      <p class="empty-text">{$t('udemy.no_courses')}</p>
    {:else}
      <div class="courses-header">
        <h2>{$t('udemy.courses_title')}</h2>
        <span class="subtext">{courses.length === 1 ? $t('udemy.course_count_one', { count: courses.length }) : $t('udemy.course_count', { count: courses.length })}</span>
      </div>

      <div class="courses-grid">
        {#each paginatedCourses as course (course.id)}
          <CourseCard
            name={course.title}
            price={formatLectures(course)}
            imageUrl={course.image_url ?? undefined}
            downloadStatus={getCourseDownloadStatus(course.id)}
            downloadPercent={getCourseDownloadPercent(course.id)}
            onDownload={() => downloadCourse(course)}
          />
        {/each}
      </div>

      {#if totalPages > 1}
        <div class="pagination">
          <span class="pagination-info">
            {$t('udemy.page_of', { current: currentPage, total: totalPages })} &middot; {courses.length === 1 ? $t('udemy.course_count_one', { count: courses.length }) : $t('udemy.course_count', { count: courses.length })}
          </span>
          <div class="pagination-controls">
            <button
              class="button pagination-btn"
              disabled={currentPage <= 1}
              onclick={() => goToPage(currentPage - 1)}
            >
              &lt;
            </button>

            {#each pageNumbers() as page}
              {#if page === -1}
                <span class="pagination-ellipsis">&hellip;</span>
              {:else}
                <button
                  class="button pagination-btn"
                  class:active={page === currentPage}
                  onclick={() => goToPage(page)}
                >
                  {page}
                </button>
              {/if}
            {/each}

            <button
              class="button pagination-btn"
              disabled={currentPage >= totalPages}
              onclick={() => goToPage(currentPage + 1)}
            >
              &gt;
            </button>
          </div>
        </div>
      {/if}
    {/if}
  </div>
{:else}
  <div class="page-center">
    <div class="login-card">
      <h2>{$t('udemy.title')}</h2>

      <div class="login-tabs">
        <button
          class="button login-tab"
          class:active={loginMode === "email"}
          onclick={() => { loginMode = "email"; error = ""; }}
        >
          {$t('udemy.login_email_tab')}
        </button>
        <button
          class="button login-tab"
          class:active={loginMode === "cookies"}
          onclick={() => { loginMode = "cookies"; error = ""; }}
        >
          {$t('udemy.login_cookies_tab')}
        </button>
      </div>

      {#if loginMode === "email"}
        {#if loading && waitingCode}
          <div class="waiting-code">
            <svg viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="var(--blue)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M3 7a2 2 0 012-2h14a2 2 0 012 2v10a2 2 0 01-2 2H5a2 2 0 01-2-2V7z" />
              <path d="M3 7l9 6 9-6" />
            </svg>
            <h3>{$t('udemy.waiting_code_title')}</h3>
            <p class="waiting-code-text">{$t('udemy.waiting_code')}</p>
            <span class="spinner"></span>
          </div>
        {:else}
          <form class="form" onsubmit={(e) => { e.preventDefault(); handleLogin(); }}>
            <label class="field">
              <span class="field-label">{$t('udemy.email_label')}</span>
              <input
                type="email"
                placeholder={$t('udemy.email_placeholder')}
                bind:value={email}
                class="input"
                disabled={loading}
                required
              />
            </label>

            {#if error}
              <p class="error-msg">{error}</p>
            {/if}

            <button type="submit" class="button" disabled={loading}>
              {#if loading}
                {$t('udemy.authenticating')}
              {:else}
                {$t('udemy.login')}
              {/if}
            </button>
          </form>
        {/if}
      {:else}
        <div class="form">
          <p class="cookies-instructions">{$t('udemy.cookies_instructions')}</p>

          <textarea
            class="input cookies-textarea"
            placeholder={$t('udemy.cookies_placeholder')}
            bind:value={cookieJson}
            disabled={loading}
            rows="6"
          ></textarea>

          <input
            type="file"
            accept=".json"
            class="hidden-file-input"
            bind:this={fileInput}
            onchange={onFileSelected}
          />

          <button class="button" onclick={loadCookieFile} disabled={loading}>
            {$t('udemy.cookies_load_file')}
          </button>

          {#if error}
            <p class="error-msg">{error}</p>
          {/if}

          <button class="button" onclick={handleCookieLogin} disabled={loading || !cookieJson.trim()}>
            {#if loading}
              {$t('udemy.authenticating')}
            {:else}
              {$t('udemy.cookies_login')}
            {/if}
          </button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .page-center {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: calc(100vh - var(--padding) * 4);
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
  }

  .session-info {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .session-actions {
    display: flex;
    gap: calc(var(--padding) / 2);
  }

  .session-bar :global(.button) {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 12.5px;
  }

  .spinning {
    animation: spin 0.6s linear infinite;
  }

  .courses-header {
    display: flex;
    align-items: baseline;
    gap: var(--padding);
  }

  .courses-header h2 {
    margin-block: 0;
  }

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

  @media (max-width: 1000px) {
    .courses-grid {
      grid-template-columns: repeat(3, 1fr);
    }
  }

  @media (max-width: 750px) {
    .courses-grid {
      grid-template-columns: repeat(2, 1fr);
    }
  }

  @media (max-width: 535px) {
    .courses-grid {
      grid-template-columns: 1fr;
    }
  }

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

  .login-card h2 {
    margin-block: 0;
  }

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

  .cookies-instructions {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    line-height: 1.6;
  }

  .cookies-textarea {
    resize: vertical;
    min-height: 100px;
    font-size: 12px;
    line-height: 1.5;
  }

  .hidden-file-input {
    display: none;
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

  .input::placeholder {
    color: var(--gray);
  }

  .input:focus-visible {
    border-color: var(--secondary);
    outline: none;
  }

  .input:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .error-msg {
    color: var(--red);
    font-size: 12.5px;
    font-weight: 500;
  }

  .waiting-code {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    padding: var(--padding) 0;
    text-align: center;
  }

  .waiting-code h3 {
    margin-block: 0;
  }

  .waiting-code-text {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    line-height: 1.6;
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

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

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
