<script lang="ts">
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import DialogContainer from "$components/dialog/DialogContainer.svelte";

  type Section = {
    id: number;
    index: number;
    title: string;
    lecture_count: number;
    video_count: number;
    drm_video_count: number;
  };

  type Curriculum = {
    course_id: number;
    title: string;
    total_lectures: number;
    sections: Section[];
  };

  let {
    courseId = $bindable<number | string | null>(null),
    courseName = "",
    command,
    onConfirm,
  }: {
    courseId?: number | string | null;
    courseName?: string;
    command: string;
    onConfirm: (sectionIds: number[]) => void;
  } = $props();

  let isOpen = $derived(courseId !== null);
  let loading = $state(false);
  let error = $state("");
  let curriculum = $state<Curriculum | null>(null);
  let selected = $state<Set<number>>(new Set());
  let loadedFor = $state<number | string | null>(null);

  let selectedCount = $derived(selected.size);
  let selectedLectures = $derived(
    curriculum?.sections.filter((s) => selected.has(s.id)).reduce((n, s) => n + s.lecture_count, 0) ?? 0,
  );
  let allSelected = $derived(curriculum !== null && selectedCount === curriculum.sections.length);

  $effect(() => {
    if (courseId !== null && courseId !== loadedFor) {
      loadedFor = courseId;
      void load(courseId);
    }
  });

  async function load(id: number | string) {
    loading = true;
    error = "";
    curriculum = null;
    selected = new Set();
    try {
      const result = await pluginInvoke<Curriculum>("courses", command, { courseId: Number(id) });
      if (courseId !== id) return;
      curriculum = result;
      selected = new Set(result.sections.map((s) => s.id));
    } catch (e: unknown) {
      error = typeof e === "string" ? e : e instanceof Error ? e.message : ($t("common.error") as string);
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

  function setAll(on: boolean) {
    selected = on ? new Set(curriculum?.sections.map((s) => s.id) ?? []) : new Set();
  }

  function close() {
    courseId = null;
    loadedFor = null;
    curriculum = null;
    error = "";
  }

  function confirm() {
    if (!curriculum || selectedCount === 0) return;
    const ids = allSelected ? [] : curriculum.sections.filter((s) => selected.has(s.id)).map((s) => s.id);
    onConfirm(ids);
    close();
  }
</script>

<DialogContainer bind:isOpen onClose={close} titleId="course-sections-title">
  <div class="sections-header">
    <div class="sections-heading">
      <h2 id="course-sections-title">{$t("courses.sections_title")}</h2>
      {#if courseName}<p class="sections-course" title={courseName}>{courseName}</p>{/if}
    </div>
    <button class="dialog-close" onclick={close} aria-label={$t("common.close")}>
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M18 6L6 18M6 6l12 12" />
      </svg>
    </button>
  </div>

  <div class="sections-body">
    {#if loading}
      <p class="sections-state" role="status">
        <span class="spinner" aria-hidden="true"></span>
        {$t("courses.sections_loading")}
      </p>
    {:else if error}
      <div class="sections-state error" role="alert">
        <span>{$t("courses.sections_load_failed")}</span>
        <code>{error}</code>
        <button class="button" onclick={() => courseId !== null && load(courseId)}>{$t("common.retry")}</button>
      </div>
    {:else if curriculum}
      <div class="sections-toolbar">
        <span class="sections-summary">
          {$t("courses.sections_selected", { sections: selectedCount, total: curriculum.sections.length, lectures: selectedLectures })}
        </span>
        <button class="link-btn" onclick={() => setAll(!allSelected)}>
          {allSelected ? $t("courses.sections_select_none") : $t("courses.sections_select_all")}
        </button>
      </div>
      <ul class="sections-list">
        {#each curriculum.sections as section (section.id)}
          <li>
            <label class="section-row" class:checked={selected.has(section.id)}>
              <input type="checkbox" checked={selected.has(section.id)} onchange={() => toggle(section.id)} />
              <span class="section-index">{section.index}</span>
              <span class="section-text">
                <span class="section-title">{section.title}</span>
                <span class="section-meta">
                  {$t("courses.sections_lectures", { count: section.lecture_count })}
                  {#if section.drm_video_count > 0}
                    · <span class="section-drm">{$t("courses.sections_drm", { count: section.drm_video_count })}</span>
                  {/if}
                </span>
              </span>
            </label>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <div class="sections-footer">
    <button class="btn-secondary" onclick={close}>{$t("common.cancel")}</button>
    <button class="btn-primary" onclick={confirm} disabled={loading || !curriculum || selectedCount === 0}>
      {selectedCount === 0
        ? $t("courses.sections_none_selected")
        : allSelected
          ? $t("courses.sections_download_all")
          : $t("courses.sections_download", { count: selectedCount })}
    </button>
  </div>
</DialogContainer>

<style>
  .sections-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--padding);
    padding: 16px 20px 4px;
  }

  .sections-heading {
    min-width: 0;
  }

  .sections-header h2 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--secondary);
  }

  .sections-course {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    color: var(--tertiary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dialog-close {
    background: transparent;
    border: none;
    color: var(--tertiary);
    padding: 4px;
    cursor: pointer;
    border-radius: var(--border-radius);
    flex-shrink: 0;
  }

  .dialog-close:hover {
    background: var(--button-hover);
    color: var(--secondary);
  }

  .sections-body {
    padding: 8px 20px 12px;
    overflow-y: auto;
    max-height: 60vh;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .sections-state {
    margin: 0;
    padding: 24px 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    color: var(--tertiary);
    font-size: var(--text-sm);
    text-align: center;
  }

  .sections-state.error code {
    font-size: var(--text-xs, 12px);
    color: var(--gray);
    background: var(--button);
    padding: 6px 8px;
    border-radius: var(--border-radius);
    max-width: 100%;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .sections-toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--padding);
    font-size: var(--text-sm);
    color: var(--tertiary);
  }

  .link-btn {
    background: transparent;
    border: none;
    color: var(--secondary);
    cursor: pointer;
    padding: 4px 6px;
    border-radius: var(--border-radius);
    font-size: var(--text-sm);
  }

  .link-btn:hover {
    background: var(--button-hover);
  }

  .sections-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .section-row {
    display: grid;
    grid-template-columns: auto auto 1fr;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: var(--border-radius);
    background: var(--button);
    cursor: pointer;
  }

  .section-row:hover {
    background: var(--button-hover);
  }

  .section-row.checked {
    outline: 1px solid var(--accent, var(--secondary));
    outline-offset: -1px;
  }

  .section-row input {
    accent-color: var(--accent, var(--secondary));
    margin: 0;
  }

  .section-index {
    font-size: var(--text-xs, 12px);
    font-variant-numeric: tabular-nums;
    color: var(--gray);
    min-width: 2ch;
    text-align: right;
  }

  .section-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .section-title {
    font-size: var(--text-sm);
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .section-meta {
    font-size: var(--text-xs, 12px);
    color: var(--gray);
  }

  .section-drm {
    color: var(--warning);
  }

  .sections-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 8px 20px 16px;
  }

  .btn-secondary,
  .btn-primary {
    border: none;
    border-radius: var(--border-radius);
    padding: 8px 14px;
    font-size: var(--text-sm);
    cursor: pointer;
  }

  .btn-secondary {
    background: var(--button);
    color: var(--secondary);
  }

  .btn-secondary:hover {
    background: var(--button-hover);
  }

  .btn-primary {
    background: var(--secondary);
    color: var(--primary);
    font-weight: 500;
  }

  .btn-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--gray);
    border-top-color: var(--secondary);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
