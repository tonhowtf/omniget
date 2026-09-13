<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";

  type PluginStatus =
    | "checking"
    | "ready"
    | "not-installed"
    | "needs-restart"
    | "incompatible"
    | "load-failed";
  type PluginLoadError = {
    message: string;
    kind: string;
    plugin_abi?: number | null;
    expected_abi?: number | null;
  };
  let pluginStatus = $state<PluginStatus>("checking");
  let loadError = $state<PluginLoadError | null>(null);

  type Course = {
    id: number;
    title: string;
    platform?: string | null;
    progress_pct?: number | null;
  };

  type Book = {
    id: number;
    title: string | null;
    file_path: string;
    reading_pct: number;
    last_opened_at: number | null;
  };

  type RecentCourse = {
    course_id: number;
    title: string;
    thumbnail_path: string | null;
    opened_at: number;
    last_lesson_id: number | null;
  };

  type GamificationState = {
    xp: number;
    level: number;
    xp_to_next: number;
    level_progress_pct: number;
    updated_at: number;
  };

  type Branding = { image_data_url: string; name: string };

  let visited = $state<Course[]>([]);
  let recentCourses = $state<RecentCourse[]>([]);
  let gamification = $state<GamificationState | null>(null);
  let unlockedCount = $state(0);
  let dueToday = $state(0);
  let streakDays = $state(0);
  let goalMet = $state(false);
  let resumeBook = $state<Book | null>(null);
  let loading = $state(true);
  let branding = $state<Branding | null>(null);

  async function loadHub() {
    await pluginInvoke("study", "study:anki:storage:open").catch(() => null);
    pluginInvoke<Branding>("study", "study:hub:get_branding")
      .then((b) => (branding = b ?? null))
      .catch(() => (branding = null));
    pluginInvoke<RecentCourse[]>("study", "study:courses:recents:list", {
      limit: 8,
    })
      .then((rows) => (recentCourses = rows ?? []))
      .catch(() => (recentCourses = []));
    pluginInvoke<GamificationState>("study", "study:gamification:state")
      .then((g) => (gamification = g))
      .catch(() => (gamification = null));
    pluginInvoke<unknown[]>("study", "study:gamification:achievements")
      .then((rows) => (unlockedCount = Array.isArray(rows) ? rows.length : 0))
      .catch(() => (unlockedCount = 0));
    const [histRes, dueRes, overviewRes, goalsRes, booksRes] = await Promise.all([
      pluginInvoke("study", "study:history:recent", {
        limit: 12,
        entityTypes: ["course"],
      }).catch(() => ({ items: [] })),
      pluginInvoke("study", "study:anki:search:cards", {
        query: "is:due",
        limit: 0,
        offset: 0,
      }).catch(() => ({ total: 0 })),
      pluginInvoke("study", "study:progress:overview", { days: 7 }).catch(
        () => null,
      ),
      pluginInvoke("study", "study:goals:progress").catch(() => null),
      pluginInvoke("study", "study:read:library:list", {
        filters: { pageSize: 50 },
      }).catch(() => ({ items: [] as Book[], total: 0 })),
    ]);

    const h = histRes as { items?: { entity_id?: number }[] } | null;
    const histItems = Array.isArray(h?.items) ? h.items : [];
    const courseIds = histItems
      .map((x) => x.entity_id)
      .filter((x): x is number => typeof x === "number");
    if (courseIds.length) {
      const all = (await pluginInvoke("study", "study:courses:list", {
        filters: {},
      }).catch(() => [])) as Course[];
      const byId = new Map((Array.isArray(all) ? all : []).map((c) => [c.id, c]));
      visited = histItems
        .map((hi) => (hi.entity_id != null ? byId.get(hi.entity_id) : undefined))
        .filter((c): c is Course => !!c)
        .slice(0, 8);
    }

    const due = dueRes as { total?: number } | null;
    dueToday = due?.total ?? 0;

    const ov = overviewRes as { streak?: number } | null;
    streakDays = ov?.streak ?? 0;

    const g = goalsRes as {
      daily?: {
        minutes?: { reached_at_target?: boolean };
        lessons?: { reached_at_target?: boolean };
        cards?: { reached_at_target?: boolean };
      };
    } | null;
    goalMet =
      (g?.daily?.minutes?.reached_at_target ?? false) ||
      (g?.daily?.lessons?.reached_at_target ?? false) ||
      (g?.daily?.cards?.reached_at_target ?? false);

    const booksAny = booksRes as { items?: Book[] } | Book[] | null;
    const books: Book[] = Array.isArray(booksAny)
      ? (booksAny as Book[])
      : (booksAny?.items ?? []);
    const cutoff = Math.floor(Date.now() / 1000) - 7 * 24 * 60 * 60;
    resumeBook = books.find(
      (b) =>
        b.reading_pct > 0 &&
        b.reading_pct < 0.99 &&
        typeof b.last_opened_at === "number" &&
        b.last_opened_at >= cutoff,
    ) ?? null;
  }

  onMount(async () => {
    try {
      const plugins = await invoke<{
        id: string;
        enabled: boolean;
        loaded: boolean;
        load_error?: PluginLoadError | null;
      }[]>("list_plugins");
      const study = plugins.find((p) => p.id === "study");
      if (!study || !study.enabled) {
        pluginStatus = "not-installed";
        return;
      }
      if (!study.loaded) {
        if (study.load_error) {
          loadError = study.load_error;
          pluginStatus =
            study.load_error.kind === "abi_mismatch" ||
            study.load_error.kind === "missing_abi_symbol"
              ? "incompatible"
              : "load-failed";
        } else {
          pluginStatus = "needs-restart";
        }
        return;
      }
      pluginStatus = "ready";
    } catch {
      pluginStatus = "ready";
    }

    try {
      await loadHub();
    } catch (_) {
      // ignore
    } finally {
      loading = false;
    }
  });

  function openCourse(c: Course) {
    pluginInvoke("study", "study:history:record", {
      entityType: "course",
      entityId: c.id,
    }).catch(() => {});
    goto(`/study/course/${encodeURIComponent(c.id)}`);
  }

  const resumeCourse = $derived(
    visited.find((c) => (c.progress_pct ?? 0) < 100) ?? null,
  );

  type Hero =
    | { kind: "continue"; course: Course }
    | { kind: "review"; count: number }
    | { kind: "focus" };

  const hero = $derived(
    resumeCourse
      ? { kind: "continue", course: resumeCourse }
      : dueToday > 0
        ? { kind: "review", count: dueToday }
        : { kind: "focus" },
  );

  const greeting = $derived.by(() => {
    const h = new Date().getHours();
    if (h < 12) return $t("study.hub.greeting_morning");
    if (h < 18) return $t("study.hub.greeting_afternoon");
    return $t("study.hub.greeting_evening");
  });

  const streakColor = $derived(
    goalMet
      ? "var(--success)"
      : streakDays > 0
        ? "var(--accent)"
        : "var(--tertiary)",
  );
</script>

{#if pluginStatus === "checking"}
  <div class="plugin-guard"><span class="spinner"></span></div>
{:else if pluginStatus === "not-installed"}
  <div class="plugin-guard">
    <h2>{$t("marketplace.plugin_not_installed")}</h2>
    <a href="/marketplace" class="guard-link">{$t("marketplace.go_to_marketplace")}</a>
  </div>
{:else if pluginStatus === "needs-restart"}
  <div class="plugin-guard">
    <h2>{$t("marketplace.restart_required")}</h2>
    <p>{$t("marketplace.plugin_restart_hint")}</p>
  </div>
{:else if pluginStatus === "incompatible"}
  <div class="plugin-guard">
    <h2>{$t("marketplace.plugin_incompatible_title")}</h2>
    <p>{$t("marketplace.plugin_incompatible_hint")}</p>
    <a href="/marketplace" class="guard-link">{$t("marketplace.go_to_marketplace")}</a>
    {#if loadError}
      <p class="guard-detail"><code>{loadError.message}</code></p>
    {/if}
  </div>
{:else if pluginStatus === "load-failed"}
  <div class="plugin-guard">
    <h2>{$t("marketplace.plugin_load_failed_title")}</h2>
    <p>{$t("marketplace.plugin_load_failed_hint")}</p>
    <a href="/marketplace" class="guard-link">{$t("marketplace.go_to_marketplace")}</a>
    {#if loadError}
      <p class="guard-detail"><code>{loadError.message}</code></p>
    {/if}
  </div>
{:else}
  <section class="hub">
    <header class="masthead">
      <h1 class="page-title">{greeting}</h1>
      <div class="masthead-pills">
        {#if gamification && gamification.xp > 0}
          <a
            class="xp-pill"
            href="/study/achievements"
            title={`${gamification.xp.toLocaleString()} XP · ${gamification.level_progress_pct}% para nível ${gamification.level + 1}`}
          >
            <span class="xp-level">L{gamification.level}</span>
            <span class="xp-bar-mini">
              <span
                class="xp-bar-mini-fill"
                style:width={`${gamification.level_progress_pct}%`}
              ></span>
            </span>
            {#if unlockedCount > 0}
              <span class="ach-count">🏆 {unlockedCount}</span>
            {/if}
          </a>
        {/if}
        {#if streakDays > 0 || goalMet}
          <button
            class="streak"
            type="button"
            onclick={() => goto("/study/progress")}
            title={goalMet ? $t("study.hub.goal_met") : ""}
            style:--streak-color="{streakColor}"
          >
            <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true">
              <path d="M13.5 0.67s.74 2.65.74 4.8c0 2.06-1.35 3.73-3.41 3.73S7 7.53 7 5.47c0-1.02.25-1.98.58-2.82-.84 1.5-1.58 3.3-1.58 5.19 0 4.42 3.58 8 8 8s8-3.58 8-8c0-5.9-4.5-9.17-8.5-12.17z"></path>
            </svg>
            <span class="mono">{streakDays}</span>
          </button>
        {/if}
      </div>
    </header>

    {#if !loading}
      <article class="hero" class:continue={hero.kind === "continue"} class:review={hero.kind === "review"} class:focus={hero.kind === "focus"}>
        {#if hero.kind === "continue"}
          <span class="hero-label">{$t("study.hub.continue_title")}</span>
          <h2 class="hero-title">{hero.course.title}</h2>
          <div class="hero-progress">
            <div class="progress-track">
              <div class="progress-fill" style:width="{Math.round(hero.course.progress_pct ?? 0)}%"></div>
            </div>
            <span class="mono hero-pct">{Math.round(hero.course.progress_pct ?? 0)}%</span>
          </div>
          <button class="hero-cta" onclick={() => openCourse(hero.course)}>
            {$t("study.hub.continue_cta")}
            <span aria-hidden="true">→</span>
          </button>
        {:else if hero.kind === "review"}
          <span class="hero-label">{$t("study.hub.review_title")}</span>
          <div class="hero-big">
            <strong class="big-n mono">{hero.count}</strong>
            <span class="big-unit">{$t("study.hub.review_subtitle")}</span>
          </div>
          <button class="hero-cta" onclick={() => goto("/study/anki")}>
            {$t("study.hub.review_cta")}
            <span aria-hidden="true">→</span>
          </button>
        {:else}
          <span class="hero-label">{$t("study.hub.focus_title")}</span>
          <p class="hero-hint">{$t("study.hub.focus_hint_default")}</p>
          <button class="hero-cta" onclick={() => goto("/study/focus")}>
            {$t("study.hub.focus_cta")}
            <span aria-hidden="true">→</span>
          </button>
        {/if}
      </article>

      <nav class="secondary" aria-label="more actions">
        {#if hero.kind !== "review" && dueToday > 0}
          <a href="/study/anki" class="s-link">
            <span>{$t("study.hub.secondary_review", { n: dueToday })}</span>
            <span aria-hidden="true">→</span>
          </a>
        {/if}
        {#if hero.kind !== "continue" && resumeCourse}
          <a href="/study/course/{resumeCourse.id}" class="s-link">
            <span>{$t("study.hub.secondary_continue", { title: resumeCourse.title })}</span>
            <span aria-hidden="true">→</span>
          </a>
        {/if}
        {#if resumeBook}
          <a href="/study/read/{resumeBook.id}" class="s-link">
            <span>{$t("study.hub.secondary_resume_book", { title: resumeBook.title ?? resumeBook.file_path.split(/[\\/]/).pop() ?? "…" })}</span>
            <span aria-hidden="true">→</span>
          </a>
        {/if}
        {#if hero.kind !== "focus"}
          <a href="/study/focus" class="s-link">
            <span>{$t("study.hub.secondary_focus")}</span>
            <span aria-hidden="true">→</span>
          </a>
        {/if}
        <a href="/study/library" class="s-link">
          <span>{$t("study.hub.secondary_explore")}</span>
          <span aria-hidden="true">→</span>
        </a>
      </nav>

      {#if recentCourses.length > 0}
        <section class="recents-widget">
          <header class="recents-head">
            <h2>{$t("study.hub.continue_section")}</h2>
            <a href="/study/library" class="see-all">{$t("study.hub.see_all")} →</a>
          </header>
          <ul class="recents-list">
            {#each recentCourses.slice(0, 6) as r (r.course_id)}
              <li>
                <a
                  class="recent-card"
                  href={r.last_lesson_id
                    ? `/study/course/${r.course_id}/lesson/${r.last_lesson_id}`
                    : `/study/course/${r.course_id}`}
                >
                  <span class="recent-title">{r.title}</span>
                  <span class="recent-when">
                    {new Date(r.opened_at * 1000).toLocaleDateString()}
                  </span>
                </a>
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    {/if}

    {#if branding}
      <footer class="made-by">
        <img class="made-by-img" src={branding.image_data_url} alt={branding.name} />
        <span class="made-by-text">
          {$t("study.hub.made_by")}
          <strong>{branding.name}</strong>
        </span>
      </footer>
    {/if}
  </section>
{/if}

<style>
  .hub {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 3);
    width: 100%;
    max-width: 560px;
    margin-inline: auto;
    padding-top: calc(var(--padding) * 3);
  }
  .masthead {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
    animation: fade-in 320ms cubic-bezier(0.22, 1, 0.36, 1) both;
  }
  .masthead h1 {
    margin: 0;
    font-size: 2rem;
    font-weight: 500;
    letter-spacing: -0.75px;
    color: var(--secondary);
  }
  .streak {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.3rem 0.65rem;
    background: color-mix(in oklab, var(--streak-color) 10%, transparent);
    border: none;
    border-radius: 999px;
    color: var(--streak-color);
    cursor: pointer;
    font-family: inherit;
    font-size: 12px;
    font-weight: 500;
    transition:
      background 150ms ease,
      border-color 150ms ease;
  }
  .streak:hover {
    background: color-mix(in oklab, var(--streak-color) 18%, transparent);
  }
  .streak .mono {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-variant-numeric: tabular-nums;
  }

  .hero {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    padding: calc(var(--padding) * 3);
    background: var(--button-elevated);
    border: none;
    border-radius: calc(var(--border-radius) * 1.4);
    animation: hero-in 420ms cubic-bezier(0.22, 1, 0.36, 1) both;
    animation-delay: 80ms;
  }
  .hero > * {
    animation: hero-in 360ms cubic-bezier(0.22, 1, 0.36, 1) both;
  }
  .hero > *:nth-child(1) {
    animation-delay: 140ms;
  }
  .hero > *:nth-child(2) {
    animation-delay: 200ms;
  }
  .hero > *:nth-child(3) {
    animation-delay: 260ms;
  }
  .hero > *:nth-child(4) {
    animation-delay: 320ms;
  }
  .hero-label {
    font-size: 11px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--tertiary);
  }
  .hero-title {
    margin: 0;
    font-size: 1.5rem;
    font-weight: 500;
    letter-spacing: -0.5px;
    color: var(--secondary);
    line-height: 1.25;
  }
  .hero-progress {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-top: -0.25rem;
  }
  .progress-track {
    flex: 1;
    height: 3px;
    background: color-mix(in oklab, var(--content-border) 70%, transparent);
    border-radius: 2px;
    overflow: hidden;
  }
  .progress-fill {
    height: 100%;
    background: var(--accent);
    transition: width 600ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  .hero-pct {
    font-size: 12px;
    color: var(--tertiary);
    font-variant-numeric: tabular-nums;
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
  }
  .hero-big {
    display: flex;
    align-items: baseline;
    gap: 0.6rem;
    line-height: 1;
    margin-top: 0.25rem;
  }
  .big-n {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-variant-numeric: tabular-nums;
    font-size: 3rem;
    font-weight: 500;
    color: var(--accent);
    letter-spacing: -1px;
  }
  .big-unit {
    font-size: 13px;
    color: var(--tertiary);
  }
  .hero-hint {
    margin: 0.25rem 0 0;
    font-size: 14px;
    color: var(--tertiary);
  }
  .hero-cta {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.4rem;
    margin-top: 0.5rem;
    padding: 0.75rem 1.25rem;
    background: var(--accent);
    color: var(--on-accent);
    border: 0;
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    align-self: flex-start;
    transition: background 150ms ease;
  }
  .hero-cta:hover {
    background: color-mix(in oklab, var(--accent) 88%, black);
  }

  .secondary {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    animation: fade-in 420ms cubic-bezier(0.22, 1, 0.36, 1) both;
    animation-delay: 380ms;
  }
  .s-link {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.45rem 0.9rem;
    color: var(--secondary);
    text-decoration: none;
    font-size: 13px;
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border: 1px solid color-mix(in oklab, var(--accent) 18%, var(--content-border));
    border-radius: 999px;
    transition:
      background 150ms ease,
      border-color 150ms ease,
      color 150ms ease,
      transform 150ms ease;
  }
  .s-link:hover {
    background: color-mix(in oklab, var(--accent) 16%, transparent);
    border-color: color-mix(in oklab, var(--accent) 35%, var(--content-border));
    color: var(--accent);
    transform: translateY(-1px);
  }
  .s-link:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }
  .s-link span[aria-hidden] {
    color: color-mix(in oklab, var(--secondary) 60%, transparent);
    font-size: 12px;
    transition: transform 150ms ease;
  }
  .s-link:hover span[aria-hidden] {
    transform: translateX(2px);
    color: var(--accent);
  }

  @keyframes fade-in {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
  @keyframes hero-in {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .masthead,
    .hero,
    .hero > *,
    .secondary {
      animation: none;
    }
    .progress-fill,
    .s-link,
    .s-link span[aria-hidden] {
      transition: none;
    }
  }

  .plugin-guard {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: calc(100vh - var(--padding) * 4);
    gap: calc(var(--padding) * 1.5);
    text-align: center;
    color: var(--tertiary);
  }
  .plugin-guard h2 {
    font-size: 18px;
    color: var(--secondary);
  }
  .plugin-guard p {
    font-size: 14px;
    max-width: 300px;
  }
  .guard-link {
    padding: 10px 24px;
    font-size: 14px;
    font-weight: 500;
    background: var(--cta);
    color: var(--on-cta);
    border-radius: var(--border-radius);
    text-decoration: none;
  }
  .guard-detail {
    font-size: 12px;
    color: var(--tertiary);
    max-width: 480px;
    margin-top: calc(var(--padding) * 0.5);
  }
  .guard-detail code {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    word-break: break-word;
    background: var(--surface);
    padding: 2px 6px;
    border-radius: 4px;
  }
  .spinner {
    width: 24px;
    height: 24px;
    border: 2px solid var(--input-border);
    border-top-color: var(--secondary);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .masthead-pills {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .xp-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 999px;
    border: 1px solid color-mix(in oklab, var(--accent) 40%, var(--input-border));
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    color: var(--accent);
    text-decoration: none;
    font-size: 11px;
    font-weight: 600;
    transition: background 120ms ease;
  }
  .xp-pill:hover {
    background: color-mix(in oklab, var(--accent) 16%, transparent);
  }
  .xp-level {
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .xp-bar-mini {
    width: 50px;
    height: 5px;
    background: color-mix(in oklab, var(--accent) 20%, transparent);
    border-radius: 999px;
    overflow: hidden;
    display: inline-block;
  }
  .xp-bar-mini-fill {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 400ms ease;
  }
  .ach-count {
    color: var(--accent);
    font-size: 11px;
  }

  .recents-widget {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: calc(var(--padding) * 1);
  }
  .recents-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }
  .recents-head h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--secondary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .see-all {
    color: var(--accent);
    font-size: 11px;
    text-decoration: none;
  }
  .see-all:hover {
    text-decoration: underline;
  }
  .recents-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: 1fr;
    gap: 4px;
  }
  .recent-card {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--surface);
    color: var(--text);
    text-decoration: none;
    font-size: 13px;
    transition: border-color 120ms ease, background 120ms ease;
  }
  .recent-card:hover {
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 5%, var(--surface));
  }
  .recent-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .recent-when {
    color: var(--tertiary);
    font-size: 11px;
    flex-shrink: 0;
  }

  .made-by {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.6rem;
    margin-top: calc(var(--padding) * 2);
    padding: 0.75rem 1rem;
    border-top: none;
    color: var(--tertiary);
    font-size: 12px;
    animation: fade-in 480ms cubic-bezier(0.22, 1, 0.36, 1) both;
    animation-delay: 480ms;
  }
  .made-by-img {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    object-fit: cover;
    border: none;
    flex-shrink: 0;
  }
  .made-by-text strong {
    color: var(--secondary);
    font-weight: 600;
    margin-left: 0.25rem;
    letter-spacing: 0.01em;
  }
  @media (prefers-reduced-motion: reduce) {
    .made-by {
      animation: none;
    }
  }
</style>
