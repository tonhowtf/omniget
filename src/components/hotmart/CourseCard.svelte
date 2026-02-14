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
        <button class="button" disabled>Baixar Curso</button>
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
    background: var(--button-elevated);
    box-shadow: var(--button-box-shadow);
    border-radius: var(--border-radius);
    overflow: hidden;
  }

  .course-card.expanded {
    box-shadow: 0 0 0 1px var(--blue) inset;
  }

  .card-header {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: calc(var(--padding) + 2px) calc(var(--padding) + 4px);
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    color: var(--secondary);
    gap: var(--padding);
  }

  @media (hover: hover) {
    .card-header:hover {
      background: var(--button-stroke);
    }
  }

  .card-header:active {
    background: var(--button-stroke);
  }

  .card-header:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .header-left {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .course-name {
    font-weight: 500;
    font-size: 14.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .course-seller {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .header-right {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 1.5);
    flex-shrink: 0;
  }

  .badge {
    font-size: 11px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    background: var(--blue);
    color: var(--primary);
    padding: 2px calc(var(--padding) / 2);
    border-radius: calc(var(--border-radius) / 2);
  }

  .chevron {
    font-size: 12px;
    color: var(--gray);
    transition: transform 0.15s;
  }

  .chevron.rotated {
    transform: rotate(180deg);
  }

  .card-body {
    padding: 0 calc(var(--padding) + 4px) calc(var(--padding) + 2px);
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    animation: slideDown 0.15s ease-out;
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
    border-top: 1px solid var(--content-border);
    padding-top: var(--padding);
  }

  .summary {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .body-top .button {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 12.5px;
    opacity: 0.5;
  }

  .spinner-wrap {
    display: flex;
    justify-content: center;
    padding: calc(var(--padding) * 1.5) 0;
  }

  .spinner {
    width: 20px;
    height: 20px;
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

  .error-msg {
    color: var(--red);
    font-size: 12.5px;
    font-weight: 500;
  }

  .empty-msg {
    color: var(--gray);
    font-size: 12.5px;
    font-weight: 500;
    text-align: center;
    padding: var(--padding) 0;
  }
</style>
