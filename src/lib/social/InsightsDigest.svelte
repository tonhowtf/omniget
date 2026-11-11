<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open as shellOpen } from "@tauri-apps/plugin-shell";
  import { t, locale } from "$lib/i18n";
  import SocialIcon from "./SocialIcon.svelte";
  import { ago, compact, social, tool, type Platform } from "./api";
  import { toolError, weekdayName, type ToolError } from "./insights";
  import { recParams, signed, type Digest, type Recommendation, type ReportResult } from "./second";

  let { platform, ready, openProfile }: { platform: Platform; ready: boolean; openProfile: (handle: string) => void } = $props();

  const KNOWN = ["best_time", "media_type", "frequency_drop", "not_following_back", "new_unfollowers", "unread_hits", "drafts", "missed_posts"];

  let digest = $state<Digest | null>(null);
  let loading = $state(false);
  let error = $state<ToolError | null>(null);
  let marking = $state(false);
  let reporting = $state(false);
  let report = $state<ReportResult | null>(null);
  let reportError = $state<ToolError | null>(null);
  let why = $state(false);
  let asked = "";

  async function load(mark = false) {
    if (mark) marking = true;
    else loading = true;
    error = null;
    const mine = platform;
    try {
      const r = await social.call<Digest>("social_digest", { platform: mine, mark });
      if (mine === platform) digest = r;
    } catch (e) {
      if (mine === platform) error = toolError(e);
    } finally {
      loading = false;
      marking = false;
    }
  }

  async function createReport() {
    if (reporting) return;
    reporting = true;
    reportError = null;
    report = null;
    try {
      report = await social.call<ReportResult>(tool(platform, "report"), {});
      openFile(report.path);
    } catch (e) {
      reportError = toolError(e);
    } finally {
      reporting = false;
    }
  }

  function openFile(path: string) {
    void invoke("open_path_default", { path }).catch(() => {});
  }

  function external(url: string) {
    if (url) void shellOpen(url).catch(() => {});
  }

  function recText(r: Recommendation): string {
    if (!KNOWN.includes(r.id)) return r.message;
    const params = recParams(r, (i) => weekdayName(i, $locale ?? undefined));
    if (typeof params.media === "string") params.media = $t(`social_analytics.media.${params.media}`) as string;
    return $t(`social_analytics.extras.rec.${r.id}`, params) as string;
  }

  $effect(() => {
    if (ready && asked !== platform) {
      asked = platform;
      digest = null;
      report = null;
      void load();
    }
  });
</script>

<section class="dg" aria-labelledby="digest-title">
  <header class="dg-head">
    <h3 class="dg-title" id="digest-title">{$t("social_analytics.extras.digest.title")}</h3>
    {#if digest}
      <span class="dg-dim">{digest.first ? $t("social_analytics.extras.digest.first") : $t("social_analytics.extras.digest.since", { time: ago(digest.since, $locale ?? undefined) })}</span>
    {/if}
    <span class="dg-grow"></span>
    <button class="btn btn-ghost btn-sm" disabled={loading || !ready} onclick={() => void load()} title={$t("social_analytics.extras.digest.refresh")} aria-label={$t("social_analytics.extras.digest.refresh")}><SocialIcon name="refresh" /></button>
    <button class="btn btn-ghost btn-sm" disabled={!digest || marking || loading} onclick={() => void load(true)}>{$t("social_analytics.extras.digest.mark")}</button>
    <button class="btn btn-secondary btn-sm" disabled={reporting} onclick={() => void createReport()} title={$t("social_analytics.extras.report.note")}>
      {#if reporting}<span class="spinner"></span>{:else}<SocialIcon name="chart" />{/if} {$t(reporting ? "social_analytics.extras.report.creating" : "social_analytics.extras.report.create")}
    </button>
  </header>

  {#if report}
    <div class="notice notice-success" role="status">
      <div class="notice-body">
        <div class="notice-text">{$t("social_analytics.extras.report.done", { path: report.path })}</div>
        <div class="notice-text dg-dim">{$t("social_analytics.extras.report.note")}</div>
      </div>
      <button class="btn btn-ghost btn-sm" onclick={() => report && openFile(report.path)}>{$t("social_analytics.extras.report.open")}</button>
    </div>
  {/if}
  {#if reportError}
    <div class="notice notice-warning" role="alert">
      <div class="notice-body">
        <div class="notice-text">{reportError.message}</div>
        {#if reportError.next_step}<div class="notice-text dg-dim">{reportError.next_step}</div>{/if}
      </div>
    </div>
  {/if}

  {#if error}
    <div class="notice notice-warning" role="alert">
      <div class="notice-body">
        <div class="notice-text">{error.message}</div>
        {#if error.next_step}<div class="notice-text dg-dim">{error.next_step}</div>{/if}
      </div>
      <button class="btn btn-ghost btn-sm" onclick={() => void load()}>{$t("social.common.retry")}</button>
    </div>
  {:else if !digest}
    <div class="dg-state" role="status"><span class="spinner"></span><span>{$t("social_analytics.extras.digest.loading")}</span></div>
  {:else}
    {#if digest.marked}
      <p class="dg-dim" role="status">{$t("social_analytics.extras.digest.marked")}</p>
    {:else if digest.quiet}
      <p class="dg-dim">{$t("social_analytics.extras.digest.quiet")}</p>
    {/if}
    <div class="dg-cards">
      <div class="dg-card" class:dg-off={!digest.followers.available}>
        <span class="dg-label">{$t("social_analytics.extras.digest.followers")}</span>
        {#if digest.followers.available}
          <strong>{signed(digest.followers.delta ?? 0, 0)}</strong>
          <span class="dg-dim">{$t("social_analytics.extras.digest.gained")} {digest.followers.gained?.total ?? 0} · {$t("social_analytics.extras.digest.lost")} {digest.followers.lost?.total ?? 0}</span>
          <span class="dg-people">
            {#each digest.followers.gained?.items ?? [] as u (u.handle)}<button class="dg-link dg-up" onclick={() => openProfile(u.handle)}>+@{u.handle}</button>{/each}
            {#each digest.followers.lost?.items ?? [] as u (u.handle)}<button class="dg-link dg-down" onclick={() => openProfile(u.handle)}>−@{u.handle}</button>{/each}
          </span>
        {:else}
          <strong>{digest.followers.current != null ? compact(digest.followers.current, $locale ?? undefined) : "–"}</strong>
          <span class="dg-dim">{digest.followers.reason ?? $t("social_analytics.extras.digest.unavailable")}</span>
        {/if}
      </div>
      <div class="dg-card" class:dg-off={!digest.monitors.available}>
        <span class="dg-label">{$t("social_analytics.extras.digest.unread")}</span>
        <strong>{digest.monitors.available ? (digest.monitors.unread ?? 0) : "–"}</strong>
        {#if digest.monitors.available}
          <span class="dg-dim">{(digest.monitors.monitors ?? []).map((m) => `${m.name} ${m.unread}`).join(" · ")}</span>
        {:else}
          <span class="dg-dim">{digest.monitors.reason ?? $t("social_analytics.extras.digest.unavailable")}</span>
        {/if}
      </div>
      <div class="dg-card" class:dg-off={!digest.top_post.available}>
        <span class="dg-label">{$t("social_analytics.extras.digest.top_post")}</span>
        {#if digest.top_post.available && digest.top_post.post}
          {@const post = digest.top_post.post}
          <strong>{compact(post.engagement, $locale ?? undefined)}</strong>
          <button class="dg-link dg-text" onclick={() => external(post.url)}>{post.text || post.url}</button>
        {:else}
          <strong>–</strong>
          <span class="dg-dim">{digest.top_post.available ? $t("social_analytics.extras.digest.no_posts") : (digest.top_post.reason ?? $t("social_analytics.extras.digest.unavailable"))}</span>
        {/if}
      </div>
      <div class="dg-card" class:dg-off={!digest.scheduled.available}>
        <span class="dg-label">{$t("social_analytics.extras.digest.due")}</span>
        <strong>{digest.scheduled.available ? (digest.scheduled.due_next_24h ?? 0) : "–"}</strong>
        {#if digest.scheduled.available}
          <span class="dg-dim dg-text">{(digest.scheduled.items ?? []).map((i) => i.text ?? "").filter(Boolean).join(" · ")}</span>
        {:else}
          <span class="dg-dim">{digest.scheduled.reason ?? $t("social_analytics.extras.digest.unavailable")}</span>
        {/if}
      </div>
    </div>

    <h4 class="dg-sub">{$t("social_analytics.extras.rec.title")}</h4>
    {#if digest.recommendations.items.length}
      <ul class="dg-recs">
        {#each digest.recommendations.items as r (r.id)}
          <li>
            <span>{recText(r)}</span>
            <span class="dg-dim" title={r.act_with.note ?? ""}>{$t("social_analytics.extras.rec.tool", { tool: r.act_with.tool })}</span>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="dg-dim">{$t("social_analytics.extras.rec.empty")}</p>
    {/if}
    {#if digest.recommendations.not_triggered.length}
      <button class="dg-link dg-dim" aria-expanded={why} aria-controls="digest-why" onclick={() => (why = !why)}>{$t("social_analytics.extras.rec.why_not")}</button>
      {#if why}
        <ul class="dg-why" id="digest-why">
          {#each digest.recommendations.not_triggered as n (n.id)}
            <li><code>{n.id}</code> {n.reason}</li>
          {/each}
        </ul>
      {/if}
    {/if}
  {/if}
</section>

<style>
  .dg {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    min-width: 0;
  }

  .dg-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .dg-title,
  .dg-sub {
    margin: 0;
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .dg-sub {
    margin-top: var(--space-1);
    color: var(--text-dim);
    font-size: var(--text-xs);
  }

  .dg-grow {
    flex: 1;
  }

  .dg-dim {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .dg-state {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-3) 0;
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  .dg-cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
    gap: var(--space-2);
  }

  .dg-card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-2);
  }

  .dg-card strong {
    font-size: var(--text-xl);
    font-variant-numeric: tabular-nums;
  }

  .dg-off {
    opacity: 0.7;
  }

  .dg-label {
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .dg-people {
    display: flex;
    flex-wrap: wrap;
    gap: 2px var(--space-2);
  }

  .dg-link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--text-xs);
    text-align: left;
    cursor: pointer;
  }

  .dg-link:hover,
  .dg-link:focus-visible {
    text-decoration: underline;
  }

  .dg-link.dg-dim {
    align-self: flex-start;
    color: var(--text-muted);
  }

  .dg-up {
    color: var(--success);
  }

  .dg-down {
    color: var(--danger);
  }

  .dg-text {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }

  .dg-recs,
  .dg-why {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-sm);
  }

  .dg-recs li {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: 2px var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-2);
  }

  .dg-why {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }
</style>
