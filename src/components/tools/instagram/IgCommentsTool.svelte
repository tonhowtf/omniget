<script lang="ts">
  /**
   * Comentários e curtidas de um post (exportar) e sorteio entre os
   * comentários. `mode` = "comments" | "likers" | "giveaway".
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openUrl, type ToolProgress } from "$lib/tools/rt";
  import { cancelJob, exportCsv, fmtDate, igState, jobId, n, profileUrl, slugArg, type Comment, type GiveawayResult, type GiveawayRules, type MediaItem, type MiniUser } from "$lib/tools/ig.svelte";
  import IgAccountRow from "./IgAccountRow.svelte";
  import IgUserList from "./IgUserList.svelte";

  let { mode = "comments" }: { mode?: "comments" | "likers" | "giveaway" } = $props();
  let url = $state("");
  let limit = $state(500);
  let busy = $state(false);
  let job = $state("");
  let progress = $state<ToolProgress | null>(null);
  let item = $state<MediaItem | null>(null);
  let comments = $state<Comment[]>([]);
  let likers = $state<{ count: number; users: MiniUser[] } | null>(null);
  let filter = $state("");
  let rules = $state<GiveawayRules>({ winners: 1, unique_users: true, min_mentions: 0, keyword: "", exclude: [], owner_username: "" });
  let excludeText = $state("");
  let result = $state<GiveawayResult | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (job && p.id === `ig:${job}`) progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!url.trim() || busy) return;
    busy = true;
    item = null;
    comments = [];
    likers = null;
    result = null;
    job = jobId(mode);
    try {
      if (mode === "likers") {
        const r = await invoke<{ item: MediaItem; count: number; users: MiniUser[] }>("tool_ig_likers", { slug: slugArg(), url: url.trim() });
        item = r.item;
        likers = { count: r.count, users: r.users };
      } else {
        const r = await invoke<{ item: MediaItem; comments: Comment[] }>("tool_ig_comments", { slug: slugArg(), url: url.trim(), limit: mode === "giveaway" ? 0 : limit, job });
        item = r.item;
        comments = r.comments;
        rules.owner_username = r.item.username;
      }
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  let shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return q ? comments.filter((c) => c.text.toLowerCase().includes(q) || c.user.username.toLowerCase().includes(q)) : comments;
  });

  async function csv() {
    const p = await exportCsv("instagram-comments", ["username", "text", "date", "likes", "replies", "mentions", "profile"], shown.map((c) => [c.user.username, c.text, fmtDate(c.created_at), String(c.like_count), String(c.reply_count), c.mentions.join(" "), profileUrl(c.user.username)]));
    if (p) showToast("success", $t("tools.common.done") as string);
  }

  async function draw() {
    rules.exclude = excludeText.split(/[\s,]+/).filter(Boolean);
    result = await invoke<GiveawayResult>("tool_ig_giveaway", { comments, rules: $state.snapshot(rules) });
  }

  let commenters = $derived.by(() => {
    const seen = new Map<string, MiniUser>();
    for (const c of comments) if (!seen.has(c.user.pk)) seen.set(c.user.pk, c.user);
    return [...seen.values()];
  });
</script>

<div class="tool">
  <IgAccountRow />
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder={$t("tools.ig.download.placeholder")} onkeydown={(e) => e.key === "Enter" && run()} /></div>
        <div class="group-row-trailing btn-row">
          {#if mode === "comments"}<select class="select" bind:value={limit}><option value={100}>100</option><option value={500}>500</option><option value={2000}>2000</option><option value={0}>{$t("tools.ig.profile.all")}</option></select>{/if}
          {#if busy}<button class="btn btn-secondary btn-sm" type="button" onclick={() => cancelJob(job)}>{$t("tools.ig.common.cancel")}</button>
          {:else}<button class="btn btn-primary" type="button" disabled={!url.trim() || !igState.me} onclick={run}>{$t("tools.ig.comments.fetch")}</button>{/if}
        </div>
      </div>
      {#if busy && progress}<div class="group-row"><div class="group-row-sub mono">{progress.stage} {progress.done}{#if progress.total}/{progress.total}{/if}</div></div>{/if}
      {#if item}
        <div class="group-row post">
          <img src={item.thumbnail} alt="" />
          <div class="group-row-content"><div class="group-row-title">@{item.username} · {fmtDate(item.taken_at)}</div><div class="group-row-sub">♥ {n(item.like_count)} · 💬 {n(item.comment_count)} · {item.caption.slice(0, 140)}</div></div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(item!.url)}>{$t("tools.common.open")}</button></div>
        </div>
      {/if}
    </div>
  </section>

  {#if mode === "likers" && likers}
    <IgUserList users={likers.users} title={`${$t("tools.ig.comments.likers")} · ${n(likers.count)}`} csvName="likers" />
  {:else if mode === "giveaway" && comments.length}
    <section>
      <span class="group-label">{$t("tools.ig.giveaway.rules")} · {comments.length} {$t("tools.ig.comments.comments")}</span>
      <div class="group">
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.ig.giveaway.winners")}</div></div><div class="group-row-trailing"><input class="input input-number" type="number" min="1" max="50" bind:value={rules.winners} /></div></div>
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.ig.giveaway.min_mentions")}</div><div class="group-row-sub">{$t("tools.ig.giveaway.min_mentions_hint")}</div></div><div class="group-row-trailing"><input class="input input-number" type="number" min="0" max="10" bind:value={rules.min_mentions} /></div></div>
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.ig.giveaway.keyword")}</div></div><div class="group-row-trailing"><input class="input" type="text" bind:value={rules.keyword} placeholder="#promo" /></div></div>
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.ig.giveaway.exclude")}</div></div><div class="group-row-trailing"><input class="input" type="text" bind:value={excludeText} placeholder="@a @b" /></div></div>
        <div class="group-row"><div class="group-row-content"><label class="chk"><input type="checkbox" bind:checked={rules.unique_users} /> {$t("tools.ig.giveaway.unique")}</label></div><div class="group-row-trailing"><button class="btn btn-primary" type="button" onclick={draw}>{$t("tools.ig.giveaway.draw")}</button></div></div>
      </div>
    </section>
    {#if result}
      <section>
        <span class="group-label">{$t("tools.ig.giveaway.result", { eligible: result.eligible })} · seed {result.seed}</span>
        <div class="group">
          {#each result.winners as w, i (w.pk)}
            <div class="group-row">
              <div class="win">{i + 1}</div>
              <div class="group-row-content"><div class="group-row-title">@{w.user.username}</div><div class="group-row-sub">{w.text}</div></div>
              <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(profileUrl(w.user.username))}>{$t("tools.common.open")}</button></div>
            </div>
          {/each}
          {#if !result.winners.length}<div class="group-row"><div class="group-row-sub">{$t("tools.ig.list.empty")}</div></div>{/if}
        </div>
      </section>
    {/if}
  {:else if comments.length}
    <section>
      <div class="head">
        <span class="group-label">{comments.length} {$t("tools.ig.comments.comments")} · {commenters.length} {$t("tools.ig.comments.people")}</span>
        <div class="btn-row"><input class="input" type="search" bind:value={filter} placeholder={$t("tools.ig.list.filter")} /><button class="btn btn-secondary btn-sm" type="button" onclick={csv}>CSV</button></div>
      </div>
      <div class="group">
        {#each shown.slice(0, 300) as c (c.pk)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">@{c.user.username} <span class="muted">· {fmtDate(c.created_at)}{#if c.like_count} · ♥ {c.like_count}{/if}{#if c.reply_count} · ↳ {c.reply_count}{/if}</span></div><div class="group-row-sub text">{c.text}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(profileUrl(c.user.username))}>{$t("tools.common.open")}</button></div>
          </div>
        {/each}
        {#if shown.length > 300}<div class="group-row"><div class="group-row-sub">+{shown.length - 300}</div></div>{/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .head { display: flex; justify-content: space-between; align-items: center; gap: var(--space-2); margin-bottom: var(--space-2); }
  .head .input { max-width: 200px; }
  .post img { width: 48px; height: 48px; border-radius: var(--radius-sm, 6px); object-fit: cover; margin-right: var(--space-2); }
  .muted { color: var(--text-muted); font-weight: 400; }
  .text { white-space: pre-wrap; }
  .win { width: 28px; height: 28px; border-radius: 50%; display: grid; place-items: center; background: var(--accent); color: #fff; font-weight: 700; margin-right: var(--space-2); flex: none; }
  .chk { display: inline-flex; align-items: center; gap: var(--space-1); font-size: var(--text-sm); }
  .input-number { width: 72px; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); }
</style>
