<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import ConfirmDialog from "$lib/study-components/ConfirmDialog.svelte";
  import PageHero from "$lib/study-components/PageHero.svelte";

  type GamificationState = {
    xp: number;
    level: number;
    xp_to_next: number;
    level_progress_pct: number;
    updated_at: number;
  };

  type Achievement = {
    code: string;
    tier: string;
    unlocked_at: number;
    metadata: Record<string, unknown>;
  };

  type XpEntry = {
    source: string;
    amount: number;
    awarded_at: number;
    metadata: Record<string, unknown>;
  };

  type CatalogEntry = {
    code: string;
    family: string;
    labelKey: string;
    descKey: string;
    icon: string;
    threshold: number;
    tier: "bronze" | "silver" | "gold";
    counterKey?: string;
  };

  const CATALOG: CatalogEntry[] = [
    { code: "xp:100", family: "xp", labelKey: "study.achievements.label_xp_100", descKey: "study.achievements.desc_xp_100", icon: "✨", threshold: 100, tier: "bronze" },
    { code: "xp:500", family: "xp", labelKey: "study.achievements.label_xp_500", descKey: "study.achievements.desc_xp_500", icon: "⭐", threshold: 500, tier: "bronze" },
    { code: "xp:1000", family: "xp", labelKey: "study.achievements.label_xp_1000", descKey: "study.achievements.desc_xp_1000", icon: "🌟", threshold: 1000, tier: "silver" },
    { code: "xp:5000", family: "xp", labelKey: "study.achievements.label_xp_5000", descKey: "study.achievements.desc_xp_5000", icon: "💫", threshold: 5000, tier: "silver" },
    { code: "xp:10000", family: "xp", labelKey: "study.achievements.label_xp_10000", descKey: "study.achievements.desc_xp_10000", icon: "🏆", threshold: 10000, tier: "gold" },

    { code: "lessons:1", family: "lessons", labelKey: "study.achievements.label_lessons_1", descKey: "study.achievements.desc_lessons_1", icon: "🎓", threshold: 1, tier: "bronze", counterKey: "lessons_completed" },
    { code: "lessons:10", family: "lessons", labelKey: "study.achievements.label_lessons_10", descKey: "study.achievements.desc_lessons_10", icon: "📚", threshold: 10, tier: "bronze", counterKey: "lessons_completed" },
    { code: "lessons:50", family: "lessons", labelKey: "study.achievements.label_lessons_50", descKey: "study.achievements.desc_lessons_50", icon: "📖", threshold: 50, tier: "silver", counterKey: "lessons_completed" },
    { code: "lessons:100", family: "lessons", labelKey: "study.achievements.label_lessons_100", descKey: "study.achievements.desc_lessons_100", icon: "🥇", threshold: 100, tier: "gold", counterKey: "lessons_completed" },

    { code: "focus:60", family: "focus", labelKey: "study.achievements.label_focus_60", descKey: "study.achievements.desc_focus_60", icon: "🧘", threshold: 60, tier: "bronze", counterKey: "focus_minutes" },
    { code: "focus:600", family: "focus", labelKey: "study.achievements.label_focus_600", descKey: "study.achievements.desc_focus_600", icon: "🎯", threshold: 600, tier: "silver", counterKey: "focus_minutes" },
    { code: "focus:6000", family: "focus", labelKey: "study.achievements.label_focus_6000", descKey: "study.achievements.desc_focus_6000", icon: "🔥", threshold: 6000, tier: "gold", counterKey: "focus_minutes" },

    { code: "streak:3", family: "streak", labelKey: "study.achievements.label_streak_3", descKey: "study.achievements.desc_streak_3", icon: "🔥", threshold: 3, tier: "bronze" },
    { code: "streak:7", family: "streak", labelKey: "study.achievements.label_streak_7", descKey: "study.achievements.desc_streak_7", icon: "🔥🔥", threshold: 7, tier: "bronze" },
    { code: "streak:30", family: "streak", labelKey: "study.achievements.label_streak_30", descKey: "study.achievements.desc_streak_30", icon: "🔥🔥🔥", threshold: 30, tier: "silver" },
    { code: "streak:100", family: "streak", labelKey: "study.achievements.label_streak_100", descKey: "study.achievements.desc_streak_100", icon: "🏅", threshold: 100, tier: "gold" },
  ];

  let xpState = $state<GamificationState | null>(null);
  let unlocked = $state<Achievement[]>([]);
  let history = $state<XpEntry[]>([]);
  let counters = $state<Record<string, number>>({});
  let loading = $state(true);
  let toast = $state<{ kind: "ok" | "err"; msg: string } | null>(null);

  let activeFamily = $state<"all" | "xp" | "lessons" | "focus" | "streak">("all");
  let confirmResetOpen = $state(false);

  function showToast(kind: "ok" | "err", msg: string) {
    toast = { kind, msg };
    setTimeout(() => (toast = null), 2400);
  }

  async function load() {
    loading = true;
    try {
      const [s, ach, hist] = await Promise.all([
        pluginInvoke<GamificationState>("study", "study:gamification:state"),
        pluginInvoke<Achievement[]>(
          "study",
          "study:gamification:achievements",
        ),
        pluginInvoke<XpEntry[]>("study", "study:gamification:history", {
          limit: 30,
        }),
      ]);
      xpState = s;
      unlocked = ach;
      history = hist;

      const counterKeys = ["lessons_completed", "focus_minutes"];
      const counterEntries = await Promise.all(
        counterKeys.map((k) =>
          pluginInvoke<{ value: number }>("study", "study:gamification:counter:get", {
            key: k,
          })
            .then((r) => [k, r.value] as const)
            .catch(() => [k, 0] as const),
        ),
      );
      counters = Object.fromEntries(counterEntries);
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  async function resetAll() {
    try {
      await pluginInvoke("study", "study:gamification:reset");
      confirmResetOpen = false;
      await load();
      showToast("ok", $t("study.achievements.reset_done") as string);
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  function unlockMap(): Map<string, Achievement> {
    return new Map(unlocked.map((a: Achievement) => [a.code, a] as const));
  }

  function progressForCatalog(c: CatalogEntry): { value: number; pct: number } {
    let value = 0;
    if (c.family === "xp") {
      value = xpState?.xp ?? 0;
    } else if (c.counterKey) {
      value = counters[c.counterKey] ?? 0;
    } else if (c.family === "streak") {
      value = 0;
    }
    const pct = Math.min(100, Math.round((value / c.threshold) * 100));
    return { value, pct };
  }

  function fmtSource(s: string): string {
    return s.replace(/_/g, " ");
  }

  function fmtRel(secs: number): string {
    if (!secs) return "—";
    const d = secs * 1000;
    const diff = Date.now() - d;
    const min = Math.floor(diff / 60000);
    if (min < 1) return "·";
    if (min < 60) return $t("study.achievements.minutes_ago", { count: min }) as string;
    const h = Math.floor(min / 60);
    if (h < 24) return $t("study.achievements.hours_ago", { count: h }) as string;
    const days = Math.floor(h / 24);
    if (days < 30) {
      const key = days === 1 ? "study.achievements.day_ago" : "study.achievements.days_ago";
      return $t(key, { count: days }) as string;
    }
    return new Date(d).toLocaleDateString();
  }

  const filteredCatalog = $derived.by(() => {
    if (activeFamily === "all") return CATALOG;
    return CATALOG.filter((c) => c.family === activeFamily);
  });

  const counts = $derived.by(() => {
    const total = CATALOG.length;
    const got = CATALOG.filter((c) => unlockMap().has(c.code)).length;
    return { total, got };
  });

  const tierStats = $derived.by(() => {
    const u = unlockMap();
    const stats = { bronze: 0, silver: 0, gold: 0 };
    for (const c of CATALOG) {
      if (u.has(c.code)) {
        const ach = u.get(c.code)!;
        const tier = (ach.tier as "bronze" | "silver" | "gold") || c.tier;
        if (tier in stats) stats[tier as keyof typeof stats]++;
      }
    }
    return stats;
  });

  const families = $derived([
    { v: "all", label: $t("study.achievements.filter_all"), icon: "🌐" },
    { v: "xp", label: $t("study.achievements.filter_xp"), icon: "✨" },
    { v: "lessons", label: $t("study.achievements.filter_lessons"), icon: "📚" },
    { v: "focus", label: $t("study.achievements.filter_focus"), icon: "🧘" },
    { v: "streak", label: $t("study.achievements.filter_streak"), icon: "🔥" },
  ]);

  onMount(load);
</script>

<section class="ach-page">
  <PageHero title={$t("study.achievements.title")} subtitle={$t("study.achievements.subtitle")} />

  {#if toast}
    <div class="toast" class:err={toast.kind === "err"} role="status">
      {toast.msg}
    </div>
  {/if}

  {#if loading}
    <div class="state">{$t("study.achievements.loading")}</div>
  {:else if xpState}
    <section class="hero">
      <div class="level-block">
        <div class="level-tag">{$t("study.achievements.level_tag")}</div>
        <div class="level-num">{xpState.level}</div>
      </div>
      <div class="xp-block">
        <div class="xp-row">
          <strong>{xpState.xp.toLocaleString()} XP</strong>
          <span class="next">
            {xpState.xp_to_next.toLocaleString()} {$t("study.achievements.to_next", { next: xpState.level + 1 })}
          </span>
        </div>
        <div class="xp-bar">
          <div
            class="xp-bar-fill"
            style:width={`${xpState.level_progress_pct}%`}
          ></div>
        </div>
        <div class="xp-meta">{$t("study.achievements.level_progress", { pct: xpState.level_progress_pct })}</div>
      </div>

      <div class="ach-summary">
        <div class="sum-num">
          <strong>{counts.got}</strong>
          <span>/{counts.total}</span>
        </div>
        <div class="sum-tiers">
          <span class="t-pill bronze">{tierStats.bronze} {$t("study.achievements.tier_bronze")}</span>
          <span class="t-pill silver">{tierStats.silver} {$t("study.achievements.tier_silver")}</span>
          <span class="t-pill gold">{tierStats.gold} {$t("study.achievements.tier_gold")}</span>
        </div>
      </div>
    </section>

    <nav class="filters" aria-label={$t("study.achievements.filter_aria") as string}>
      {#each families as f (f.v)}
        <button
          class="filter-btn"
          class:active={activeFamily === f.v}
          onclick={() => (activeFamily = f.v as typeof activeFamily)}
        >
          <span class="f-icon">{f.icon}</span>
          <span>{f.label}</span>
        </button>
      {/each}
      <a class="filter-btn link" href="/study/achievements/tree">
        <span class="f-icon">🌳</span>
        <span>{$t("study.achievements.tree_link")}</span>
      </a>
      <a class="filter-btn link" href="/study/achievements/charts">
        <span class="f-icon">📈</span>
        <span>{$t("study.achievements.charts_link")}</span>
      </a>
    </nav>

    <div class="grid">
      {#each filteredCatalog as c (c.code)}
        {@const u = unlockMap().get(c.code)}
        {@const isUnlocked = !!u}
        {@const prog = progressForCatalog(c)}
        <article class="card" class:unlocked={isUnlocked} data-tier={c.tier}>
          <div class="icon">{c.icon}</div>
          <div class="card-body">
            <h3>{$t(c.labelKey)}</h3>
            <p>{$t(c.descKey)}</p>
            {#if isUnlocked}
              <div class="unlocked-meta">
                <span class="tier-pill" data-tier={c.tier}>{$t(`study.achievements.tier_${c.tier}`)}</span>
                <span class="when">{$t("study.achievements.unlocked_prefix")} {fmtRel(u.unlocked_at)}</span>
              </div>
            {:else}
              <div class="prog">
                <div class="prog-bar">
                  <div class="prog-fill" style:width={`${prog.pct}%`}></div>
                </div>
                <span class="prog-text">
                  {prog.value.toLocaleString()} / {c.threshold.toLocaleString()}
                </span>
              </div>
            {/if}
          </div>
        </article>
      {/each}
    </div>

    <section class="history-section">
      <h2 class="section-title">{$t("study.achievements.history_title")}</h2>
      {#if history.length === 0}
        <p class="empty">{$t("study.achievements.history_empty")}</p>
      {:else}
        <ul class="history-list">
          {#each history as h, i (i + h.awarded_at)}
            <li class="hist-row">
              <span class="hist-source">{fmtSource(h.source)}</span>
              <span class="hist-amount" class:pos={h.amount > 0} class:neg={h.amount < 0}>
                {h.amount > 0 ? "+" : ""}{h.amount} XP
              </span>
              <span class="hist-when">{fmtRel(h.awarded_at)}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <footer class="ach-foot">
      <button
        class="btn ghost danger"
        onclick={() => (confirmResetOpen = true)}
      >
        {$t("study.achievements.reset_title")}
      </button>
      <span class="foot-hint">
        {$t("study.achievements.reset_desc")}
      </span>
    </footer>
  {/if}
</section>

<ConfirmDialog
  bind:open={confirmResetOpen}
  title={$t("study.achievements.reset_confirm_title") as string}
  message={$t("study.achievements.reset_confirm_message") as string}
  confirmLabel={$t("study.achievements.reset_confirm_label") as string}
  variant="danger"
  onConfirm={resetAll}
/>

<style>
  .ach-page {
    display: flex;
    flex-direction: column;
    gap: 20px;
    width: 100%;
    max-width: 1100px;
    margin-inline: auto;
    padding: 16px 20px;
  }
  .state {
    padding: 60px;
    text-align: center;
    color: var(--tertiary);
  }

  .hero {
    display: grid;
    grid-template-columns: max-content 1fr max-content;
    gap: 24px;
    align-items: center;
    padding: 20px 24px;
    background: linear-gradient(
      135deg,
      color-mix(in oklab, var(--accent) 14%, var(--surface)) 0%,
      var(--surface) 100%
    );
    border: 1px solid color-mix(in oklab, var(--accent) 30%, var(--input-border));
    border-radius: var(--border-radius);
  }
  @media (max-width: 760px) {
    .hero {
      grid-template-columns: 1fr;
      text-align: center;
    }
  }
  .level-block {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 4px 16px;
    border-right: none;
  }
  @media (max-width: 760px) {
    .level-block {
      border-right: 0;
      border-bottom: none;
      padding-bottom: 12px;
    }
  }
  .level-tag {
    font-size: var(--text-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-muted);
  }
  .level-num {
    font-size: 56px;
    font-weight: 700;
    color: var(--accent);
    line-height: 1;
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .xp-block {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .xp-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }
  .xp-row strong {
    font-size: 22px;
    font-weight: 600;
  }
  .next {
    color: var(--tertiary);
    font-size: 12px;
  }
  .xp-bar {
    height: 10px;
    background: color-mix(in oklab, var(--input-border) 30%, transparent);
    border-radius: 999px;
    overflow: hidden;
  }
  .xp-bar-fill {
    height: 100%;
    background: linear-gradient(
      90deg,
      var(--accent),
      color-mix(in oklab, var(--accent) 70%, white)
    );
    transition: width 400ms ease;
  }
  .xp-meta {
    font-size: 11px;
    color: var(--tertiary);
  }
  .ach-summary {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 6px;
  }
  @media (max-width: 760px) {
    .ach-summary {
      align-items: center;
    }
  }
  .sum-num {
    display: flex;
    align-items: baseline;
    gap: 2px;
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .sum-num strong {
    font-size: 28px;
    color: var(--text);
  }
  .sum-num span {
    font-size: 16px;
    color: var(--tertiary);
  }
  .sum-tiers {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .t-pill {
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-weight: 600;
  }
  .t-pill.bronze {
    background: color-mix(in oklab, #cd7f32 20%, transparent);
    color: oklch(60% 0.13 50);
  }
  .t-pill.silver {
    background: color-mix(in oklab, #a8a8a8 20%, transparent);
    color: oklch(70% 0.02 240);
  }
  .t-pill.gold {
    background: color-mix(in oklab, #ffd700 22%, transparent);
    color: oklch(72% 0.16 90);
  }

  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .filter-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border: none;
    border-radius: 999px;
    background: transparent;
    color: var(--secondary);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    transition: border-color 120ms ease, background 120ms ease;
  }
  .filter-btn:hover {
    border-color: var(--accent);
  }
  .filter-btn.active {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }
  .filter-btn.link {
    text-decoration: none;
  }
  .filter-btn.link:first-of-type {
    margin-left: auto;
  }
  .f-icon {
    font-size: 14px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 12px;
  }
  .card {
    display: flex;
    gap: var(--space-3);
    padding: var(--space-4);
    border: none;
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: var(--elev-1);
    transition: transform var(--duration-fast) var(--ease-out), border-color var(--duration-fast) var(--ease-out), box-shadow var(--duration-fast) var(--ease-out);
  }
  .card.unlocked {
    border-color: color-mix(in oklab, var(--accent) 50%, var(--input-border));
    background: linear-gradient(
      135deg,
      color-mix(in oklab, var(--accent) 8%, var(--surface)),
      var(--surface)
    );
  }
  .card.unlocked[data-tier="gold"] {
    border-color: oklch(72% 0.16 90);
    box-shadow: 0 0 0 1px oklch(72% 0.16 90 / 0.2),
      inset 0 0 24px oklch(72% 0.16 90 / 0.05);
  }
  .card.unlocked[data-tier="silver"] {
    border-color: oklch(70% 0.02 240);
  }
  .card:not(.unlocked) .icon {
    opacity: 0.55;
  }
  .icon {
    flex-shrink: 0;
    font-size: 28px;
    width: 48px;
    height: 48px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in oklab, var(--input-border) 30%, transparent);
    border-radius: 8px;
    filter: grayscale(0.5);
  }
  .card.unlocked .icon {
    filter: none;
    background: color-mix(in oklab, var(--accent) 12%, transparent);
  }
  .card-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .card h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .card p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    line-height: 1.4;
  }
  .unlocked-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 4px;
  }
  .tier-pill {
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-weight: 600;
  }
  .tier-pill[data-tier="bronze"] {
    background: color-mix(in oklab, #cd7f32 20%, transparent);
    color: oklch(60% 0.13 50);
  }
  .tier-pill[data-tier="silver"] {
    background: color-mix(in oklab, #a8a8a8 20%, transparent);
    color: oklch(70% 0.02 240);
  }
  .tier-pill[data-tier="gold"] {
    background: color-mix(in oklab, #ffd700 22%, transparent);
    color: oklch(72% 0.16 90);
  }
  .when {
    font-size: 10px;
    color: var(--tertiary);
  }
  .prog {
    display: flex;
    flex-direction: column;
    gap: 3px;
    margin-top: 6px;
  }
  .prog-bar {
    height: 4px;
    background: color-mix(in oklab, var(--input-border) 30%, transparent);
    border-radius: 999px;
    overflow: hidden;
  }
  .prog-fill {
    height: 100%;
    background: var(--accent);
  }
  .prog-text {
    font-size: 10px;
    color: var(--tertiary);
    font-family: var(--font-mono, ui-monospace, monospace);
  }

  .history-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .history-section h2 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--tertiary);
  }
  .empty {
    color: var(--tertiary);
    font-size: 13px;
    margin: 0;
  }
  .history-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    overflow: hidden;
  }
  .hist-row {
    display: grid;
    grid-template-columns: 1fr max-content max-content;
    align-items: center;
    gap: 12px;
    padding: 8px 14px;
    font-size: 13px;
    border-bottom: none;
  }
  .hist-row:last-child {
    border-bottom: 0;
  }
  .hist-source {
    color: var(--text);
  }
  .hist-amount {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-weight: 600;
  }
  .hist-amount.pos {
    color: var(--success, var(--accent));
  }
  .hist-amount.neg {
    color: var(--error, var(--accent));
  }
  .hist-when {
    color: var(--tertiary);
    font-size: 11px;
  }

  .ach-foot {
    display: flex;
    align-items: center;
    gap: 12px;
    padding-top: 12px;
    border-top: none;
  }
  .foot-hint {
    color: var(--tertiary);
    font-size: 11px;
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

  .btn {
    padding: 6px 12px;
    border-radius: var(--border-radius);
    border: 1px solid transparent;
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: background 120ms ease;
  }
  .btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--text);
  }
  .btn.ghost:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .btn.ghost.danger {
    color: var(--error, var(--accent));
    border-color: color-mix(
      in oklab,
      var(--error, var(--accent)) 40%,
      var(--input-border)
    );
  }
</style>
