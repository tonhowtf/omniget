<script lang="ts">
  /** Lê o export oficial de dados do Reddit (pilha de CSV). Tudo local, sem rede. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openUrl, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Totals = {
    posts: number; comments: number; saved_posts: number; saved_comments: number;
    votes_up: number; votes_down: number; subscribed: number; messages: number;
    chat_messages: number; hidden: number; friends: number; moderated: number;
    drafts: number; poll_votes: number;
  };
  type SubCount = { subreddit: string; posts: number; comments: number; total: number };
  type MonthCount = { month: string; posts: number; comments: number; total: number };
  type Item = { kind: string; subreddit: string; date: string; score: number | null; title: string; body: string; permalink: string };
  type Stat = { name: string; value: string };
  type Result = {
    source: string; files: { name: string; rows: number }[]; totals: Totals;
    top_subreddits: SubCount[]; timeline: MonthCount[]; top_posts: Item[]; top_comments: Item[];
    saved: Item[]; subscribed: string[]; statistics: Stat[]; ranked_by: string;
    first_month: string; last_month: string; warnings: string[]; exported: string[];
  };

  let path = $state("");
  let exportDir = $state("");
  let fmtJson = $state(true);
  let fmtCsv = $state(false);
  let fmtMd = $state(true);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const TOTALS: [keyof Totals, string][] = [
    ["posts", "tools.rdgdpr.t_posts"],
    ["comments", "tools.rdgdpr.t_comments"],
    ["saved_posts", "tools.rdgdpr.t_saved_posts"],
    ["saved_comments", "tools.rdgdpr.t_saved_comments"],
    ["votes_up", "tools.rdgdpr.t_votes_up"],
    ["votes_down", "tools.rdgdpr.t_votes_down"],
    ["subscribed", "tools.rdgdpr.t_subscribed"],
    ["messages", "tools.rdgdpr.t_messages"],
    ["chat_messages", "tools.rdgdpr.t_chat"],
    ["hidden", "tools.rdgdpr.t_hidden"],
    ["friends", "tools.rdgdpr.t_friends"],
    ["moderated", "tools.rdgdpr.t_moderated"],
    ["drafts", "tools.rdgdpr.t_drafts"],
    ["poll_votes", "tools.rdgdpr.t_polls"],
  ];

  const peak = $derived(Math.max(1, ...(result?.timeline ?? []).map((m) => m.total)));

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "rd-gdpr") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run(withExport: boolean) {
    if (!path || busy) return;
    busy = true;
    progress = null;
    const formats = [fmtJson && "json", fmtCsv && "csv", fmtMd && "md"].filter(Boolean) as string[];
    try {
      const r = await invoke<Result>("tool_rd_gdpr", {
        opts: {
          path,
          export_dir: withExport && exportDir ? exportDir : null,
          formats,
          top: 20,
        },
      });
      result = r;
      if (withExport && r.exported.length) showToast("success", `${r.exported.length} ${$t("tools.rdgdpr.written")}`);
      else showToast("success", `${r.totals.posts + r.totals.comments} ${$t("tools.rdgdpr.entries")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.rdgdpr.source")}</div>
          <div class="group-row-sub mono">{path ? baseName(path) : $t("tools.rdgdpr.source_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile([{ name: "Zip", extensions: ["zip"] }]); if (f) path = f; }}>{$t("tools.rdgdpr.pick_zip")}</button>
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) path = d; }}>{$t("tools.rdgdpr.pick_folder")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-sub">{$t("tools.rdgdpr.how")} <button class="link" type="button" onclick={() => openUrl("https://www.reddit.com/settings/data-request")}>reddit.com/settings/data-request</button></div></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !path} onclick={() => run(false)}>{busy ? $t("tools.common.working") : $t("tools.rdgdpr.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.files.length} {$t("tools.rdgdpr.files_read")}</div>
            {#if result.first_month}<div class="group-row-sub">{result.first_month} → {result.last_month}</div>{/if}
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            <div class="totals">
              {#each TOTALS as [key, label] (key)}
                {#if result.totals[key] > 0}
                  <div class="cell"><strong>{result.totals[key]}</strong><span>{$t(label)}</span></div>
                {/if}
              {/each}
            </div>
          </div>
        </div>
      </div>
    </section>

    {#if result.top_subreddits.length}
      <section>
        <div class="group">
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.rdgdpr.top_subs")}</div></div></div>
          {#each result.top_subreddits as s (s.subreddit)}
            <div class="group-row">
              <div class="group-row-content"><div class="group-row-sub">r/{s.subreddit}</div></div>
              <div class="group-row-trailing"><span class="dim">{s.posts} · {s.comments}</span> <strong>{s.total}</strong></div>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {#if result.timeline.length}
      <section>
        <div class="group">
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.rdgdpr.timeline")}</div></div></div>
          <div class="group-row">
            <div class="group-row-content">
              <div class="chart">
                {#each result.timeline as m (m.month)}
                  <div class="bar" style:height="{Math.max(2, (m.total / peak) * 100)}%" title="{m.month}: {m.total}"></div>
                {/each}
              </div>
              <div class="group-row-sub">{result.timeline[0].month} → {result.timeline[result.timeline.length - 1].month}</div>
            </div>
          </div>
        </div>
      </section>
    {/if}

    {#if result.top_posts.length || result.top_comments.length}
      <section>
        <div class="group">
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{$t("tools.rdgdpr.highlights")}</div>
              <div class="group-row-sub">{result.ranked_by === "score" ? $t("tools.rdgdpr.by_score") : $t("tools.rdgdpr.by_date")}</div>
            </div>
          </div>
          {#each [...result.top_posts, ...result.top_comments].slice(0, 20) as item, i (i)}
            <div class="group-row">
              <div class="group-row-content">
                <div class="group-row-sub">{item.title || item.body || "—"}</div>
                <div class="group-row-sub dim">r/{item.subreddit} · {item.date}{item.score !== null ? ` · ${item.score}` : ""}</div>
              </div>
              {#if item.permalink}<div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(item.permalink)}>↗</button></div>{/if}
            </div>
          {/each}
        </div>
      </section>
    {/if}

    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.rdgdpr.export")}</div>
            <div class="group-row-sub mono">{exportDir || "—"}</div>
          </div>
          <div class="group-row-trailing btn-row">
            <div class="segmented">
              <button class="segmented-btn" class:active={fmtJson} type="button" aria-pressed={fmtJson} onclick={() => (fmtJson = !fmtJson)}>JSON</button>
              <button class="segmented-btn" class:active={fmtCsv} type="button" aria-pressed={fmtCsv} onclick={() => (fmtCsv = !fmtCsv)}>CSV</button>
              <button class="segmented-btn" class:active={fmtMd} type="button" aria-pressed={fmtMd} onclick={() => (fmtMd = !fmtMd)}>Markdown</button>
            </div>
            <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) exportDir = d; }}>{$t("tools.common.choose")}</button>
            <button class="btn btn-primary btn-sm" type="button" disabled={busy || !exportDir || !(fmtJson || fmtCsv || fmtMd)} onclick={() => run(true)}>{$t("tools.common.save")}</button>
          </div>
        </div>
        {#each result.exported as f (f)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub mono">{baseName(f)}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(f)}>{$t("tools.common.reveal")}</button></div>
          </div>
        {/each}
      </div>
    </section>

    {#if result.warnings.length}
      <section>
        <div class="group">
          {#each result.warnings as w (w)}
            <div class="group-row"><div class="group-row-content"><div class="group-row-sub dim">{w}</div></div></div>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .link { background: none; border: 0; padding: 0; color: var(--accent); cursor: pointer; font: inherit; }
  .totals { display: flex; flex-wrap: wrap; gap: var(--space-4); }
  .cell { display: flex; flex-direction: column; min-width: 5.5rem; }
  .cell strong { font-size: var(--text-lg); }
  .cell span { color: var(--text-dim); font-size: var(--text-xs); }
  .chart { display: flex; align-items: flex-end; gap: 2px; height: 72px; margin-bottom: var(--space-2); }
  .bar { flex: 1; min-width: 2px; background: var(--accent); border-radius: 2px 2px 0 0; opacity: 0.8; }
</style>
