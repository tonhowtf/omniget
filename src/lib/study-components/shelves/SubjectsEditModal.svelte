<script lang="ts">
  import { pluginInvoke } from "$lib/plugin-invoke";
  import {
    studySubjectsListForCourse,
    studySubjectsSetForCourse,
    type CourseSubject,
  } from "$lib/study-bridge";

  type Props = {
    courseId: number;
    open: boolean;
    onClose: () => void;
    onSaved?: () => void;
  };

  let { courseId, open, onClose, onSaved }: Props = $props();

  type SubjectRow = { id: number; name: string; color: string | null };

  let allSubjects = $state<SubjectRow[]>([]);
  let selected = $state<Set<number>>(new Set());
  let initial = $state<Set<number>>(new Set());
  let loading = $state(false);
  let saving = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    if (open) {
      void load();
    }
  });

  async function load() {
    loading = true;
    error = null;
    try {
      const [all, current] = await Promise.all([
        pluginInvoke<SubjectRow[]>("study", "study:subjects:list", {}),
        studySubjectsListForCourse(courseId),
      ]);
      allSubjects = all;
      const set = new Set(current.map((s: CourseSubject) => s.id));
      selected = set;
      initial = new Set(set);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function toggle(id: number) {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selected = next;
  }

  const dirty = $derived.by(() => {
    if (selected.size !== initial.size) return true;
    for (const id of selected) {
      if (!initial.has(id)) return true;
    }
    return false;
  });

  async function save() {
    saving = true;
    error = null;
    try {
      await studySubjectsSetForCourse(courseId, [...selected]);
      onSaved?.();
      onClose();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  function onBackdropKey(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
  }
</script>

{#if open}
  <div
    class="backdrop"
    role="dialog"
    aria-modal="true"
    aria-label="Editar matérias"
    tabindex="-1"
    onkeydown={onBackdropKey}
  >
    <button
      type="button"
      class="backdrop-btn"
      aria-label="Fechar"
      onclick={onClose}
    ></button>
    <div class="modal" role="document">
      <header class="head">
        <h2>Matérias do curso</h2>
        <button type="button" class="close" onclick={onClose} aria-label="Fechar">×</button>
      </header>
      <div class="body">
        {#if loading}
          <p class="muted">Carregando…</p>
        {:else if error}
          <p class="error">{error}</p>
        {:else if allSubjects.length === 0}
          <p class="muted">
            Nenhuma matéria criada. Vá para a aba Foco e crie uma primeiro.
          </p>
        {:else}
          <ul class="list" aria-label="Lista de matérias">
            {#each allSubjects as s (s.id)}
              {@const isSelected = selected.has(s.id)}
              <li>
                <label class="row" class:selected={isSelected}>
                  <input
                    type="checkbox"
                    checked={isSelected}
                    onchange={() => toggle(s.id)}
                  />
                  {#if s.color}
                    <span class="dot" style:background={s.color}></span>
                  {/if}
                  <span class="name">{s.name}</span>
                </label>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
      <footer class="foot">
        <button type="button" class="btn ghost" onclick={onClose}>Cancelar</button>
        <button
          type="button"
          class="btn primary"
          disabled={!dirty || saving}
          onclick={save}
        >
          {saving ? "Salvando…" : "Salvar"}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: grid;
    place-items: center;
    background: color-mix(in oklab, black 50%, transparent);
  }

  .backdrop-btn {
    position: absolute;
    inset: 0;
    background: transparent;
    border: none;
    cursor: default;
  }

  .modal {
    position: relative;
    width: min(420px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
    display: flex;
    flex-direction: column;
    background: var(--content-bg);
    border-radius: 14px;
    box-shadow: 0 16px 48px color-mix(in oklab, black 40%, transparent);
    overflow: hidden;
    z-index: 1;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: none;
  }

  .head h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
  }

  .close {
    background: transparent;
    border: none;
    color: inherit;
    font-size: 24px;
    line-height: 1;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 6px;
  }

  .close:hover {
    background: color-mix(in oklab, currentColor 8%, transparent);
  }

  .body {
    padding: 16px 20px;
    overflow-y: auto;
    flex: 1 1 auto;
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: 8px;
    cursor: pointer;
    transition: background var(--tg-duration-fast, 150ms);
  }

  .row:hover {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }

  .row.selected {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
  }

  .dot {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    flex: 0 0 auto;
  }

  .name {
    font-size: 14px;
  }

  .foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 20px;
    border-top: none;
  }

  .btn {
    padding: 8px 16px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    border: none;
    background: transparent;
    color: inherit;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn.primary {
    background: var(--accent);
    color: var(--accent-contrast, white);
    border-color: var(--accent);
  }

  .btn.primary:not(:disabled):hover {
    filter: brightness(1.05);
  }

  .btn.ghost:hover {
    background: color-mix(in oklab, currentColor 8%, transparent);
  }

  .muted {
    color: color-mix(in oklab, currentColor 60%, transparent);
    font-size: 13px;
    margin: 0;
  }

  .error {
    color: var(--error);
    font-size: 13px;
    margin: 0;
  }
</style>
