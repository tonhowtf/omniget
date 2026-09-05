<script lang="ts">
  /** Arquivo do X (estudo 67): abre o zip de "Baixar seus dados" offline. */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { openUrl, pickDir, pickFile, reveal } from "$lib/tools/rt";
  import { fmtDate, fmtN, xErr, type XPost } from "$lib/tools/x";
  import PostCard from "./PostCard.svelte";

  type Acc = { account_id: string; user_link: string };
  type Year = { year: number; tweets: number; likes_received: number };
  type Summary = {
    path: string; username: string; display_name: string; account_id: string; created_at: string;
    tweets: number; replies: number; reposts: number; likes: number; followers: number; following: number; blocked: number; muted: number; dm_messages: number;
    first_tweet: string; last_tweet: string; likes_received: number; reposts_received: number;
    by_year: Year[]; by_weekday: number[]; by_hour: number[]; top_tweets: XPost[]; not_following_back: Acc[]; fans: Acc[]; files: string[];
  };

  let path = $state("");
  let busy = $state<string | null>(null);
  let s = $state<Summary | null>(null);
  let what = $state("tweets");
  let format = $state("csv");
  let dest = $state("");

  async function open(p: string) {
    busy = "open";
    s = null;
    try {
      s = await invoke<Summary>("tool_x_archive_open", { path: p });
      path = p;
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = null;
    }
  }

  async function exportNow() {
    if (!s) return;
    if (!dest) {
      const d = await pickDir();
      if (!d) return;
      dest = d;
    }
    busy = "export";
    try {
      const f = await invoke<string>("tool_x_archive_export", { path, dest, what, format });
      showToast("success", $t("tools.common.done") as string);
      await reveal(f);
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = null;
    }
  }

  const WD = ["dom", "seg", "ter", "qua", "qui", "sex", "sáb"];
  let maxYear = $derived(Math.max(1, ...(s?.by_year.map((y) => y.tweets) ?? [1])));
  let maxWd = $derived(Math.max(1, ...(s?.by_weekday ?? [1])));
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.x.archive_file")}</div><div class="group-row-sub mono">{path || $t("tools.x.archive_hint")}</div></div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={async () => { const f = await pickFile([{ name: "ZIP", extensions: ["zip"] }]); if (f) open(f); }}>{busy === "open" ? $t("tools.common.working") : $t("tools.x.archive_zip")}</button>
          <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={async () => { const d = await pickDir(); if (d) open(d); }}>{$t("tools.x.archive_folder")}</button>
        </div>
      </div>
      <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{$t("tools.x.archive_intro")} <button class="link" type="button" onclick={() => openUrl("https://x.com/settings/download_your_data")}>x.com/settings/download_your_data</button></div></div></div>
    </div>
  </section>

  {#if s}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{s.display_name} <span class="dim">@{s.username}</span></div><div class="group-row-sub">{$t("tools.x.joined")} {fmtDate(s.created_at)} · {s.files.length} {$t("tools.common.files")}{#if s.first_tweet} · {fmtDate(s.first_tweet).slice(0, 12)} → {fmtDate(s.last_tweet).slice(0, 12)}{/if}</div></div>
        </div>
        <div class="group-row"><div class="tiles">
          <div class="tile"><div class="v">{fmtN(s.tweets)}</div><div class="k">{$t("tools.x.posts")}</div></div>
          <div class="tile"><div class="v">{fmtN(s.replies)}</div><div class="k">{$t("tools.x.replies")}</div></div>
          <div class="tile"><div class="v">{fmtN(s.reposts)}</div><div class="k">{$t("tools.x.reposts")}</div></div>
          <div class="tile"><div class="v">{fmtN(s.likes)}</div><div class="k">{$t("tools.x.likes_given")}</div></div>
          <div class="tile"><div class="v">{fmtN(s.likes_received)}</div><div class="k">{$t("tools.x.likes_received")}</div></div>
          <div class="tile"><div class="v">{fmtN(s.followers)}</div><div class="k">{$t("tools.x.followers")}</div></div>
          <div class="tile"><div class="v">{fmtN(s.following)}</div><div class="k">{$t("tools.x.following")}</div></div>
          <div class="tile warn"><div class="v">{fmtN(s.not_following_back.length)}</div><div class="k">{$t("tools.x.not_following_back")}</div></div>
          <div class="tile"><div class="v">{fmtN(s.fans.length)}</div><div class="k">{$t("tools.x.fans")}</div></div>
          <div class="tile"><div class="v">{fmtN(s.dm_messages)}</div><div class="k">DMs</div></div>
          <div class="tile"><div class="v">{fmtN(s.blocked)}</div><div class="k">{$t("tools.x.blocked")}</div></div>
          <div class="tile"><div class="v">{fmtN(s.muted)}</div><div class="k">{$t("tools.x.muted")}</div></div>
        </div></div>
      </div>
    </section>

    <section>
      <div class="group">
        <div class="group-row"><div class="group-row-content">
          <div class="group-row-sub">{$t("tools.x.by_year")}</div>
          <div class="bars">{#each s.by_year as y (y.year)}<div class="bar" title="{y.year}: {y.tweets} · ♥ {y.likes_received}"><div class="bar-fill" style:height="{Math.max(2, (y.tweets / maxYear) * 100)}%"></div><span class="bar-label">{String(y.year).slice(2)}</span></div>{/each}</div>
        </div></div>
        <div class="group-row"><div class="group-row-content">
          <div class="group-row-sub">{$t("tools.x.by_weekday")}</div>
          <div class="bars wd">{#each s.by_weekday as n, i (i)}<div class="bar" title="{WD[i]}: {n}"><div class="bar-fill" style:height="{Math.max(2, (n / maxWd) * 100)}%"></div><span class="bar-label">{WD[i]}</span></div>{/each}</div>
        </div></div>
      </div>
    </section>

    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content btn-row wrap">
            <select class="input" bind:value={what}>
              {#each ["tweets", "likes", "followers", "following", "not_following_back", "fans", "blocked", "muted"] as w (w)}<option value={w}>{$t(`tools.x.exp_${w}`)}</option>{/each}
            </select>
            <select class="input" bind:value={format}><option value="csv">CSV</option><option value="json">JSON</option><option value="md">Markdown</option>{#if what === "tweets"}<option value="html">HTML</option>{/if}</select>
            <span class="dim mono">{dest || $t("tools.common.ask_on_run")}</span>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button>
            <button class="btn btn-primary btn-sm" type="button" disabled={busy !== null} onclick={exportNow}>{busy === "export" ? $t("tools.common.working") : $t("tools.x.export")}</button>
          </div>
        </div>
      </div>
    </section>

    {#if s.top_tweets.length}
      <section>
        <span class="group-label">{$t("tools.x.top_posts")}</span>
        <div class="group">{#each s.top_tweets as p (p.id)}<PostCard post={p} compact />{/each}</div>
      </section>
    {/if}

    {#if s.not_following_back.length}
      <section>
        <span class="group-label">{$t("tools.x.not_following_back")} · {s.not_following_back.length}</span>
        <div class="group list">
          {#each s.not_following_back.slice(0, 200) as a (a.account_id)}
            <div class="group-row"><div class="group-row-content mono">{a.account_id}</div><div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(a.user_link || `https://x.com/i/user/${a.account_id}`)}>↗</button></div></div>
          {/each}
          {#if s.not_following_back.length > 200}<div class="group-row"><div class="group-row-sub">+{s.not_following_back.length - 200} · {$t("tools.x.export_for_all")}</div></div>{/if}
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .dim { color: var(--text-muted); font-weight: 400; }
  .link { background: none; border: 0; color: var(--accent-hi); cursor: pointer; font: inherit; padding: 0; }
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); gap: var(--space-2); width: 100%; }
  .tile { padding: var(--space-2) var(--space-3); border-radius: var(--radius-md); background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .tile.warn { background: color-mix(in srgb, var(--warning) 14%, transparent); }
  .tile .v { font-family: var(--font-display); font-size: var(--text-lg); font-weight: 700; color: var(--text); }
  .tile .k { font-size: var(--text-xs); color: var(--text-muted); }
  .bars { display: flex; align-items: flex-end; gap: 4px; height: 80px; width: 100%; margin-top: var(--space-1); }
  .bars.wd { gap: var(--space-2); height: 60px; }
  .bar { flex: 1; display: flex; flex-direction: column; justify-content: flex-end; height: 100%; }
  .bar-fill { background: var(--accent); border-radius: 3px 3px 0 0; opacity: 0.8; }
  .bar-label { font-size: 10px; color: var(--text-muted); text-align: center; height: 14px; }
  .wrap { flex-wrap: wrap; gap: var(--space-2); }
  .list { max-height: 360px; overflow: auto; }
</style>
