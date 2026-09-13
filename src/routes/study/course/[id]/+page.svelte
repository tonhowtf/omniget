<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";
  import PageHero from "$lib/study-components/PageHero.svelte";
  import ProgressRing from "$lib/study-components/ProgressRing.svelte";
  import { renderMarkdownSync } from "$lib/study-markdown";
  import {
    studySettingsGet,
    type StudySettings,
  } from "$lib/study-bridge";
  import SubjectsEditModal from "$lib/study-components/shelves/SubjectsEditModal.svelte";
  import NotificationsInlinePanel from "$lib/study-components/shelves/NotificationsInlinePanel.svelte";
  import CourseRecommendationsShelf from "$lib/study-components/shelves/CourseRecommendationsShelf.svelte";
  import {
    studySubjectsListForCourse,
    type CourseSubject,
  } from "$lib/study-bridge";

  type Lesson = {
    id: number;
    position: number;
    title: string;
    video_path: string;
    subtitle_path: string | null;
    duration_ms: number | null;
    current_seconds: number;
    completed: boolean;
  };
  type Module = {
    id: number;
    position: number;
    title: string;
    source_path: string;
    lessons: Lesson[];
  };
  type CourseDetail = {
    course: {
      id: number;
      title: string;
      source_path: string;
      thumbnail_path: string | null;
      added_at: string;
      last_scan_at: string;
    };
    modules: Module[];
    loose_lessons: Lesson[];
  };

  const courseId = $derived(Number($page.params.id));

  let detail = $state<CourseDetail | null>(null);
  let loading = $state(true);
  let error = $state("");
  let lessonSearch = $state("");

  let tags = $state<string[]>([]);
  let newTag = $state("");
  let tagSuggestions = $state<{ tag: string; course_count: number }[]>([]);
  let tagInputRef: HTMLInputElement | null = $state(null);
  let courseSubjects = $state<CourseSubject[]>([]);
  let subjectsModalOpen = $state(false);
  let heroBlurIntensity = $state(40);
  let description = $state<{ raw: string; format: "md" | "html" } | null>(null);
  let descriptionExpanded = $state(false);
  const mdCache = new Map<string, string>();

  async function loadDescription() {
    try {
      const detail = await pluginInvoke<{
        description_raw?: string | null;
        description_format?: "md" | "html" | null;
      }>("study", "study:course:detail", { courseId });
      if (detail.description_raw && detail.description_format) {
        description = {
          raw: detail.description_raw,
          format: detail.description_format,
        };
      } else {
        description = null;
      }
    } catch (e) {
      console.error("loadDescription failed", e);
      description = null;
    }
  }

  async function loadHeroBlur() {
    try {
      const settings: StudySettings = await studySettingsGet();
      heroBlurIntensity = settings.player?.hero_blur_intensity ?? 40;
    } catch {
      void 0;
    }
  }

  const heroBackgroundImage = $derived.by(() => {
    if (!detail?.course.thumbnail_path) return null;
    try {
      return convertFileSrc(detail.course.thumbnail_path);
    } catch {
      return null;
    }
  });

  async function loadCourseSubjects() {
    try {
      courseSubjects = await studySubjectsListForCourse(courseId);
    } catch (e) {
      console.error("loadCourseSubjects failed", e);
    }
  }
  let probing = $state(false);
  let probeReport = $state<{
    probed: number;
    skipped: number;
    failed: number;
    total_lessons: number;
  } | null>(null);
  let toast = $state<{ kind: "ok" | "err"; msg: string } | null>(null);

  function showToast(kind: "ok" | "err", msg: string) {
    toast = { kind, msg };
    setTimeout(() => (toast = null), 2400);
  }

  async function loadTags() {
    try {
      const [my, all] = await Promise.all([
        pluginInvoke<string[]>("study", "study:courses:tags:list", {
          courseId,
        }),
        pluginInvoke<{ tag: string; course_count: number }[]>(
          "study",
          "study:courses:tags:list_all",
        ),
      ]);
      tags = my;
      tagSuggestions = all.filter((s) => !my.includes(s.tag)).slice(0, 12);
    } catch (e) {
      console.error("loadTags failed", e);
    }
  }

  async function load() {
    loading = true;
    try {
      detail = await pluginInvoke<CourseDetail>("study", "study:courses:get", {
        courseId,
      });
      await Promise.all([
        loadTags(),
        loadCourseSubjects(),
        loadHeroBlur(),
        loadDescription(),
      ]);
      try {
        await pluginInvoke("study", "study:courses:recents:touch", {
          courseId,
        });
      } catch (e) {
        console.error("recents:touch failed", e);
      }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function addTag(tag: string) {
    const t = tag.trim();
    if (!t) return;
    try {
      await pluginInvoke("study", "study:courses:tags:add", { courseId, tag: t });
      newTag = "";
      await loadTags();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  async function removeTag(tag: string) {
    try {
      await pluginInvoke("study", "study:courses:tags:remove", {
        courseId,
        tag,
      });
      await loadTags();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  function onTagKey(e: KeyboardEvent) {
    if (e.key === "Enter" || e.key === ",") {
      e.preventDefault();
      addTag(newTag);
    } else if (
      e.key === "Backspace" &&
      newTag.length === 0 &&
      tags.length > 0
    ) {
      e.preventDefault();
      removeTag(tags[tags.length - 1]);
    }
  }

  async function probeDurations() {
    probing = true;
    probeReport = null;
    try {
      const r = await pluginInvoke<{
        probed: number;
        skipped: number;
        failed: number;
        total_lessons: number;
      }>("study", "study:courses:probe_durations", { courseId });
      probeReport = r;
      if (r.probed > 0) {
        showToast(
          "ok",
          r.probed === 1
            ? $t("study.course.dur_one", { n: 1 })
            : `${r.probed} durações detectadas`,
        );
        await load();
      } else if (r.failed > 0 && r.probed === 0) {
        showToast("err", $t("study.course.ffprobe_failed"));
      } else {
        showToast("ok", $t("study.course.durations_all_set"));
      }
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      probing = false;
    }
  }

  function fmtDuration(ms: number | null): string {
    if (!ms) return "";
    const totalSec = Math.floor(ms / 1000);
    const h = Math.floor(totalSec / 3600);
    const m = Math.floor((totalSec % 3600) / 60);
    if (h > 0) return `${h}h${m.toString().padStart(2, "0")}`;
    return `${m}m`;
  }

  const totalDurationMs = $derived.by(() => {
    return allLessons.reduce((sum, l) => sum + (l.duration_ms ?? 0), 0);
  });

  onMount(load);

  const allLessons = $derived.by(() => {
    if (!detail) return [] as Lesson[];
    const out: Lesson[] = [];
    for (const m of detail.modules) out.push(...m.lessons);
    out.push(...detail.loose_lessons);
    return out;
  });

  const stats = $derived.by(() => {
    const total = allLessons.length;
    const completed = allLessons.filter((l) => l.completed).length;
    const pct = total > 0 ? Math.round((completed / total) * 100) : 0;
    return { total, completed, pct };
  });

  const filteredDetail = $derived.by(() => {
    if (!detail) return null;
    const q = lessonSearch.trim().toLowerCase();
    if (!q) return detail;
    const match = (l: Lesson) => l.title.toLowerCase().includes(q);
    return {
      ...detail,
      modules: detail.modules
        .map((m) => ({ ...m, lessons: m.lessons.filter(match) }))
        .filter((m) => m.lessons.length > 0),
      loose_lessons: detail.loose_lessons.filter(match),
    };
  });

  const nextLesson = $derived.by(() => {
    const inProgress = allLessons.find((l) => !l.completed && l.current_seconds > 0);
    if (inProgress) return inProgress;
    return allLessons.find((l) => !l.completed) ?? allLessons[0] ?? null;
  });

  function modulePct(m: Module): number {
    if (m.lessons.length === 0) return 0;
    const done = m.lessons.filter((l) => l.completed).length;
    return Math.round((done / m.lessons.length) * 100);
  }

  type LessonState = "todo" | "doing" | "done";
  function lessonState(l: Lesson): LessonState {
    if (l.completed) return "done";
    if (l.current_seconds > 0) return "doing";
    return "todo";
  }

  let selectedLessons = $state<Set<number>>(new Set());
  let lastSelectedId: number | null = null;

  function toggleSelection(lesson: Lesson, ev: MouseEvent) {
    if (ev.shiftKey && lastSelectedId !== null) {
      const ids = allLessons.map((l) => l.id);
      const start = ids.indexOf(lastSelectedId);
      const end = ids.indexOf(lesson.id);
      if (start >= 0 && end >= 0) {
        const [lo, hi] = start < end ? [start, end] : [end, start];
        const next = new Set(selectedLessons);
        for (let i = lo; i <= hi; i++) next.add(ids[i]);
        selectedLessons = next;
      }
    } else {
      const next = new Set(selectedLessons);
      if (next.has(lesson.id)) next.delete(lesson.id);
      else next.add(lesson.id);
      selectedLessons = next;
    }
    lastSelectedId = lesson.id;
  }

  function clearSelection() {
    selectedLessons = new Set();
    lastSelectedId = null;
  }

  function selectAllVisible() {
    selectedLessons = new Set(allLessons.map((l) => l.id));
  }

  async function bulkMark(completed: boolean) {
    if (selectedLessons.size === 0) return;
    const ids = [...selectedLessons];
    try {
      await pluginInvoke("study", "study:progress:mark_lessons_bulk", {
        lessonIds: ids,
        completed,
      });
      if (detail) {
        const update = (l: Lesson) =>
          selectedLessons.has(l.id) ? { ...l, completed } : l;
        detail = {
          ...detail,
          modules: detail.modules.map((m) => ({
            ...m,
            lessons: m.lessons.map(update),
          })),
          loose_lessons: detail.loose_lessons.map(update),
        };
      }
      const status = completed ? "completas" : "incompletas";
      showToast(
        "ok",
        ids.length === 1
          ? `1 aula marcada como ${completed ? "completa" : "incompleta"}`
          : `${ids.length} aulas marcadas como ${status}`,
      );
      clearSelection();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  async function toggleCompleted(lesson: Lesson, ev: MouseEvent) {
    ev.stopPropagation();
    ev.preventDefault();
    const next = !lesson.completed;
    try {
      await pluginInvoke("study", "study:progress:mark_lesson", {
        lessonId: lesson.id,
        seconds: lesson.current_seconds,
        completed: next,
      });
      if (!detail) return;
      const apply = (l: Lesson) =>
        l.id === lesson.id ? { ...l, completed: next } : l;
      detail = {
        ...detail,
        modules: detail.modules.map((m) => ({
          ...m,
          lessons: m.lessons.map(apply),
        })),
        loose_lessons: detail.loose_lessons.map(apply),
      };
    } catch (e) {
      console.error("mark_lesson failed", e);
    }
  }
</script>

<section class="study-page">
  {#if loading}
    <p class="muted">{$t("study.common.loading")}</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if detail}
    <PageHero
      breadcrumb={[
        { label: $t("study.hub.library"), href: "/study/library" },
        { label: detail.course.title },
      ]}
      title={detail.course.title}
      backgroundImage={heroBackgroundImage}
      backgroundBlur={heroBlurIntensity}
      subtitle={$t("study.course.progress", {
        completed: stats.completed,
        total: stats.total,
        pct: stats.pct,
      })}
    >
      {#snippet actions()}
        <div class="hero-actions">
          <ProgressRing value={stats.pct} size={72} stroke={6}>
            {#snippet center()}
              <span class="ring-pct">{stats.pct}%</span>
            {/snippet}
          </ProgressRing>
          {#if nextLesson}
            <a class="cta" href="/study/course/{courseId}/lesson/{nextLesson.id}">
              {stats.completed > 0 ? $t("study.course.continue") : $t("study.course.start")}
            </a>
          {/if}
        </div>
      {/snippet}
    </PageHero>

    <div class="meta-panel">
      <section class="meta-tags">
        <h3>Tags</h3>
        <div class="chip-row">
          {#each tags as tag (tag)}
            <span class="chip">
              <span>#{tag}</span>
              <button
                type="button"
                class="chip-x"
                onclick={() => removeTag(tag)}
                aria-label={`Remover tag ${tag}`}
              >×</button>
            </span>
          {/each}
          <input
            type="text"
            class="chip-input"
            placeholder="adicionar tag…"
            bind:this={tagInputRef}
            bind:value={newTag}
            onkeydown={onTagKey}
            onblur={() => newTag.trim() && addTag(newTag)}
          />
        </div>
        {#if tagSuggestions.length > 0 && newTag.length === 0}
          <div class="suggestions">
            <span class="sug-label">populares:</span>
            {#each tagSuggestions as s (s.tag)}
              <button
                type="button"
                class="sug"
                onclick={() => addTag(s.tag)}
              >
                #{s.tag}
                <span class="sug-count">{s.course_count}</span>
              </button>
            {/each}
          </div>
        {/if}
      </section>

      <section class="meta-subjects">
        <h3>Matérias</h3>
        <div class="chip-row">
          {#each courseSubjects as subj (subj.id)}
            <span
              class="subj-chip"
              style:--subj-color={subj.color ?? "var(--accent)"}
            >
              <span class="subj-dot"></span>
              {subj.name}
            </span>
          {/each}
          <button
            type="button"
            class="subj-edit"
            onclick={() => (subjectsModalOpen = true)}
          >
            {courseSubjects.length === 0 ? $t("study.course.assign_subjects") : $t("study.common.edit")}
          </button>
        </div>
      </section>

      <section class="meta-actions">
        <h3>Ações</h3>
        <div class="action-row">
          <button
            type="button"
            class="action-btn"
            onclick={probeDurations}
            disabled={probing}
          >
            <span aria-hidden="true">⏱</span>
            <span>
              {probing ? $t("study.course.detecting") : $t("study.course.detect_durations")}
            </span>
            {#if totalDurationMs > 0}
              <span class="action-meta">total: {fmtDuration(totalDurationMs)}</span>
            {/if}
          </button>
        </div>
        {#if probeReport}
          <p class="report">
            ✓ {probeReport.probed} probadas
            · {probeReport.skipped} já tinham
            · {probeReport.failed} falharam
            (de {probeReport.total_lessons} aulas)
          </p>
        {/if}
      </section>
    </div>

    {#if description}
      <section class="description" class:expanded={descriptionExpanded}>
        {#if description.format === "md"}
          <div class="md-render">{@html renderMarkdownSync(description.raw, mdCache)}</div>
        {:else}
          <div class="md-render">{@html description.raw}</div>
        {/if}
        <button
          type="button"
          class="expand-toggle"
          onclick={() => (descriptionExpanded = !descriptionExpanded)}
        >
          {descriptionExpanded ? "Mostrar menos" : "Mostrar mais"}
        </button>
      </section>
    {/if}

    {#if stats.total === 0}
      <p class="muted">{$t("study.course.no_lessons")}</p>
    {:else}
      <NotificationsInlinePanel {courseId} />
      <div class="toolbar">
        <input
          type="search"
          bind:value={lessonSearch}
          placeholder={$t("study.course.search_placeholder")}
          class="lsearch"
        />
      </div>
      <div class="tree">
        {#each (filteredDetail?.modules ?? []) as m (m.id)}
          {@const mPct = modulePct(m)}
          {@const mDone = m.lessons.filter((l) => l.completed).length}
          <section class="module">
            <header class="module-head">
              <span class="pos">{String(m.position).padStart(2, "0")}</span>
              <h2>{m.title}</h2>
              <span class="count">{mDone}/{m.lessons.length}</span>
            </header>
            <div class="module-bar" aria-hidden="true">
              <div
                class="module-bar-fill"
                class:full={mPct >= 100}
                style:width="{mPct}%"
              ></div>
            </div>
            <ul>
              {#each m.lessons as l (l.id)}
                {@const ls = lessonState(l)}
                {@const sel = selectedLessons.has(l.id)}
                <li class="lesson-row" data-state={ls} class:selected={sel}>
                  <input
                    type="checkbox"
                    class="lesson-check"
                    checked={sel}
                    onclick={(e) => toggleSelection(l, e as MouseEvent)}
                    aria-label="Selecionar aula"
                  />
                  <button
                    type="button"
                    class="state-icon"
                    data-state={ls}
                    onclick={(e) => toggleCompleted(l, e)}
                    aria-label={l.completed
                      ? $t("study.course.mark_incomplete")
                      : $t("study.course.mark_complete")}
                  >
                    {#if ls === "done"}
                      ✓
                    {:else if ls === "doing"}
                      ●
                    {:else}
                      ○
                    {/if}
                  </button>
                  <a class="lesson" href="/study/course/{courseId}/lesson/{l.id}">
                    <span class="lpos">{String(l.position).padStart(2, "0")}</span>
                    <span class="ltitle" class:done={l.completed}>{l.title}</span>
                    {#if ls === "doing"}
                      <span class="resume" aria-label={$t("study.course.resume_label")}>▶</span>
                    {/if}
                  </a>
                </li>
              {/each}
            </ul>
          </section>
        {/each}

        {#if (filteredDetail?.loose_lessons.length ?? 0) > 0}
          {@const looseDone = filteredDetail?.loose_lessons.filter((l) => l.completed).length ?? 0}
          {@const looseTotal = filteredDetail?.loose_lessons.length ?? 0}
          {@const loosePct = looseTotal > 0 ? Math.round((looseDone / looseTotal) * 100) : 0}
          <section class="module">
            <header class="module-head">
              <h2>{$t("study.course.loose_lessons")}</h2>
              <span class="count">{looseDone}/{looseTotal}</span>
            </header>
            <div class="module-bar" aria-hidden="true">
              <div
                class="module-bar-fill"
                class:full={loosePct >= 100}
                style:width="{loosePct}%"
              ></div>
            </div>
            <ul>
              {#each (filteredDetail?.loose_lessons ?? []) as l (l.id)}
                {@const ls = lessonState(l)}
                {@const sel = selectedLessons.has(l.id)}
                <li class="lesson-row" data-state={ls} class:selected={sel}>
                  <input
                    type="checkbox"
                    class="lesson-check"
                    checked={sel}
                    onclick={(e) => toggleSelection(l, e as MouseEvent)}
                    aria-label="Selecionar aula"
                  />
                  <button
                    type="button"
                    class="state-icon"
                    data-state={ls}
                    onclick={(e) => toggleCompleted(l, e)}
                    aria-label={l.completed
                      ? $t("study.course.mark_incomplete")
                      : $t("study.course.mark_complete")}
                  >
                    {#if ls === "done"}
                      ✓
                    {:else if ls === "doing"}
                      ●
                    {:else}
                      ○
                    {/if}
                  </button>
                  <a class="lesson" href="/study/course/{courseId}/lesson/{l.id}">
                    <span class="lpos">{String(l.position).padStart(2, "0")}</span>
                    <span class="ltitle" class:done={l.completed}>{l.title}</span>
                    {#if ls === "doing"}
                      <span class="resume" aria-label={$t("study.course.resume_label")}>▶</span>
                    {/if}
                  </a>
                </li>
              {/each}
            </ul>
          </section>
        {/if}
      </div>
      <div class="recs-section">
        <CourseRecommendationsShelf {courseId} limit={6} />
      </div>
    {/if}
  {/if}

  <SubjectsEditModal
    {courseId}
    open={subjectsModalOpen}
    onClose={() => (subjectsModalOpen = false)}
    onSaved={() => {
      void loadCourseSubjects();
      showToast("ok", $t("study.course.subjects_updated"));
    }}
  />

  {#if toast}
    <div class="toast" class:err={toast.kind === "err"} role="status">
      {toast.msg}
    </div>
  {/if}

  {#if selectedLessons.size > 0}
    <div class="selection-bar" role="toolbar" aria-label={$t("study.anki.browse.bulk_aria")}>
      <span class="sel-count">
        <strong>{selectedLessons.size}</strong>
        {selectedLessons.size === 1 ? "selecionada" : "selecionadas"}
      </span>
      <button class="sel-btn" onclick={selectAllVisible}>
        Selecionar todas
      </button>
      <span class="sel-divider"></span>
      <button class="sel-btn primary" onclick={() => bulkMark(true)}>
        ✓ Marcar como completas
      </button>
      <button class="sel-btn" onclick={() => bulkMark(false)}>
        ○ Marcar como incompletas
      </button>
      <span class="sel-divider"></span>
      <button class="sel-btn ghost" onclick={clearSelection}>
        Limpar
      </button>
    </div>
  {/if}
</section>

<style>
  .description {
    position: relative;
    padding: 16px 20px;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius, 10px);
    margin-bottom: 16px;
    max-height: 180px;
    overflow: hidden;
    transition: max-height 240ms ease;
  }
  .description.expanded {
    max-height: none;
  }
  .description:not(.expanded)::after {
    content: "";
    position: absolute;
    inset: auto 0 0 0;
    height: 60px;
    background: linear-gradient(
      to top,
      var(--surface) 10%,
      color-mix(in oklab, var(--surface) 30%, transparent) 100%
    );
    pointer-events: none;
  }
  .expand-toggle {
    position: absolute;
    bottom: 8px;
    left: 50%;
    transform: translateX(-50%);
    padding: 4px 14px;
    background: var(--accent);
    color: var(--accent-contrast, white);
    border: none;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    z-index: 1;
  }
  .description.expanded .expand-toggle {
    position: static;
    transform: none;
    margin-top: 12px;
    display: inline-block;
  }
  .meta-panel {
    display: grid;
    grid-template-columns: minmax(0, 1.6fr) minmax(0, 1fr) minmax(0, 1fr);
    gap: 16px;
    margin-bottom: 16px;
  }
  @media (max-width: 1024px) {
    .meta-panel {
      grid-template-columns: 1fr 1fr;
    }
  }
  @media (max-width: 760px) {
    .meta-panel {
      grid-template-columns: 1fr;
    }
  }
  .meta-tags,
  .meta-subjects,
  .meta-actions {
    padding: 14px 16px;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .meta-tags h3,
  .meta-subjects h3,
  .meta-actions h3 {
    margin: 0;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--tertiary);
  }
  .subj-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--subj-color) 14%, transparent);
    border: 1px solid color-mix(in oklab, var(--subj-color) 30%, transparent);
    color: inherit;
    font-size: 12px;
    font-weight: 500;
  }
  .subj-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--subj-color);
  }
  .subj-edit {
    padding: 4px 10px;
    border-radius: 999px;
    border: none;
    background: transparent;
    color: var(--accent);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
  }
  .subj-edit:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .chip-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 4px 3px 10px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--accent);
    font-size: 12px;
    font-weight: 500;
  }
  .chip-x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 999px;
    border: 0;
    background: transparent;
    color: var(--accent);
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
    padding: 0;
  }
  .chip-x:hover {
    background: color-mix(in oklab, var(--accent) 30%, transparent);
    color: var(--on-accent, var(--on-cta, white));
  }
  .chip-input {
    flex: 1;
    min-width: 140px;
    padding: 4px 10px;
    border: none;
    border-radius: 999px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
  }
  .chip-input:focus {
    outline: none;
    border-color: var(--accent);
    border-style: solid;
  }
  .suggestions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    font-size: 11px;
  }
  .sug-label {
    color: var(--tertiary);
    margin-right: 4px;
  }
  .sug {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: 999px;
    border: none;
    background: transparent;
    color: var(--secondary);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    transition: border-color 120ms ease;
  }
  .sug:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .sug-count {
    color: var(--tertiary);
    font-weight: 600;
  }

  .action-row {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .action-btn {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border: none;
    border-radius: var(--border-radius);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 13px;
    cursor: pointer;
    transition: border-color 120ms ease, background 120ms ease;
  }
  .action-btn:hover:not(:disabled) {
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 5%, transparent);
  }
  .action-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
  .action-meta {
    margin-left: auto;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: var(--tertiary);
  }
  .report {
    margin: 0;
    font-size: 11px;
    color: var(--secondary);
    padding-left: 4px;
  }

  .lesson-check {
    margin-right: 4px;
    width: 14px;
    height: 14px;
    accent-color: var(--accent);
    cursor: pointer;
  }
  .lesson-row.selected {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border-radius: var(--border-radius);
  }

  .selection-bar {
    position: fixed;
    bottom: 20px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    background: var(--surface);
    border: 1px solid var(--accent);
    border-radius: 999px;
    box-shadow: 0 12px 32px color-mix(in oklab, black 24%, transparent);
    z-index: 80;
    animation: slide-up 200ms ease;
  }
  @keyframes slide-up {
    from {
      transform: translate(-50%, 12px);
      opacity: 0;
    }
    to {
      transform: translate(-50%, 0);
      opacity: 1;
    }
  }
  .sel-count {
    color: var(--secondary);
    font-size: 12px;
    padding: 0 6px;
  }
  .sel-count strong {
    color: var(--accent);
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .sel-btn {
    padding: 6px 12px;
    border-radius: 999px;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    transition: background 120ms ease;
  }
  .sel-btn:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .sel-btn.primary {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
    border-color: var(--accent);
  }
  .sel-btn.ghost {
    color: var(--tertiary);
  }
  .sel-divider {
    width: 1px;
    height: 18px;
    background: color-mix(in oklab, var(--input-border) 60%, transparent);
  }

  @media (prefers-reduced-motion: reduce) {
    .selection-bar {
      animation: none;
    }
  }

  .toast {
    position: fixed;
    bottom: 20px;
    right: 20px;
    padding: 10px 16px;
    border-radius: var(--border-radius);
    background: color-mix(in oklab, var(--accent) 16%, var(--surface));
    color: var(--text);
    font-size: 13px;
    z-index: 100;
  }
  .toast.err {
    background: color-mix(
      in oklab,
      var(--error, var(--accent)) 18%,
      var(--surface)
    );
  }
  .study-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 2);
    width: 100%;
    max-width: 1100px;
    margin-inline: auto;
  }
  .muted {
    color: var(--tertiary);
    font-size: 14px;
  }
  .error {
    color: var(--error);
    font-size: 14px;
  }
  .hero-actions {
    display: flex;
    align-items: center;
    gap: var(--padding);
  }
  .ring-pct {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-variant-numeric: tabular-nums;
    font-size: 14px;
    font-weight: 500;
    color: var(--secondary);
  }
  .cta {
    background: var(--cta, var(--accent));
    color: var(--on-cta, var(--on-accent));
    padding: 10px 20px;
    border-radius: var(--border-radius);
    font-size: 13px;
    font-weight: 500;
    text-decoration: none;
    transition: filter 150ms ease;
  }
  .cta:hover {
    filter: brightness(1.08);
  }
  .cta:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--padding);
  }
  .lsearch {
    flex: 1;
    background: var(--input-bg);
    border: none;
    color: var(--secondary);
    padding: 8px 12px;
    border-radius: var(--border-radius);
    font-size: 13px;
  }
  .lsearch:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }

  .tree {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
  }
  .module {
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    overflow: hidden;
  }
  .module-head {
    display: flex;
    align-items: center;
    gap: var(--padding);
    padding: calc(var(--padding) * 1.25) calc(var(--padding) * 1.5);
  }
  .module-head .pos {
    color: var(--tertiary);
    font-family: var(--font-mono, monospace);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .module-head h2 {
    font-size: 14px;
    font-weight: 500;
    margin: 0;
    color: var(--secondary);
    flex: 1;
    min-width: 0;
  }
  .module-head .count {
    color: var(--tertiary);
    font-size: 12px;
    font-family: var(--font-mono, monospace);
    font-variant-numeric: tabular-nums;
  }
  .module-bar {
    height: 2px;
    width: 100%;
    background: color-mix(in oklab, var(--input-border) 60%, transparent);
    overflow: hidden;
  }
  .module-bar-fill {
    height: 100%;
    background: var(--accent);
    transition: width 500ms ease-out, background-color 600ms ease-out;
  }
  .module-bar-fill.full {
    background: var(--success);
  }

  ul {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  .lesson-row {
    display: flex;
    align-items: center;
    gap: var(--padding);
    padding: 8px calc(var(--padding) * 1.5);
    border-top: none;
    transition: background 150ms ease;
  }
  .lesson-row:hover {
    background: var(--sidebar-highlight);
  }
  .state-icon {
    flex: 0 0 auto;
    width: 22px;
    height: 22px;
    border: 1.5px solid var(--input-border);
    border-radius: 999px;
    background: transparent;
    color: var(--tertiary);
    cursor: pointer;
    display: grid;
    place-items: center;
    font-size: 12px;
    line-height: 1;
    transition: background 150ms ease, border-color 150ms ease, color 150ms ease;
  }
  .state-icon[data-state="done"] {
    background: var(--success);
    border-color: var(--success);
    color: var(--on-cta, white);
  }
  .state-icon[data-state="doing"] {
    border-color: var(--accent);
    color: var(--accent);
  }
  .state-icon:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }

  .lesson {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--padding);
    text-decoration: none;
    color: inherit;
    min-width: 0;
  }
  .lpos {
    color: var(--tertiary);
    font-family: var(--font-mono, monospace);
    font-size: 12px;
    flex: 0 0 auto;
    font-variant-numeric: tabular-nums;
  }
  .ltitle {
    color: var(--secondary);
    font-size: 13px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ltitle.done {
    color: var(--tertiary);
    text-decoration: line-through;
  }
  .resume {
    color: var(--accent);
    font-size: 11px;
    flex: 0 0 auto;
  }

  @media (prefers-reduced-motion: reduce) {
    .module-bar-fill,
    .lesson-row,
    .state-icon,
    .cta {
      transition: none;
    }
  }
</style>
