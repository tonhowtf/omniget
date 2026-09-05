<script lang="ts">
  /**
   * Métricas de um perfil (engajamento, ritmo, melhores dias e horas,
   * hashtags, top posts) e comparação lado a lado entre perfis.
   * `mode` = "analytics" | "compare".
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount, untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openUrl, type ToolProgress } from "$lib/tools/rt";
  import { cancelJob, compact, exportCsv, fmtDay, igState, jobId, n, recall, remember, slugArg, type ProfileStats } from "$lib/tools/ig.svelte";
  import IgAccountRow from "./IgAccountRow.svelte";

  let { mode = "analytics" }: { mode?: "analytics" | "compare" } = $props();
  let users = $state(untrack(() => mode) === "compare" ? recall("compare") : "");
  let limit = $state(Number(recall("analytics_limit", "36")));
  let busy = $state(false);
  let job = $state("");
  let progress = $state<ToolProgress | null>(null);
  let stats = $state<ProfileStats[]>([]);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (job && p.id === `ig:${job}`) progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  const DAYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

  async function run() {
    const list = users.split(/[\s,]+/).map((s) => s.trim().replace(/^@/, "")).filter(Boolean);
    if (!list.length || busy) return;
    remember("analytics_limit", String(limit));
    if (mode === "compare") remember("compare", users);
    busy = true;
    stats = [];
    job = jobId("stats");
    try {
      stats = await invoke<ProfileStats[]>("tool_ig_analytics", { slug: slugArg(), users: mode === "compare" ? list.slice(0, 6) : [list[0]], postsLimit: limit, job });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  const pct = (v: number) => `${v.toFixed(1)}%`;
  const f1 = (v: number) => v.toFixed(1);

  const ROWS: [string, (s: ProfileStats) => string][] = [
    ["followers", (s) => n(s.user.follower_count)],
    ["following", (s) => n(s.user.following_count)],
    ["posts", (s) => n(s.user.media_count)],
    ["follow_ratio", (s) => f1(s.follow_ratio)],
    ["engagement", (s) => pct(s.engagement_rate)],
    ["avg_likes", (s) => n(Math.round(s.avg_likes))],
    ["avg_comments", (s) => f1(s.avg_comments)],
    ["avg_plays", (s) => n(Math.round(s.avg_plays))],
    ["comment_ratio", (s) => pct(s.comment_ratio)],
    ["posts_per_week", (s) => f1(s.posts_per_week)],
    ["share_video", (s) => pct(s.share_video)],
    ["share_carousel", (s) => pct(s.share_carousel)],
    ["avg_hashtags", (s) => f1(s.avg_hashtags)],
    ["avg_caption", (s) => n(Math.round(s.avg_caption_len))],
    ["best_day", (s) => $t(`tools.ig.analytics.day_${DAYS[s.best_weekday]}`) as string],
    ["best_hour", (s) => `${s.best_hour}h`],
    ["paid", (s) => String(s.paid_partnerships)],
  ];

  async function csv() {
    const p = await exportCsv("instagram-analytics", ["metric", ...stats.map((s) => `@${s.user.username}`)], ROWS.map(([k, fn]) => [$t(`tools.ig.analytics.m_${k}`) as string, ...stats.map(fn)]));
    if (p) showToast("success", $t("tools.common.done") as string);
  }
</script>

<div class="tool">
  <IgAccountRow />
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          {#if mode === "compare"}
            <textarea class="input" rows="3" bind:value={users} placeholder={$t("tools.ig.analytics.compare_placeholder")}></textarea>
          {:else}
            <input class="input" type="text" bind:value={users} placeholder={$t("tools.ig.common.user_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} />
          {/if}
        </div>
        <div class="group-row-trailing btn-row">
          <select class="select" bind:value={limit}><option value={12}>12</option><option value={36}>36</option><option value={72}>72</option><option value={150}>150</option></select>
          {#if busy}<button class="btn btn-secondary btn-sm" type="button" onclick={() => cancelJob(job)}>{$t("tools.ig.common.cancel")}</button>
          {:else}<button class="btn btn-primary" type="button" disabled={!users.trim() || !igState.me} onclick={run}>{$t("tools.ig.analytics.run")}</button>{/if}
        </div>
      </div>
      <div class="group-row"><div class="group-row-sub">{$t("tools.ig.analytics.hint")}{#if busy && progress} · {progress.stage} {progress.done}{/if}</div></div>
    </div>
  </section>

  {#if stats.length}
    <section>
      <div class="head"><span class="group-label">{$t("tools.ig.analytics.metrics")}</span><button class="btn btn-secondary btn-sm" type="button" onclick={csv}>CSV</button></div>
      <div class="group">
        <div class="table-wrap">
          <table>
            <thead><tr><th></th>{#each stats as s (s.user.pk)}<th><button class="link" type="button" onclick={() => openUrl(`https://www.instagram.com/${s.user.username}/`)}>@{s.user.username}</button><div class="sub">{s.posts_analyzed} {$t("tools.ig.common.posts")} · {fmtDay(s.first_post_at)} → {fmtDay(s.last_post_at)}</div></th>{/each}</tr></thead>
            <tbody>
              {#each ROWS as [k, fn] (k)}
                <tr><td>{$t(`tools.ig.analytics.m_${k}`)}</td>{#each stats as s (s.user.pk)}<td class="num">{fn(s)}</td>{/each}</tr>
              {/each}
            </tbody>
          </table>
        </div>
      </div>
    </section>
    {#each stats as s (s.user.pk)}
      {#if s.posts_analyzed}
        <section>
          <span class="group-label">@{s.user.username} · {$t("tools.ig.analytics.when")}</span>
          <div class="charts">
            <div class="chart">
              <div class="chart-title">{$t("tools.ig.analytics.weekday")}</div>
              <div class="bars">
                {#each s.weekday_engagement as v, i (i)}
                  {@const max = Math.max(1, ...s.weekday_engagement)}
                  <div class="bar" class:best={i === s.best_weekday} title="{compact(Math.round(v))} · {s.weekday_counts[i]} posts"><div class="fill" style:height="{(v / max) * 100}%"></div><span>{$t(`tools.ig.analytics.day_${DAYS[i]}`).slice(0, 3)}</span></div>
                {/each}
              </div>
            </div>
            <div class="chart">
              <div class="chart-title">{$t("tools.ig.analytics.hour")}</div>
              <div class="bars">
                {#each s.hour_counts as v, i (i)}
                  {@const max = Math.max(1, ...s.hour_counts)}
                  <div class="bar" class:best={i === s.best_hour} title="{i}h · {v}"><div class="fill" style:height="{(v / max) * 100}%"></div>{#if i % 6 === 0}<span>{i}h</span>{/if}</div>
                {/each}
              </div>
            </div>
          </div>
          {#if s.top_hashtags.length}
            <div class="chips">{#each s.top_hashtags as [h, c] (h)}<span class="tag">#{h} · {c}</span>{/each}</div>
          {/if}
          {#if s.top_posts.length}
            <div class="top">
              {#each s.top_posts as p (p.code)}
                <button class="post" type="button" onclick={() => openUrl(p.url)}>
                  <img src={p.thumbnail} alt="" loading="lazy" />
                  <div class="pm">♥ {compact(p.likes)} · 💬 {compact(p.comments)}{#if p.plays} · ▶ {compact(p.plays)}{/if}</div>
                </button>
              {/each}
            </div>
          {/if}
        </section>
      {/if}
    {/each}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .head { display: flex; justify-content: space-between; align-items: center; }
  textarea.input { width: 100%; resize: vertical; }
  .table-wrap { overflow-x: auto; }
  table { width: 100%; border-collapse: collapse; font-size: var(--text-sm); }
  th, td { padding: var(--space-2) var(--space-3); text-align: left; border-bottom: var(--hairline) solid var(--content-border); vertical-align: top; }
  th .sub { font-weight: 400; font-size: var(--text-xs); color: var(--text-muted); }
  td.num { font-variant-numeric: tabular-nums; text-align: right; }
  .link { border: 0; background: none; color: var(--accent-hi); cursor: pointer; padding: 0; font-weight: 600; font-size: inherit; }
  .charts { display: grid; grid-template-columns: 1fr 2fr; gap: var(--space-3); }
  @media (max-width: 720px) { .charts { grid-template-columns: 1fr; } }
  .chart { padding: var(--space-3); background: var(--surface); border-radius: var(--radius-lg); box-shadow: inset 0 0 0 var(--hairline) var(--content-border); }
  .chart-title { font-size: var(--text-xs); color: var(--text-muted); margin-bottom: var(--space-2); }
  .bars { display: flex; align-items: flex-end; gap: 3px; height: 96px; }
  .bar { flex: 1; display: flex; flex-direction: column; justify-content: flex-end; align-items: center; height: 100%; gap: 2px; }
  .fill { width: 100%; background: var(--accent-soft); border-radius: 3px 3px 0 0; min-height: 2px; }
  .bar.best .fill { background: var(--accent); }
  .bar span { font-size: 9px; color: var(--text-muted); }
  .chips { display: flex; flex-wrap: wrap; gap: var(--space-1); margin-top: var(--space-3); }
  .top { display: grid; grid-template-columns: repeat(6, 1fr); gap: var(--space-2); margin-top: var(--space-3); }
  .post { border: 0; padding: 0; background: none; cursor: pointer; color: var(--text-muted); font-size: var(--text-xs); }
  .post img { width: 100%; aspect-ratio: 1; object-fit: cover; border-radius: var(--radius-md, 8px); display: block; }
  .pm { margin-top: 2px; }
</style>
