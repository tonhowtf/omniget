<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import SocialIcon from "./SocialIcon.svelte";
  import { social } from "./api";
  import { toolError, type PostRow, type ToolError } from "./insights";
  import { ENGAGEMENT_KINDS, drawArgs, type DrawResult, type Engaged, type EngagementKind, type EngagementPage } from "./second";

  let { posts, openProfile, label }: { posts: PostRow[]; openProfile: (handle: string) => void; label: (p: PostRow) => string } = $props();

  let post = $state("");
  let kind = $state<EngagementKind>("reposts");
  let people = $state<Engaged[]>([]);
  let cursor = $state<string | null>(null);
  let hidden = $state(false);
  let loading = $state(false);
  let error = $state<ToolError | null>(null);
  let seq = 0;

  let winners = $state(1);
  let runnerUps = $state(0);
  let mustFollow = $state(false);
  let mustRepost = $state(false);
  let exclude = $state("");
  let seed = $state("");
  let drawing = $state(false);
  let result = $state<DrawResult | null>(null);
  let drawError = $state<ToolError | null>(null);
  let exporting = $state(false);

  async function load(more = false) {
    if (!post) {
      people = [];
      cursor = null;
      return;
    }
    const mine = ++seq;
    loading = true;
    error = null;
    if (!more) {
      people = [];
      cursor = null;
      hidden = false;
    }
    try {
      const args: Record<string, unknown> = { post, kind };
      if (more && cursor) args.cursor = cursor;
      const r = await social.call<EngagementPage>("x_engagement", args);
      if (mine !== seq) return;
      hidden = !r.available;
      const seen = new Set(people.map((p) => p.handle.toLowerCase()));
      people = [...people, ...r.items.filter((p) => !seen.has(p.handle.toLowerCase()))];
      cursor = r.next_cursor;
    } catch (e) {
      if (mine === seq) error = toolError(e);
    } finally {
      if (mine === seq) loading = false;
    }
  }

  function choose() {
    result = null;
    drawError = null;
    void load();
  }

  function form() {
    return { post, source: kind, winners, runnerUps, mustFollow, mustRepost: mustRepost && kind !== "reposts", exclude, seed };
  }

  async function draw() {
    if (!post || drawing) return;
    drawing = true;
    drawError = null;
    try {
      result = await social.call<DrawResult>("social_draw", drawArgs(form()));
    } catch (e) {
      result = null;
      drawError = toolError(e);
    } finally {
      drawing = false;
    }
  }

  async function exportCsv() {
    if (!result || exporting) return;
    exporting = true;
    drawError = null;
    try {
      result = await social.call<DrawResult>("social_draw", drawArgs({ ...form(), seed: result.seed }, { export: true }));
    } catch (e) {
      drawError = toolError(e);
    } finally {
      exporting = false;
    }
  }

  function reveal(path: string) {
    void invoke("reveal_file", { path }).catch(() => {});
  }
</script>

<div class="ed">
  <p class="ed-dim">{$t("social_analytics.extras.engagement.hint")}</p>
  {#if !posts.length}
    <div class="ed-state">{$t("social_analytics.extras.engagement.need_posts")}</div>
  {:else}
    <label class="field">
      <span class="field-label">{$t("social_analytics.extras.engagement.pick")}</span>
      <select class="input" bind:value={post} onchange={choose}>
        <option value="">–</option>
        {#each posts as p (p.id)}<option value={p.id}>{label(p)}</option>{/each}
      </select>
    </label>
    {#if post}
      <div class="segmented ed-kinds" role="tablist" aria-label={$t("social_analytics.extras.panels.engagement")}>
        {#each ENGAGEMENT_KINDS as k (k)}
          <button class="segmented-btn" role="tab" aria-selected={kind === k} onclick={() => { kind = k; choose(); }}>{$t(`social_analytics.extras.engagement.kind.${k}`)}</button>
        {/each}
      </div>

      {#if error}
        <div class="notice notice-warning" role="alert">
          <div class="notice-body">
            <div class="notice-text">{error.message}</div>
            {#if error.next_step}<div class="notice-text ed-dim">{error.next_step}</div>{/if}
          </div>
          <button class="btn btn-ghost btn-sm" onclick={() => void load()}>{$t("social.common.retry")}</button>
        </div>
      {:else if loading && !people.length}
        <div class="ed-state"><span class="spinner"></span></div>
      {:else if hidden}
        <div class="ed-state">{$t("social_analytics.extras.engagement.likes_hidden")}</div>
      {:else if !people.length}
        <div class="ed-state">{$t("social_analytics.extras.engagement.empty")}</div>
      {:else}
        <ul class="ed-list">
          {#each people as u (u.handle)}
            <li>
              <button class="ed-link" onclick={() => openProfile(u.handle)}>@{u.handle}</button>
              <span class="ed-name">{u.name}</span>
              {#if u.follows_me}<span class="tag tag-success">{$t("social_analytics.extras.engagement.follows_you")}</span>{/if}
              {#if u.post?.text}<span class="ed-dim ed-quote">{u.post.text}</span>{/if}
            </li>
          {/each}
        </ul>
        {#if cursor}
          <button class="btn btn-ghost btn-sm ed-more" disabled={loading} onclick={() => void load(true)}>{#if loading}<span class="spinner"></span>{/if} {$t("social.common.more")}</button>
        {/if}
      {/if}

      <form class="ed-draw" onsubmit={(e) => { e.preventDefault(); void draw(); }}>
        <h4 class="ed-h">{$t("social_analytics.extras.engagement.draw.title")}</h4>
        <div class="ed-row">
          <label class="field"><span class="field-label">{$t("social_analytics.extras.engagement.draw.winners")}</span><input class="input ed-n" type="number" min="1" max="100" bind:value={winners} /></label>
          <label class="field"><span class="field-label">{$t("social_analytics.extras.engagement.draw.runner_ups")}</span><input class="input ed-n" type="number" min="0" max="100" bind:value={runnerUps} /></label>
          <label class="field ed-grow"><span class="field-label">{$t("social_analytics.extras.engagement.draw.seed")}</span><input class="input" bind:value={seed} maxlength="200" autocomplete="off" spellcheck="false" /></label>
        </div>
        <label class="field"><span class="field-label">{$t("social_analytics.extras.engagement.draw.exclude")}</span><input class="input" bind:value={exclude} autocomplete="off" spellcheck="false" /></label>
        <div class="ed-row">
          <label class="ed-check"><input type="checkbox" class="checkbox" bind:checked={mustFollow} /> {$t("social_analytics.extras.engagement.draw.must_follow")}</label>
          {#if kind !== "reposts"}
            <label class="ed-check"><input type="checkbox" class="checkbox" bind:checked={mustRepost} /> {$t("social_analytics.extras.engagement.draw.must_repost")}</label>
          {/if}
          <span class="ed-grow"></span>
          <button class="btn btn-primary btn-sm" disabled={drawing || hidden}>{#if drawing}<span class="spinner"></span>{/if} {$t(drawing ? "social_analytics.extras.engagement.draw.running" : "social_analytics.extras.engagement.draw.run")}</button>
        </div>
      </form>

      {#if drawError}
        <div class="notice notice-warning" role="alert">
          <div class="notice-body">
            <div class="notice-text">{drawError.message}</div>
            {#if drawError.next_step}<div class="notice-text ed-dim">{drawError.next_step}</div>{/if}
          </div>
        </div>
      {/if}
      {#if result}
        <div class="ed-result" role="status">
          <p class="ed-seed">{$t("social_analytics.extras.engagement.draw.seed_used", { seed: result.seed })}</p>
          <p class="ed-dim">{$t("social_analytics.extras.engagement.draw.pool", { pool: result.pool, checked: result.checked })}</p>
          {#if !result.filled}<p class="ed-dim">{$t("social_analytics.extras.engagement.draw.not_filled")}</p>{/if}
          <h5 class="ed-h">{$t("social_analytics.extras.engagement.draw.winners")}</h5>
          <ol class="ed-places">
            {#each result.winners as w (w.handle)}<li><button class="ed-link" onclick={() => openProfile(w.handle)}>@{w.handle}</button></li>{/each}
          </ol>
          {#if result.runner_ups.length}
            <h5 class="ed-h">{$t("social_analytics.extras.engagement.draw.runner_ups")}</h5>
            <ol class="ed-places" start={result.winners.length + 1}>
              {#each result.runner_ups as w (w.handle)}<li><button class="ed-link" onclick={() => openProfile(w.handle)}>@{w.handle}</button></li>{/each}
            </ol>
          {/if}
          {#if result.disqualified.length}
            <details>
              <summary class="ed-dim">{$t("social_analytics.extras.engagement.draw.disqualified")} · {result.disqualified.length}</summary>
              <ul class="ed-notes">{#each result.disqualified as d (d.handle)}<li>@{d.handle}: {d.reason}</li>{/each}</ul>
            </details>
          {/if}
          <details>
            <summary class="ed-dim">{$t("social_analytics.extras.engagement.draw.method")}</summary>
            <ul class="ed-notes">
              <li>{result.method}</li>
              {#each result.notes ?? [] as n (n)}<li>{n}</li>{/each}
            </ul>
          </details>
          <div class="ed-row">
            <button class="btn btn-secondary btn-sm" disabled={exporting || !result.winners.length} onclick={() => void exportCsv()}>
              {#if exporting}<span class="spinner"></span>{:else}<SocialIcon name="download" />{/if} {$t("social_analytics.extras.engagement.draw.export")}
            </button>
            {#if result.csv_path}
              {@const path = result.csv_path}
              <span class="ed-dim ed-path">{$t("social_analytics.extras.engagement.draw.exported", { path })}</span>
              <button class="btn btn-ghost btn-sm" onclick={() => reveal(path)}>{$t("social_analytics.reveal")}</button>
            {/if}
          </div>
        </div>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .ed {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .ed-dim {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .ed-state {
    display: flex;
    justify-content: center;
    padding: var(--space-4) 0;
    color: var(--text-muted);
    font-size: var(--text-sm);
    text-align: center;
  }

  .ed-kinds,
  .ed-more {
    align-self: flex-start;
  }

  .ed-list {
    display: flex;
    flex-direction: column;
    max-height: 320px;
    margin: 0;
    padding: var(--space-1) 0;
    overflow-y: auto;
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-size: var(--text-sm);
  }

  .ed-list li {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 2px var(--space-2);
    padding: 4px var(--space-3);
    min-width: 0;
  }

  .ed-name {
    color: var(--text-dim);
  }

  .ed-quote {
    flex-basis: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ed-link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .ed-link:hover,
  .ed-link:focus-visible {
    text-decoration: underline;
  }

  .ed-draw,
  .ed-result {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ed-h {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .ed-row {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-2);
  }

  .ed-grow {
    flex: 1;
    min-width: 140px;
  }

  .ed-n {
    width: 90px;
  }

  .ed-check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .ed-seed {
    margin: 0;
    font-family: var(--font-mono, monospace);
    font-size: var(--text-xs);
    overflow-wrap: anywhere;
    user-select: text;
  }

  .ed-places,
  .ed-notes {
    margin: 0;
    padding-left: var(--space-5);
    font-size: var(--text-sm);
  }

  .ed-notes {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .ed-path {
    overflow-wrap: anywhere;
  }
</style>
