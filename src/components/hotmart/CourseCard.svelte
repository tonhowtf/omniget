<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import ModuleList from "./ModuleList.svelte";

  type Course = {
    id: number;
    name: string;
    slug: string | null;
    seller: string;
    subdomain: string | null;
    is_hotmart_club: boolean;
  };

  type Module = {
    id: string;
    name: string;
    pages: { hash: string; name: string; page_type: string }[];
  };

  let { course }: { course: Course } = $props();

  let expanded = $state(false);
  let modules: Module[] = $state([]);
  let loadingModules = $state(false);
  let modulesLoaded = $state(false);
  let modulesError = $state("");

  let totalLessons = $derived(
    modules.reduce((sum, m) => sum + m.pages.length, 0)
  );

  async function toggle() {
    expanded = !expanded;
    if (expanded && !modulesLoaded) {
      await loadModules();
    }
  }

  async function loadModules() {
    if (!course.subdomain) {
      modulesError = "Subdomain indisponivel para este curso.";
      return;
    }
    loadingModules = true;
    modulesError = "";
    try {
      modules = await invoke("hotmart_get_modules", {
        courseId: course.id,
        slug: course.subdomain,
      });
      modulesLoaded = true;
    } catch (e: any) {
      modulesError = typeof e === "string" ? e : e.message ?? "Erro ao carregar modulos";
    } finally {
      loadingModules = false;
    }
  }
</script>

<div class="course-card" class:expanded>
  <button class="card-header" onclick={toggle}>
    <div class="header-left">
      <span class="course-name">{course.name}</span>
      <span class="course-seller">{course.seller}</span>
    </div>
    <div class="header-right">
      {#if course.is_hotmart_club}
        <span class="badge">Club</span>
      {/if}
      <span class="chevron" class:rotated={expanded}>&#9662;</span>
    </div>
  </button>

  {#if expanded}
    <div class="card-body">
      <div class="body-top">
        {#if modulesLoaded}
          <span class="summary">{modules.length} modulos &middot; {totalLessons} aulas</span>
        {/if}
        <button class="btn-download" disabled>Baixar Curso</button>
      </div>

      {#if loadingModules}
        <div class="spinner-wrap">
          <span class="spinner"></span>
        </div>
      {:else if modulesError}
        <p class="error-msg">{modulesError}</p>
      {:else if modules.length > 0}
        <ModuleList {modules} />
      {:else if modulesLoaded}
        <p class="empty-msg">Nenhum modulo encontrado.</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .course-card {
    background: #1e1e1e;
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
    transition: border-color 0.2s;
  }

  .course-card:hover {
    border-color: #333;
  }

  .course-card.expanded {
    border-color: var(--accent);
  }

  .card-header {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    color: var(--text);
    gap: 12px;
  }

  .card-header:hover {
    background: rgba(255, 255, 255, 0.02);
  }

  .header-left {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .course-name {
    font-weight: 600;
    font-size: 0.95rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .course-seller {
    font-size: 0.8rem;
    color: var(--text-muted);
  }

  .header-right {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-shrink: 0;
  }

  .badge {
    font-size: 0.7rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    background: var(--accent);
    color: #fff;
    padding: 3px 8px;
    border-radius: 6px;
  }

  .chevron {
    font-size: 0.75rem;
    color: var(--text-muted);
    transition: transform 0.2s;
  }

  .chevron.rotated {
    transform: rotate(180deg);
  }

  .card-body {
    padding: 0 20px 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    animation: slideDown 0.2s ease-out;
  }

  @keyframes slideDown {
    from {
      opacity: 0;
      transform: translateY(-8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .body-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-top: 4px;
    border-top: 1px solid var(--border);
    padding-top: 12px;
  }

  .summary {
    font-size: 0.8rem;
    color: var(--text-muted);
  }

  .btn-download {
    padding: 6px 14px;
    font-size: 0.8rem;
    font-weight: 500;
    background: var(--accent);
    color: #fff;
    border: none;
    border-radius: 6px;
    cursor: not-allowed;
    opacity: 0.5;
  }

  .spinner-wrap {
    display: flex;
    justify-content: center;
    padding: 20px 0;
  }

  .spinner {
    width: 22px;
    height: 22px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .error-msg {
    color: #ef4444;
    font-size: 0.84rem;
  }

  .empty-msg {
    color: var(--text-muted);
    font-size: 0.84rem;
    text-align: center;
    padding: 12px 0;
  }
</style>
