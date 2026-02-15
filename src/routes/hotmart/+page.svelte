<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import CourseCard from "$components/hotmart/CourseCard.svelte";

  type Course = {
    id: number;
    name: string;
    slug: string | null;
    seller: string;
    subdomain: string | null;
    is_hotmart_club: boolean;
    price: number | null;
    image_url: string | null;
    category: string | null;
    external_platform: boolean;
  };

  let email = $state("");
  let password = $state("");
  let loading = $state(false);
  let error = $state("");

  let checking = $state(true);
  let loggedIn = $state(false);
  let sessionEmail = $state("");

  let courses: Course[] = $state([]);
  let loadingCourses = $state(false);
  let coursesError = $state("");

  let debugOutput = $state("");
  let debugLoading = $state(false);

  // Pagination
  const ITEMS_PER_PAGE = 12;
  let currentPage = $state(1);

  let totalPages = $derived(Math.max(1, Math.ceil(courses.length / ITEMS_PER_PAGE)));
  let paginatedCourses = $derived(
    courses.slice((currentPage - 1) * ITEMS_PER_PAGE, currentPage * ITEMS_PER_PAGE)
  );

  // Generate page numbers for pagination display
  let pageNumbers = $derived((): number[] => {
    const pages: number[] = [];
    if (totalPages <= 7) {
      for (let i = 1; i <= totalPages; i++) pages.push(i);
    } else {
      pages.push(1);
      if (currentPage > 3) pages.push(-1); // ellipsis
      const start = Math.max(2, currentPage - 1);
      const end = Math.min(totalPages - 1, currentPage + 1);
      for (let i = start; i <= end; i++) pages.push(i);
      if (currentPage < totalPages - 2) pages.push(-1); // ellipsis
      pages.push(totalPages);
    }
    return pages;
  });

  function formatPrice(price: number | null): string {
    if (price === null || price === undefined) return "—";
    if (price === 0) return "Gratuito";
    return `R$ ${price.toFixed(2).replace(".", ",")}`;
  }

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
      const result = await invoke<string>("hotmart_check_session");
      sessionEmail = result;
      loggedIn = true;
      loadCourses();
    } catch {
      loggedIn = false;
    } finally {
      checking = false;
    }
  }

  async function handleLogin() {
    error = "";
    loading = true;
    try {
      const result = await invoke<string>("hotmart_login", { email, password });
      sessionEmail = result || email;
      loggedIn = true;
      loadCourses();
    } catch (e: any) {
      error = typeof e === "string" ? e : e.message ?? "Erro desconhecido";
    } finally {
      loading = false;
    }
  }

  async function handleLogout() {
    try {
      await invoke("hotmart_logout");
    } catch {
      /* ignore */
    }
    loggedIn = false;
    sessionEmail = "";
    courses = [];
    coursesError = "";
    currentPage = 1;
    debugOutput = "";
  }

  async function loadCourses() {
    loadingCourses = true;
    coursesError = "";
    try {
      courses = await invoke("hotmart_list_courses");
      currentPage = 1;
    } catch (e: any) {
      coursesError = typeof e === "string" ? e : e.message ?? "Erro ao carregar cursos";
    } finally {
      loadingCourses = false;
    }
  }

  let downloadingIds: Set<number> = $state(new Set());

  async function downloadCourse(course: Course) {
    const selected = await open({ directory: true, title: "Escolher pasta de download" });
    if (!selected) return;

    downloadingIds.add(course.id);
    downloadingIds = new Set(downloadingIds);

    try {
      await invoke("start_course_download", {
        courseJson: JSON.stringify(course),
        outputDir: selected,
      });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? "Erro ao iniciar download";
      console.error(msg);
    }
  }

  async function handleDebugAuth() {
    debugLoading = true;
    debugOutput = "";
    try {
      const result = await invoke<string>("hotmart_debug_auth");
      debugOutput = result;
    } catch (e: any) {
      debugOutput = `ERRO: ${typeof e === "string" ? e : e.message ?? "Erro desconhecido"}`;
    } finally {
      debugLoading = false;
    }
  }
</script>

{#if checking}
  <!-- Estado 1: Verificando sessão -->
  <div class="page-center">
    <span class="spinner"></span>
    <span class="spinner-text">Verificando sessão...</span>
  </div>
{:else if loggedIn}
  <!-- Estado 3/4: Logado -->
  <div class="page-logged">
    <div class="session-bar">
      <span class="session-info">
        Logado como {sessionEmail || "—"}
      </span>
      <div class="session-actions">
        <button
          class="button"
          onclick={handleDebugAuth}
          disabled={debugLoading}
        >
          {debugLoading ? "Debugando..." : "Debug Auth"}
        </button>
        <button class="button" onclick={handleLogout}>Sair</button>
      </div>
    </div>

    {#if debugOutput}
      <pre class="debug-output">{debugOutput}</pre>
    {/if}

    {#if loadingCourses}
      <!-- Estado 3: Carregando cursos -->
      <div class="spinner-section">
        <span class="spinner"></span>
        <span class="spinner-text">Carregando cursos...</span>
      </div>
    {:else if coursesError}
      <div class="error-section">
        <p class="error-msg">{coursesError}</p>
        <button class="button" onclick={loadCourses}>Tentar novamente</button>
      </div>
    {:else if courses.length === 0}
      <p class="empty-text">Nenhum curso encontrado nesta conta.</p>
    {:else}
      <!-- Estado 4: Cursos carregados -->
      <div class="courses-header">
        <h2>Seus Cursos</h2>
        <span class="subtext">{courses.length} {courses.length === 1 ? "curso" : "cursos"}</span>
      </div>

      <div class="courses-grid">
        {#each paginatedCourses as course (course.id)}
          <CourseCard
            name={course.name}
            price={formatPrice(course.price)}
            imageUrl={course.image_url ?? undefined}
            externalPlatform={course.external_platform}
            onDownload={() => downloadCourse(course)}
          />
        {/each}
      </div>

      <!-- Pagination -->
      {#if totalPages > 1}
        <div class="pagination">
          <span class="pagination-info">
            Página {currentPage} de {totalPages} &middot; {courses.length} cursos
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
  <!-- Estado 2: Não logado -->
  <div class="page-center">
    <div class="login-card">
      <h2>Hotmart</h2>
      <form class="form" onsubmit={(e) => { e.preventDefault(); handleLogin(); }}>
        <label class="field">
          <span class="field-label">Email</span>
          <input
            type="email"
            placeholder="you@example.com"
            bind:value={email}
            class="input"
            disabled={loading}
            required
          />
        </label>
        <label class="field">
          <span class="field-label">Senha</span>
          <input
            type="password"
            placeholder="Sua senha"
            bind:value={password}
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
            Autenticando...
          {:else}
            Entrar
          {/if}
        </button>
      </form>
    </div>
  </div>
{/if}

<style>
  /* === State 1: Checking / Center layout === */
  .page-center {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: calc(100vh - var(--padding) * 4);
    gap: var(--padding);
  }

  /* === State 3/4: Logged in layout === */
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

  /* === Session bar === */
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

  /* === Debug output === */
  .debug-output {
    background: var(--button);
    border-radius: var(--border-radius);
    padding: var(--padding);
    font-size: 11px;
    color: var(--secondary);
    white-space: pre-wrap;
    word-break: break-all;
    user-select: text;
    max-height: 200px;
    overflow-y: auto;
    box-shadow: var(--button-box-shadow);
  }

  /* === Courses header === */
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

  /* === Courses grid (responsive) === */
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

  /* === Pagination === */
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

  /* === Login card === */
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

  /* === Shared === */
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
