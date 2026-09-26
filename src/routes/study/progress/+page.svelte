<script lang="ts">
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";
  import PageHero from "$lib/study-components/PageHero.svelte";
  import StatCard from "$lib/study-components/StatCard.svelte";
  import ProgressRing from "$lib/study-components/ProgressRing.svelte";
  import SegmentedControl from "$lib/study-components/SegmentedControl.svelte";
  import EmptyState from "$lib/study-components/EmptyState.svelte";

  type HeatCell = { date: string; lessons: number; minutes: number };
  type Overview = {
    streak: number;
    heatmap: HeatCell[];
    total_completed_lessons: number;
    courses_total: number;
    courses_completed: number;
    today_lessons: number;
    today_minutes: number;
  };

  type ReadSession = {
    id: number;
    book_id: number;
    started_at: number;
    ended_at: number | null;
    seconds_active: number;
    pages_read: number;
  };

  type Book = {
    id: number;
    title: string | null;
    file_path: string;
    reading_pct: number;
    last_opened_at: number | null;
  };

  type ReadStats = {
    streak: number;
    today_seconds: number;
    today_pages: number;
    week_seconds: number;
    total_seconds: number;
    active_book: Book | null;
  };

  const DAILY_LESSON_GOAL = 3;
  const DAILY_MINUTE_GOAL = 45;

  type TimePerCourse = {
    items: { id: number; title: string; minutes: number }[];
    days: number;
  };

  type ActivityByHour = {
    buckets: { hour: number; minutes: number }[];
    days: number;
  };

  type SubjectBreakdown = {
    items: { id: number; name: string; color: string; minutes: number }[];
    days: number;
    unassigned_minutes: number;
  };

  let period = $state<"1" | "7" | "30" | "90">("30");
  let data = $state<Overview | null>(null);
  let readStats = $state<ReadStats | null>(null);
  let timePerCourse = $state<TimePerCourse | null>(null);
  let activityByHour = $state<ActivityByHour | null>(null);
  let subjectBreakdown = $state<SubjectBreakdown | null>(null);

  type RecentAchievement = {
    code: string;
    tier: string;
    unlocked_at: number;
  };

  let recentAchievements = $state<RecentAchievement[]>([]);

  function fmtRelDays(secs: number): string {
    const days = Math.floor((Date.now() / 1000 - secs) / 86400);
    if (days <= 0) return $t("study.progress.time_today") as string;
    if (days === 1) return $t("study.progress.time_yesterday") as string;
    if (days < 30) return $t("study.progress.time_days_ago", { count: days }) as string;
    if (days < 365)
      return $t("study.progress.time_months_ago", { count: Math.floor(days / 30) }) as string;
    return $t("study.progress.time_years_ago", { count: Math.floor(days / 365) }) as string;
  }

  function tierColor(tier: string): string {
    switch (tier.toLowerCase()) {
      case "gold":
        return "#f59e0b";
      case "silver":
        return "#9ca3af";
      case "bronze":
        return "#b45309";
      default:
        return "var(--accent)";
    }
  }

  function achievementLabel(code: string): string {
    const keyMap: Record<string, string> = {
      "xp:100": "study.progress.ach_label_xp_100",
      "xp:500": "study.progress.ach_label_xp_500",
      "xp:1000": "study.progress.ach_label_xp_1000",
      "xp:5000": "study.progress.ach_label_xp_5000",
      "xp:10000": "study.progress.ach_label_xp_10000",
      "lessons:1": "study.progress.ach_label_lessons_1",
      "lessons:10": "study.progress.ach_label_lessons_10",
      "lessons:50": "study.progress.ach_label_lessons_50",
      "lessons:100": "study.progress.ach_label_lessons_100",
      "focus:60": "study.progress.ach_label_focus_60",
      "focus:600": "study.progress.ach_label_focus_600",
      "focus:6000": "study.progress.ach_label_focus_6000",
      "streak:3": "study.progress.ach_label_streak_3",
      "streak:7": "study.progress.ach_label_streak_7",
      "streak:30": "study.progress.ach_label_streak_30",
      "streak:100": "study.progress.ach_label_streak_100",
    };
    const key = keyMap[code];
    return key ? ($t(key) as string) : code;
  }

  type DonutSlice = { name: string; minutes: number; color: string };

  const donutSlices = $derived.by<DonutSlice[]>(() => {
    if (!subjectBreakdown) return [];
    const arr: DonutSlice[] = subjectBreakdown.items.map((i) => ({
      name: i.name,
      minutes: i.minutes,
      color: i.color || "var(--accent)",
    }));
    if (subjectBreakdown.unassigned_minutes > 0) {
      arr.push({
        name: $t("study.progress.chart_subject_unassigned") as string,
        minutes: subjectBreakdown.unassigned_minutes,
        color: "var(--tertiary)",
      });
    }
    return arr;
  });

  const donutTotal = $derived(
    donutSlices.reduce((s, x) => s + x.minutes, 0),
  );
  let loading = $state(true);
  let error = $state("");

  const days = $derived(parseInt(period, 10));

  const periodOptions = $derived([
    { value: "1", label: $t("study.progress.period_today") },
    { value: "7", label: $t("study.progress.period_7d") },
    { value: "30", label: $t("study.progress.period_30d") },
    { value: "90", label: $t("study.progress.period_90d") },
  ]);

  async function load() {
    loading = true;
    error = "";
    try {
      const [overview, sessions, libRes, tpc, abh, sb, achs] = await Promise.all([
        pluginInvoke<Overview>("study", "study:progress:overview", { days }),
        pluginInvoke<ReadSession[]>("study", "study:read:session:history", {
          limit: 365,
        }).catch(() => []),
        pluginInvoke<{ items: Book[]; total: number } | Book[]>(
          "study",
          "study:read:library:list",
          { filters: { pageSize: 200 } },
        ).catch(() => ({ items: [] as Book[], total: 0 })),
        pluginInvoke<TimePerCourse>("study", "study:progress:time_per_course", {
          days,
          limit: 6,
        }).catch(() => ({ items: [], days })),
        pluginInvoke<ActivityByHour>(
          "study",
          "study:progress:activity_by_hour",
          { days },
        ).catch(() => ({ buckets: [], days })),
        pluginInvoke<SubjectBreakdown>(
          "study",
          "study:progress:subject_breakdown",
          { days, limit: 8 },
        ).catch(() => ({ items: [], days, unassigned_minutes: 0 })),
        pluginInvoke<RecentAchievement[]>(
          "study",
          "study:gamification:achievements",
        ).catch(() => []),
      ]);
      data = overview;
      const books: Book[] = Array.isArray(libRes) ? libRes : (libRes?.items ?? []);
      readStats = aggregateReadStats(sessions, books);
      timePerCourse = tpc;
      activityByHour = abh;
      subjectBreakdown = sb;
      recentAchievements = (Array.isArray(achs) ? achs : [])
        .slice()
        .sort((a, b) => b.unlocked_at - a.unlocked_at)
        .slice(0, 6);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void days;
    load();
  });

  function dayKey(ts: number): string {
    const d = new Date(ts * 1000);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }

  function aggregateReadStats(sessions: ReadSession[], books: Book[]): ReadStats {
    const now = new Date();
    const todayKey = dayKey(Math.floor(now.getTime() / 1000));
    const weekCutoff = Math.floor(now.getTime() / 1000) - 7 * 24 * 60 * 60;

    let todaySeconds = 0;
    let todayPages = 0;
    let weekSeconds = 0;
    let totalSeconds = 0;
    const activeDays = new Set<string>();

    for (const s of sessions) {
      totalSeconds += s.seconds_active;
      if (s.started_at >= weekCutoff) weekSeconds += s.seconds_active;
      const key = dayKey(s.started_at);
      if (key === todayKey) {
        todaySeconds += s.seconds_active;
        todayPages += s.pages_read;
      }
      if (s.seconds_active > 0) activeDays.add(key);
    }

    let streak = 0;
    const cursor = new Date(now);
    while (true) {
      const k = dayKey(Math.floor(cursor.getTime() / 1000));
      if (activeDays.has(k)) {
        streak += 1;
        cursor.setDate(cursor.getDate() - 1);
      } else if (streak === 0 && k === todayKey) {
        cursor.setDate(cursor.getDate() - 1);
      } else {
        break;
      }
    }

    const active = books.find(
      (b) => b.reading_pct > 0 && b.reading_pct < 0.99,
    ) ?? null;

    return {
      streak,
      today_seconds: todaySeconds,
      today_pages: todayPages,
      week_seconds: weekSeconds,
      total_seconds: totalSeconds,
      active_book: active,
    };
  }

  function fmtDuration(seconds: number): string {
    if (seconds < 60) return `${seconds}s`;
    const m = Math.floor(seconds / 60);
    if (m < 60) return `${m}m`;
    const h = Math.floor(m / 60);
    const mm = m % 60;
    return mm === 0 ? `${h}h` : `${h}h${mm}m`;
  }

  function cellColor(c: HeatCell): string {
    const score = c.lessons * 2 + Math.floor(c.minutes / 15);
    if (score <= 0) return "var(--sidebar-highlight)";
    if (score <= 2) return "color-mix(in oklab, var(--accent) 25%, transparent)";
    if (score <= 4) return "color-mix(in oklab, var(--accent) 50%, transparent)";
    if (score <= 8) return "color-mix(in oklab, var(--accent) 75%, transparent)";
    return "var(--accent)";
  }

  function cellTitle(c: HeatCell): string {
    return `${c.date} · ${c.lessons} · ${c.minutes}min`;
  }

  const weeks = $derived.by(() => {
    if (!data) return [] as HeatCell[][];
    const cells = data.heatmap;
    const out: HeatCell[][] = [];
    for (let i = 0; i < cells.length; i += 7) {
      out.push(cells.slice(i, i + 7));
    }
    return out;
  });

  const lessonsPct = $derived.by(() =>
    data ? Math.min(100, Math.round((data.today_lessons / DAILY_LESSON_GOAL) * 100)) : 0,
  );
  const minutesPct = $derived.by(() =>
    data ? Math.min(100, Math.round((data.today_minutes / DAILY_MINUTE_GOAL) * 100)) : 0,
  );
  const goalPct = $derived(Math.min(lessonsPct, minutesPct));
  const lessonsHit = $derived(data ? data.today_lessons >= DAILY_LESSON_GOAL : false);
  const minutesHit = $derived(data ? data.today_minutes >= DAILY_MINUTE_GOAL : false);

  const isEmpty = $derived(
    data != null &&
      data.total_completed_lessons === 0 &&
      data.streak === 0 &&
      (readStats?.total_seconds ?? 0) === 0,
  );
</script>

<section class="study-page">
  <PageHero
    title={$t("study.progress.title")}
    subtitle={$t("study.progress.page_subtitle")}
  />

  <div class="period-row">
    <SegmentedControl
      options={periodOptions}
      bind:value={period}
      ariaLabel={$t("study.progress.title")}
    />
  </div>

  {#if loading}
    <p class="muted">{$t("study.common.loading")}</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if isEmpty}
    <EmptyState
      title={$t("study.progress.no_data")}
      description={$t("study.progress.placeholder")}
    >
      {#snippet icon()}
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <line x1="3" y1="20" x2="21" y2="20" />
          <rect x="5" y="12" width="3" height="8" />
          <rect x="10.5" y="8" width="3" height="12" />
          <rect x="16" y="14" width="3" height="6" />
        </svg>
      {/snippet}
      {#snippet actions()}
        <a class="btn-link" href="/study/library">
          {$t("study.progress.chart_explore_library")}
        </a>
      {/snippet}
    </EmptyState>
  {:else if data}
    <section class="hero-today">
      <div class="ring-wrap">
        <span class="hero-eyebrow">{$t("study.progress.hero_today")}</span>
        <ProgressRing value={goalPct} size={112} stroke={8}>
          {#snippet center()}
            <div class="ring-center">
              <span class="ring-pct">{goalPct}%</span>
              <span class="ring-sub">{$t("study.progress.daily_goal_pct")}</span>
            </div>
          {/snippet}
        </ProgressRing>
      </div>
      <div class="hero-stats">
        <StatCard
          label={$t("study.progress.stat_lessons")}
          value={`${data.today_lessons}/${DAILY_LESSON_GOAL}`}
          accent={lessonsHit}
        />
        <StatCard
          label={$t("study.progress.stat_minutes")}
          value={`${data.today_minutes}/${DAILY_MINUTE_GOAL}`}
          accent={minutesHit}
        />
        <StatCard
          label={$t("study.progress.stat_streak")}
          value={data.streak}
          hint={$t("study.hub.streak_label")}
          accent={data.streak > 0}
        />
        {#if readStats}
          <StatCard
            label={$t("study.progress.read_today")}
            value={fmtDuration(readStats.today_seconds)}
          />
        {/if}
      </div>
    </section>

    <section class="card heatmap">
      <header class="card-head">
        <h2>{$t("study.progress.heatmap_title")}</h2>
        <small class="card-hint">{$t("study.progress.heatmap_hint", { count: days })}</small>
      </header>
      <div class="grid" role="img" aria-label={$t("study.progress.heatmap_title")}>
        {#each weeks as week, wi (wi)}
          <div class="col">
            {#each week as c (c.date)}
              <span class="cell" style:background="{cellColor(c)}" title={cellTitle(c)}></span>
            {/each}
          </div>
        {/each}
      </div>
      <footer class="card-foot">
        <div class="legend">
          <small>{$t("study.progress.legend_less")}</small>
          <span class="cell" style="background: var(--sidebar-highlight)"></span>
          <span class="cell" style="background: color-mix(in oklab, var(--accent) 25%, transparent)"></span>
          <span class="cell" style="background: color-mix(in oklab, var(--accent) 50%, transparent)"></span>
          <span class="cell" style="background: color-mix(in oklab, var(--accent) 75%, transparent)"></span>
          <span class="cell" style="background: var(--accent)"></span>
          <small>{$t("study.progress.legend_more")}</small>
        </div>
        <a class="card-cta" href="/study/library">{$t("study.progress.chart_explore_library")}</a>
      </footer>
    </section>

    <section class="charts-grid">
      <article class="card chart-card">
        <header class="card-head">
          <h2>{$t("study.progress.chart_time_per_course")}</h2>
          <small class="card-hint">{$t("study.progress.chart_time_period_days", { count: days })}</small>
        </header>
        <div class="chart-body">
          {#if timePerCourse && timePerCourse.items.length > 0}
            {@const maxMin = Math.max(...timePerCourse.items.map((x) => x.minutes), 1)}
            <ul class="hbar-list">
              {#each timePerCourse.items as c (c.id)}
                <li class="hbar-row">
                  <span class="hbar-label" title={c.title}>{c.title}</span>
                  <div class="hbar-track">
                    <div
                      class="hbar-fill"
                      style:width={`${(c.minutes / maxMin) * 100}%`}
                    ></div>
                  </div>
                  <span class="hbar-value">{c.minutes}m</span>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="chart-empty">{$t("study.progress.chart_time_per_course_empty")}</p>
          {/if}
        </div>
      </article>

      <article class="card chart-card">
        <header class="card-head">
          <h2>{$t("study.progress.chart_activity_by_hour")}</h2>
          <small class="card-hint">{$t("study.progress.chart_activity_hint")}</small>
        </header>
        <div class="chart-body">
          {#if activityByHour && activityByHour.buckets.some((b) => b.minutes > 0)}
            {@const maxBucket = Math.max(...activityByHour.buckets.map((b) => b.minutes), 1)}
            <div class="hours-grid">
              {#each activityByHour.buckets as b (b.hour)}
                <div
                  class="hour-bar"
                  class:has={b.minutes > 0}
                  title={`${b.hour}h: ${b.minutes}min`}
                >
                  <div
                    class="hour-fill"
                    style:height={`${(b.minutes / maxBucket) * 100}%`}
                  ></div>
                </div>
              {/each}
            </div>
            <div class="hours-axis">
              <span>0h</span>
              <span>6h</span>
              <span>12h</span>
              <span>18h</span>
              <span>24h</span>
            </div>
          {:else}
            <p class="chart-empty">{$t("study.progress.chart_activity_empty", { count: days })}</p>
          {/if}
        </div>
      </article>

      <article class="card chart-card">
        <header class="card-head">
          <h2>{$t("study.progress.chart_subject")}</h2>
          <small class="card-hint">{$t("study.progress.chart_subject_unit")}</small>
        </header>
        <div class="chart-body">
          {#if donutSlices.length > 0 && donutTotal > 0}
            <div class="donut-wrap">
              <svg class="donut" viewBox="0 0 120 120" aria-hidden="true">
                {#each donutSlices as it, i (it.name)}
                  {@const before = donutSlices.slice(0, i).reduce((s, x) => s + x.minutes, 0)}
                  {@const offset = (before / donutTotal) * 100}
                  {@const dash = (it.minutes / donutTotal) * 100}
                  <circle
                    cx="60"
                    cy="60"
                    r="48"
                    fill="none"
                    stroke={it.color}
                    stroke-width="14"
                    stroke-dasharray={`${dash} ${100 - dash}`}
                    stroke-dashoffset={-offset}
                    transform="rotate(-90 60 60)"
                    pathLength="100"
                  />
                {/each}
                <text x="60" y="56" text-anchor="middle" class="donut-total">
                  {donutTotal}
                </text>
                <text x="60" y="72" text-anchor="middle" class="donut-unit">
                  min
                </text>
              </svg>
              <ul class="donut-legend">
                {#each donutSlices as it (it.name)}
                  <li>
                    <span
                      class="legend-dot"
                      style:background={it.color}
                    ></span>
                    <span class="legend-name">{it.name}</span>
                    <span class="legend-val">{it.minutes}m</span>
                  </li>
                {/each}
              </ul>
            </div>
          {:else}
            <p class="chart-empty">
              {$t("study.progress.chart_subject_empty")}
            </p>
          {/if}
        </div>
      </article>

      <article class="card chart-card retention-card">
        <header class="card-head">
          <h2>{$t("study.progress.retention_title")}</h2>
          <small class="card-hint">{$t("study.progress.retention_method")}</small>
        </header>
        <div class="chart-body retention-stub">
          <p class="chart-empty">
            {$t("study.progress.retention_stub_before")}
            <a href="/study/anki/stats" class="card-cta inline">
              {$t("study.progress.retention_link")}
            </a>
            {$t("study.progress.retention_stub_after")}
          </p>
        </div>
      </article>
    </section>

    {#if readStats && (readStats.total_seconds > 0 || readStats.active_book)}
      <section class="card read-block">
        <header class="card-head">
          <h2>{$t("study.progress.read_title")}</h2>
        </header>
        <div class="hero-stats">
          <StatCard
            label={$t("study.progress.read_streak")}
            value={readStats.streak}
            hint={$t("study.hub.streak_label")}
            accent={readStats.streak > 0}
          />
          <StatCard
            label={$t("study.progress.read_today")}
            value={fmtDuration(readStats.today_seconds)}
          />
          <StatCard
            label={$t("study.progress.read_week")}
            value={fmtDuration(readStats.week_seconds)}
          />
          <StatCard
            label={$t("study.progress.read_total")}
            value={fmtDuration(readStats.total_seconds)}
          />
        </div>
        {#if readStats.active_book}
          <a class="active-book" href="/study/read/{readStats.active_book.id}">
            <div class="ab-meta">
              <span class="ab-label">{$t("study.progress.read_active")}</span>
              <span class="ab-title">
                {readStats.active_book.title ?? readStats.active_book.file_path.split(/[\\/]/).pop()}
              </span>
            </div>
            <div class="ab-progress">
              <div class="ab-track">
                <div class="ab-fill" style:width="{Math.round(readStats.active_book.reading_pct * 100)}%"></div>
              </div>
              <span class="ab-pct">{Math.round(readStats.active_book.reading_pct * 100)}%</span>
            </div>
          </a>
        {/if}
      </section>
    {/if}

    {#if recentAchievements.length > 0}
      <section class="card achievements-block">
        <header class="card-head">
          <h2>{$t("study.progress.recent_achievements")}</h2>
          <a class="head-link" href="/study/achievements">{$t("study.progress.view_all")}</a>
        </header>
        <ul class="ach-list">
          {#each recentAchievements as a (a.code)}
            <li class="ach-row">
              <span class="ach-tier" style:background={tierColor(a.tier)}>
                {a.tier.slice(0, 1).toUpperCase()}
              </span>
              <div class="ach-meta">
                <span class="ach-name">{achievementLabel(a.code)}</span>
                <span class="ach-when">{fmtRelDays(a.unlocked_at)}</span>
              </div>
            </li>
          {/each}
        </ul>
      </section>
    {/if}
  {/if}
</section>

<style>
  .study-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
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
  .period-row {
    display: flex;
    justify-content: flex-start;
  }

  .hero-today {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: calc(var(--padding) * 2);
    align-items: center;
    padding: calc(var(--padding) * 2);
    border: none;
    border-radius: var(--border-radius);
    background: var(--button-elevated);
  }
  .ring-wrap {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    min-width: 120px;
  }
  .hero-eyebrow {
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .ring-center {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    text-align: center;
  }
  .ring-pct {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-variant-numeric: tabular-nums;
    font-size: 24px;
    font-weight: 500;
    color: var(--secondary);
    line-height: 1;
  }
  .ring-sub {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--tertiary);
    max-width: 80px;
    line-height: 1.2;
  }
  .hero-stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--padding);
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-5);
    border: none;
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: var(--elev-1);
  }
  .card-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--padding);
  }
  .card-head h2 {
    font-size: 14px;
    font-weight: 500;
    margin: 0;
    color: var(--secondary);
  }
  .card-hint {
    font-size: 11px;
    color: var(--tertiary);
  }
  .card-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
    flex-wrap: wrap;
    margin-top: 4px;
  }
  .card-cta {
    font-size: 12px;
    color: var(--accent);
    text-decoration: none;
    padding: 4px 10px;
    border-radius: var(--border-radius);
    transition: background 150ms ease;
  }
  .card-cta:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .card-cta:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }

  .heatmap .grid {
    display: flex;
    gap: 3px;
    overflow-x: auto;
    padding: 4px 0;
  }
  .heatmap .col {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .cell {
    width: 12px;
    height: 12px;
    border-radius: 2px;
    display: inline-block;
  }
  .legend {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--tertiary);
    font-size: 11px;
  }

  .read-block .active-book {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: var(--padding);
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
    text-decoration: none;
    transition: border-color 150ms ease;
    margin-top: 4px;
  }
  .read-block .active-book:hover {
    border-color: var(--accent);
  }
  .ab-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ab-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    color: var(--tertiary);
  }
  .ab-title {
    font-size: 14px;
    color: var(--secondary);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ab-progress {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .ab-track {
    flex: 1;
    height: 3px;
    background: color-mix(in oklab, var(--input-border) 60%, transparent);
    border-radius: 2px;
    overflow: hidden;
  }
  .ab-fill {
    height: 100%;
    background: var(--accent);
    transition: width 500ms ease-out;
  }
  .ab-pct {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    color: var(--tertiary);
  }

  .btn-link {
    display: inline-flex;
    align-items: center;
    padding: 6px 14px;
    font-size: 13px;
    color: var(--on-accent);
    background: var(--accent);
    border-radius: var(--border-radius);
    text-decoration: none;
    transition: filter 150ms ease;
  }
  .btn-link:hover {
    filter: brightness(1.1);
  }
  .btn-link:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }

  @media (max-width: 720px) {
    .hero-today {
      grid-template-columns: 1fr;
      justify-items: center;
    }
    .hero-stats {
      grid-template-columns: repeat(2, 1fr);
      width: 100%;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .ab-fill,
    .card-cta,
    .btn-link,
    .read-block .active-book {
      transition: none;
    }
  }

  .charts-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--padding);
  }
  @media (max-width: 760px) {
    .charts-grid {
      grid-template-columns: 1fr;
    }
  }
  .chart-card {
    display: flex;
    flex-direction: column;
    min-height: 260px;
  }
  .chart-body {
    flex: 1;
    padding: 12px 16px 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .chart-empty {
    margin: auto;
    color: var(--tertiary);
    font-size: 12px;
    text-align: center;
    padding: 20px;
  }
  .card-cta.inline {
    color: var(--accent);
    text-decoration: none;
    font-weight: 600;
  }
  .card-cta.inline:hover {
    text-decoration: underline;
  }

  .hbar-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .hbar-row {
    display: grid;
    grid-template-columns: minmax(60px, 1fr) 2fr max-content;
    gap: 8px;
    align-items: center;
    font-size: 12px;
  }
  .hbar-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--secondary);
  }
  .hbar-track {
    height: 14px;
    background: color-mix(in oklab, var(--input-border) 30%, transparent);
    border-radius: 4px;
    overflow: hidden;
  }
  .hbar-fill {
    height: 100%;
    background: linear-gradient(
      90deg,
      var(--accent),
      color-mix(in oklab, var(--accent) 70%, white)
    );
    transition: width 400ms ease;
  }
  .hbar-value {
    font-family: var(--font-mono, ui-monospace, monospace);
    color: var(--tertiary);
    font-size: 11px;
  }

  .hours-grid {
    display: grid;
    grid-template-columns: repeat(24, 1fr);
    gap: 2px;
    align-items: end;
    height: 140px;
  }
  .hour-bar {
    position: relative;
    height: 100%;
    background: color-mix(in oklab, var(--input-border) 20%, transparent);
    border-radius: 2px;
    overflow: hidden;
    display: flex;
    align-items: flex-end;
  }
  .hour-fill {
    width: 100%;
    background: var(--accent);
    transition: height 400ms ease;
    min-height: 1px;
  }
  .hour-bar.has .hour-fill {
    background: var(--accent);
  }
  .hour-bar:not(.has) {
    opacity: 0.4;
  }
  .hours-axis {
    display: flex;
    justify-content: space-between;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 10px;
    color: var(--tertiary);
    margin-top: 4px;
  }

  .donut-wrap {
    display: grid;
    grid-template-columns: 130px 1fr;
    gap: 16px;
    align-items: center;
  }
  .donut {
    width: 130px;
    height: 130px;
  }
  .donut-total {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 18px;
    font-weight: 600;
    fill: var(--primary);
  }
  .donut-unit {
    font-size: 10px;
    fill: var(--tertiary);
  }
  .donut-legend {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
  }
  .donut-legend li {
    display: grid;
    grid-template-columns: 10px 1fr max-content;
    gap: 8px;
    align-items: center;
  }
  .legend-dot {
    width: 10px;
    height: 10px;
    border-radius: 999px;
    flex-shrink: 0;
  }
  .legend-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--secondary);
  }
  .legend-val {
    color: var(--tertiary);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
  }

  .retention-stub {
    align-items: center;
    justify-content: center;
  }

  @media (prefers-reduced-motion: reduce) {
    .hbar-fill,
    .hour-fill {
      transition: none;
    }
  }

  .achievements-block .head-link {
    margin-left: auto;
    color: var(--accent);
    text-decoration: none;
    font-size: 12px;
  }
  .achievements-block .head-link:hover {
    text-decoration: underline;
  }
  .ach-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 8px;
  }
  .ach-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border: none;
    border-radius: var(--border-radius);
    background: color-mix(in oklab, var(--accent) 4%, transparent);
  }
  .ach-tier {
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    color: white;
    font-weight: 700;
    font-size: 13px;
    flex-shrink: 0;
  }
  .ach-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .ach-name {
    font-size: 13px;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ach-when {
    font-size: 11px;
    color: var(--tertiary);
  }
</style>
