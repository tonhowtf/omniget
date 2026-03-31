<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import CourseCard from "$components/hotmart/CourseCard.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getDownloads } from "$lib/stores/download-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { t } from "$lib/i18n";

  let downloads = $derived(getDownloads());

  type AlpaclassCourse = {
    slug: string;
    name: string;
    author: string;
  };

  let token = $state("");
  let platformUrl = $state("");
  let loading = $state(false);
  let error = $state("");

  let fileInput: HTMLInputElement = $state() as HTMLInputElement;

  function onFileSelected(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => { token = reader.result as string; };
    reader.readAsText(file);
    input.value = "";
  }

  let checking = $state(true);
  let loggedIn = $state(false);
  let sessionEmail = $state("");

  let courses: AlpaclassCourse[] = $state([]);
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

  function goToPage(page: number) {
    if (page >= 1 && page <= totalPages) {
      currentPage = page;
    }
  }

  $effect(() => {
    checkSession();
  });

  async function checkSession() {
    try {
      const result = await invoke<string>("alpaclass_check_session");
      sessionEmail = result;
      loggedIn = true;
      loadCourses();
    } catch {
      loggedIn = false;
    } finally {
      checking = false;
    }
  }

  async function handleTokenLogin() {
    if (!token.trim()) return;
    error = "";
    loading = true;
    try {
      const result = await invoke<string>("alpaclass_login", {
        token: token.trim(),
        platformUrl: platformUrl.trim(),
      });
      sessionEmail = result || "Token";
      loggedIn = true;
      loadCourses();
    } catch (e: any) {
      error = typeof e === "string" ? e : e.message ?? $t('hotmart.unknown_error');
    } finally {
      loading = false;
    }
  }

  async function handleLogout() {
    try {
      await invoke("alpaclass_logout");
    } catch {
    }
    loggedIn = false;
    sessionEmail = "";
    courses = [];
    coursesError = "";
    currentPage = 1;
  }

  async function loadCourses() {
    loadingCourses = true;
    coursesError = "";
    try {
      courses = await invoke("alpaclass_list_courses");
      currentPage = 1;
    } catch (e: any) {
      coursesError = typeof e === "string" ? e : e.message ?? $t('hotmart.courses_error');
    } finally {
      loadingCourses = false;
    }
  }

  function getCourseDownloadStatus(_courseSlug: string): "idle" | "downloading" | "complete" | "error" {
    return "idle";
  }

  function getCourseDownloadPercent(_courseSlug: string): number {
    return 0;
  }

  async function downloadCourse(course: AlpaclassCourse) {
    const status = getCourseDownloadStatus(course.slug);
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

    try {
      await invoke("start_alpaclass_course_download", {
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
      courses = await invoke("alpaclass_refresh_courses");
      currentPage = 1;
    } catch (e: any) {
      coursesError = typeof e === "string" ? e : e.message ?? $t('hotmart.courses_error');
    } finally {
      loadingCourses = false;
      refreshing = false;
    }
  }
</script>

<a href="/courses" class="back-link">
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M19 12H5" />
    <path d="M12 19l-7-7 7-7" />
  </svg>
  {$t("courses.back")}
</a>

{#if checking}
  <div class="page-center">
    <span class="spinner"></span>
    <span class="spinner-text">{$t('hotmart.checking_session')}</span>
  </div>
{:else if loggedIn}
  <div class="page-logged">
    <div class="session-bar">
      <span class="session-info">
        {$t('hotmart.logged_as', { email: sessionEmail || "—" })}
      </span>
      <div class="session-actions">
        <button
          class="button"
          onclick={refreshCourses}
          disabled={refreshing}
          aria-label={$t('hotmart.refresh')}
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class:spinning={refreshing}>
            <path d="M21 2v6h-6" />
            <path d="M3 12a9 9 0 0115-6.7L21 8" />
            <path d="M3 22v-6h6" />
            <path d="M21 12a9 9 0 01-15 6.7L3 16" />
          </svg>
        </button>
        <button class="button" onclick={handleLogout}>{$t('hotmart.logout')}</button>
      </div>
    </div>

    {#if loadingCourses}
      <div class="spinner-section">
        <span class="spinner"></span>
        <span class="spinner-text">{$t('hotmart.loading_courses')}</span>
      </div>
    {:else if coursesError}
      <div class="error-section">
        <p class="error-msg">{coursesError}</p>
        <button class="button" onclick={loadCourses}>{$t('common.retry')}</button>
      </div>
    {:else if courses.length === 0}
      <p class="empty-text">{$t('hotmart.no_courses')}</p>
    {:else}
      <div class="courses-header">
        <h2>{$t('hotmart.courses_title')}</h2>
        <span class="subtext">{courses.length === 1 ? $t('hotmart.course_count_one', { count: courses.length }) : $t('hotmart.course_count', { count: courses.length })}</span>
      </div>

      <div class="courses-grid">
        {#each paginatedCourses as course (course.slug)}
          <CourseCard
            name={course.name}
            price={course.author}
            downloadStatus={getCourseDownloadStatus(course.slug)}
            downloadPercent={getCourseDownloadPercent(course.slug)}
            onDownload={() => downloadCourse(course)}
          />
        {/each}
      </div>

      {#if totalPages > 1}
        <div class="pagination">
          <span class="pagination-info">
            {$t('hotmart.page_of', { current: currentPage, total: totalPages })} &middot; {courses.length === 1 ? $t('hotmart.course_count_one', { count: courses.length }) : $t('hotmart.course_count', { count: courses.length })}
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
      <h2>AlpaClass</h2>

      <div class="form">
        <label class="field">
          <span class="field-label">Platform URL</span>
          <input
            type="url"
            placeholder="https://example.alpaclass.com"
            bind:value={platformUrl}
            class="input"
            disabled={loading}
          />
        </label>
        <label class="field">
          <span class="field-label">Cookies JSON</span>
          <textarea
            class="input token-textarea"
            placeholder="Paste cookies JSON from browser extension or a raw token"
            bind:value={token}
            disabled={loading}
            rows="5"
          ></textarea>
        </label>

        <input
          type="file"
          accept=".json,.txt"
          class="hidden-file-input"
          bind:this={fileInput}
          onchange={onFileSelected}
        />
        <button class="button" onclick={() => fileInput?.click()} disabled={loading}>
          Import .json file
        </button>

        {#if error}
          <p class="error-msg">{error}</p>
        {/if}

        <button class="button" onclick={handleTokenLogin} disabled={loading || !token.trim()}>
          {#if loading}
            {$t('hotmart.authenticating')}
          {:else}
            {$t('hotmart.login')}
          {/if}
        </button>
      </div>
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
    .back-link:hover {
      color: var(--secondary);
    }
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

  .hidden-file-input {
    display: none;
  }

  .token-textarea {
    resize: vertical;
    min-height: 80px;
    font-size: 11.5px;
    font-family: var(--font-mono);
    line-height: 1.5;
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
