<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import CourseCard from "$components/hotmart/CourseCard.svelte";

  type Course = {
    id: number;
    name: string;
    slug: string | null;
    seller: string;
    subdomain: string | null;
    is_hotmart_club: boolean;
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
  }

  async function loadCourses() {
    loadingCourses = true;
    coursesError = "";
    try {
      courses = await invoke("hotmart_list_courses");
    } catch (e: any) {
      coursesError = typeof e === "string" ? e : e.message ?? "Erro ao carregar cursos";
    } finally {
      loadingCourses = false;
    }
  }
</script>

{#if checking}
  <div class="page-center">
    <span class="spinner"></span>
  </div>
{:else if loggedIn}
  <div class="page-scroll">
    <div class="session-bar">
      <span class="session-info">
        {sessionEmail ? `Logado como ${sessionEmail}` : "Logado"}
      </span>
      <button class="button" onclick={handleLogout}>Sair</button>
    </div>

    {#if loadingCourses}
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
      <p class="empty-text">Nenhum curso encontrado.</p>
    {:else}
      <p class="courses-count">{courses.length} {courses.length === 1 ? 'curso' : 'cursos'}</p>
      <div class="courses-list">
        {#each courses as course (course.id)}
          <CourseCard {course} />
        {/each}
      </div>
    {/if}
  </div>
{:else}
  <div class="page-center">
    <div class="login-card">
      <h2>Login</h2>
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
          <span class="field-label">Password</span>
          <input
            type="password"
            placeholder="Your password"
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
            Sign in
          {/if}
        </button>
      </form>
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
  }

  .page-scroll {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    padding-top: var(--padding);
  }

  .page-scroll > :global(*) {
    width: 100%;
    max-width: 580px;
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

  .session-bar .button {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 12.5px;
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

  .courses-count {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .courses-list {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 1.5);
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
