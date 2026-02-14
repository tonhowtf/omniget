<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import CourseCard from "../../components/hotmart/CourseCard.svelte";

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
  let loggedIn = $state(false);

  let courses: Course[] = $state([]);
  let loadingCourses = $state(false);
  let coursesError = $state("");

  async function handleLogin() {
    error = "";
    loading = true;
    try {
      await invoke("hotmart_login", { email, password });
      loggedIn = true;
      loadCourses();
    } catch (e: any) {
      error = typeof e === "string" ? e : e.message ?? "Erro desconhecido";
    } finally {
      loading = false;
    }
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

<div class="hotmart">
  <h1 class="page-title">Hotmart</h1>

  {#if loggedIn}
    <div class="courses-section">
      {#if loadingCourses}
        <div class="spinner-wrap">
          <span class="spinner"></span>
          <span class="spinner-text">Carregando cursos...</span>
        </div>
      {:else if coursesError}
        <div class="card error-card">
          <p class="error-msg">{coursesError}</p>
          <button class="btn-retry" onclick={loadCourses}>Tentar novamente</button>
        </div>
      {:else if courses.length === 0}
        <div class="card empty">
          <p class="empty-text">Nenhum curso encontrado.</p>
        </div>
      {:else}
        <div class="courses-header">
          <span class="courses-count">{courses.length} {courses.length === 1 ? 'curso' : 'cursos'}</span>
        </div>
        <div class="courses-list">
          {#each courses as course (course.id)}
            <CourseCard {course} />
          {/each}
        </div>
      {/if}
    </div>
  {:else}
    <div class="card">
      <h2 class="card-title">Login</h2>
      <form class="form" onsubmit={(e) => { e.preventDefault(); handleLogin(); }}>
        <label class="field">
          <span class="label">Email</span>
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
          <span class="label">Password</span>
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

        <button type="submit" class="btn" disabled={loading}>
          {#if loading}
            Autenticando...
          {:else}
            Sign in
          {/if}
        </button>
      </form>
    </div>
  {/if}
</div>

<style>
  .hotmart {
    max-width: 580px;
  }

  .page-title {
    font-size: 1.5rem;
    font-weight: 600;
    margin-bottom: 24px;
  }

  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 24px;
    margin-bottom: 16px;
  }

  .card-title {
    font-size: 1rem;
    font-weight: 500;
    margin-bottom: 20px;
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .label {
    font-size: 0.85rem;
    color: var(--text-muted);
  }

  .input {
    width: 100%;
    padding: 10px 14px;
    font-size: 0.9rem;
    background: #2a2a2a;
    border-radius: 8px;
    color: var(--text);
    border: 1px solid var(--border);
    transition: border-color 0.2s;
  }

  .input::placeholder {
    color: var(--text-muted);
  }

  .input:focus {
    border-color: var(--accent);
  }

  .input:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .error-msg {
    color: #ef4444;
    font-size: 0.85rem;
  }

  .btn {
    padding: 10px 20px;
    font-size: 0.9rem;
    font-weight: 500;
    background: var(--accent);
    color: #fff;
    border-radius: 8px;
    transition: background-color 0.2s;
    margin-top: 4px;
  }

  .btn:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .courses-section {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .courses-header {
    display: flex;
    align-items: center;
  }

  .courses-count {
    font-size: 0.85rem;
    color: var(--text-muted);
  }

  .courses-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .spinner-wrap {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 48px 0;
  }

  .spinner {
    width: 28px;
    height: 28px;
    border: 2.5px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .spinner-text {
    font-size: 0.85rem;
    color: var(--text-muted);
  }

  .error-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }

  .btn-retry {
    padding: 8px 16px;
    font-size: 0.84rem;
    font-weight: 500;
    background: transparent;
    color: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 6px;
    cursor: pointer;
    transition: background-color 0.15s;
  }

  .btn-retry:hover {
    background: rgba(46, 134, 193, 0.1);
  }

  .empty {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 120px;
  }

  .empty-text {
    color: var(--text-muted);
    font-size: 0.9rem;
  }
</style>
