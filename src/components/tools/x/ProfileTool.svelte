<script lang="ts">
  /** Raio-X de perfil (estudo 67): engajamento, melhor horário, top posts, hashtags. */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { openUrl, reveal, saveAs } from "$lib/tools/rt";
  import { fmtDate, fmtN, xErr, type XPost, type XUser } from "$lib/tools/x";
  import PostCard from "./PostCard.svelte";

  type Slot = { key: number; posts: number; avg_likes: number };
  type Tag = { tag: string; count: number };
  type Report = {
    user: XUser; sampled: number; since: string; until: string; days_spanned: number; posts_per_day: number;
    avg_likes: number; median_likes: number; avg_reposts: number; avg_replies: number; avg_views: number; engagement_rate: number;
    reply_share: number; repost_share: number; media_share: number; link_share: number;
    by_hour: Slot[]; by_weekday: Slot[]; best_hour: number | null; best_weekday: number | null;
    top_posts: XPost[]; top_hashtags: Tag[]; top_mentions: Tag[]; utc_offset_minutes: number;
  };

  let input = $state("");
  let limit = $state(200);
  let withReplies = $state(false);
  let busy = $state(false);
  let report = $state<Report | null>(null);

  async function run() {
    if (!input.trim() || busy) return;
    busy = true;
    report = null;
    try {
      report = await invoke<Report>("tool_x_profile", { input, limit, withReplies });
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = false;
    }
  }

  async function exportJson() {
    if (!report) return;
    const dest = await saveAs(`x-profile-${report.user.handle}.json`);
    if (!dest) return;
    try {
      await invoke("tool_x_write_text", { dest, content: JSON.stringify(report, null, 2) });
      await reveal(dest);
    } catch (e) {
      showToast("error", xErr(e));
    }
  }

  const WD = ["dom", "seg", "ter", "qua", "qui", "sex", "sáb"];
  let maxHour = $derived(Math.max(1, ...(report?.by_hour.map((s) => s.avg_likes) ?? [1])));
  let maxWd = $derived(Math.max(1, ...(report?.by_weekday.map((s) => s.avg_likes) ?? [1])));
  const f1 = (n: number) => (n >= 100 ? Math.round(n).toLocaleString() : n.toFixed(1));
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="text" bind:value={input} placeholder={$t("tools.x.handle_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} /></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !input.trim()} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.x.analyze")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.x.sample")}</div><div class="group-row-sub">{$t("tools.x.sample_hint")}</div></div>
        <div class="group-row-trailing btn-row">
          <div class="segmented">{#each [100, 200, 500] as n (n)}<button class="segmented-btn" class:active={limit === n} type="button" onclick={() => (limit = n)}>{n}</button>{/each}</div>
          <label class="opt"><input class="checkbox" type="checkbox" bind:checked={withReplies} /> {$t("tools.x.with_replies")}</label>
        </div>
      </div>
    </div>
  </section>

  {#if report}
    {@const u = report.user}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content user">
            {#if u.avatar}<img class="avatar" src={u.avatar} alt="" />{/if}
            <div>
              <div class="group-row-title">{u.name} <span class="dim">@{u.handle}</span>{#if u.verified} <span class="tag tag-accent">✓</span>{/if}</div>
              <div class="group-row-sub">{u.bio}</div>
              <div class="group-row-sub">{fmtN(u.followers)} {$t("tools.x.followers")} · {fmtN(u.following)} {$t("tools.x.following")} · {fmtN(u.posts)} {$t("tools.x.posts")} · {$t("tools.x.joined")} {fmtDate(u.joined).slice(0, 12)}{#if u.location} · {u.location}{/if}</div>
            </div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => openUrl(`https://x.com/${u.handle}`)}>{$t("tools.x.open_on_x")}</button>
            <button class="btn btn-secondary btn-sm" type="button" onclick={exportJson}>JSON</button>
          </div>
        </div>
      </div>
    </section>

    <section>
      <span class="group-label">{$t("tools.x.engagement")} · {report.sampled} {$t("tools.x.posts")} · {report.days_spanned.toFixed(0)} {$t("tools.x.days")}</span>
      <div class="group"><div class="group-row"><div class="tiles">
        <div class="tile"><div class="v">{f1(report.avg_likes)}</div><div class="k">{$t("tools.x.avg_likes")}</div></div>
        <div class="tile"><div class="v">{f1(report.median_likes)}</div><div class="k">{$t("tools.x.median_likes")}</div></div>
        <div class="tile"><div class="v">{f1(report.avg_reposts)}</div><div class="k">{$t("tools.x.avg_reposts")}</div></div>
        <div class="tile"><div class="v">{f1(report.avg_replies)}</div><div class="k">{$t("tools.x.avg_replies")}</div></div>
        <div class="tile"><div class="v">{fmtN(Math.round(report.avg_views))}</div><div class="k">{$t("tools.x.avg_views")}</div></div>
        <div class="tile"><div class="v">{report.engagement_rate.toFixed(2)}%</div><div class="k">{$t("tools.x.engagement_rate")}</div></div>
        <div class="tile"><div class="v">{report.posts_per_day.toFixed(1)}</div><div class="k">{$t("tools.x.posts_per_day")}</div></div>
        <div class="tile"><div class="v">{report.media_share.toFixed(0)}%</div><div class="k">{$t("tools.x.media_share")}</div></div>
        <div class="tile"><div class="v">{report.reply_share.toFixed(0)}%</div><div class="k">{$t("tools.x.reply_share")}</div></div>
        <div class="tile"><div class="v">{report.link_share.toFixed(0)}%</div><div class="k">{$t("tools.x.link_share")}</div></div>
      </div></div></div>
    </section>

    <section>
      <span class="group-label">{$t("tools.x.best_time")}{#if report.best_hour !== null} · {String(report.best_hour).padStart(2, "0")}h{/if}{#if report.best_weekday !== null} · {WD[report.best_weekday]}{/if}</span>
      <div class="group">
        <div class="group-row"><div class="group-row-content">
          <div class="group-row-sub">{$t("tools.x.by_hour")}</div>
          <div class="bars">
            {#each report.by_hour as s (s.key)}
              <div class="bar" class:best={s.key === report.best_hour} title="{s.key}h: {s.posts} posts · {f1(s.avg_likes)} ♥"><div class="bar-fill" style:height="{Math.max(2, (s.avg_likes / maxHour) * 100)}%"></div><span class="bar-label">{s.key % 3 === 0 ? s.key : ""}</span></div>
            {/each}
          </div>
        </div></div>
        <div class="group-row"><div class="group-row-content">
          <div class="group-row-sub">{$t("tools.x.by_weekday")}</div>
          <div class="bars wd">
            {#each report.by_weekday as s (s.key)}
              <div class="bar" class:best={s.key === report.best_weekday} title="{WD[s.key]}: {s.posts} posts · {f1(s.avg_likes)} ♥"><div class="bar-fill" style:height="{Math.max(2, (s.avg_likes / maxWd) * 100)}%"></div><span class="bar-label">{WD[s.key]}</span></div>
            {/each}
          </div>
        </div></div>
      </div>
    </section>

    {#if report.top_hashtags.length || report.top_mentions.length}
      <section>
        <div class="group"><div class="group-row"><div class="group-row-content chips">
          {#each report.top_hashtags as h (h.tag)}<span class="tag">#{h.tag} · {h.count}</span>{/each}
          {#each report.top_mentions as m (m.tag)}<span class="tag tag-accent">@{m.tag} · {m.count}</span>{/each}
        </div></div></div>
      </section>
    {/if}

    <section>
      <span class="group-label">{$t("tools.x.top_posts")}</span>
      <div class="group">{#each report.top_posts as p (p.id)}<PostCard post={p} compact />{/each}</div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .opt { display: inline-flex; align-items: center; gap: var(--space-1); font-size: var(--text-sm); }
  .user { display: flex; gap: var(--space-3); align-items: flex-start; }
  .avatar { width: 56px; height: 56px; border-radius: 50%; flex-shrink: 0; }
  .dim { color: var(--text-muted); font-weight: 400; }
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); gap: var(--space-2); width: 100%; }
  .tile { padding: var(--space-2) var(--space-3); border-radius: var(--radius-md); background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .tile .v { font-family: var(--font-display); font-size: var(--text-lg); font-weight: 700; color: var(--text); }
  .tile .k { font-size: var(--text-xs); color: var(--text-muted); }
  .bars { display: flex; align-items: flex-end; gap: 3px; height: 90px; width: 100%; margin-top: var(--space-1); }
  .bars.wd { height: 70px; gap: var(--space-2); }
  .bar { flex: 1; display: flex; flex-direction: column; justify-content: flex-end; height: 100%; position: relative; }
  .bar-fill { background: color-mix(in srgb, var(--accent) 55%, transparent); border-radius: 3px 3px 0 0; }
  .bar.best .bar-fill { background: var(--accent); }
  .bar-label { font-size: 10px; color: var(--text-muted); text-align: center; height: 14px; }
  .chips { display: flex; flex-wrap: wrap; gap: var(--space-1); }
</style>
