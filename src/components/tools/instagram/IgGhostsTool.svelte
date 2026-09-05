<script lang="ts">
  /** Seguidores fantasmas: seguem, mas não curtiram nem comentaram os últimos posts. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, type ToolProgress } from "$lib/tools/rt";
  import { cancelJob, DEFAULT_PACING, igState, jobId, n, slugArg, type ActionReport, type GhostReport, type MiniUser } from "$lib/tools/ig.svelte";
  import IgAccountRow from "./IgAccountRow.svelte";
  import IgUserList from "./IgUserList.svelte";

  let posts = $state(12);
  let pages = $state(2);
  let busy = $state(false);
  let job = $state("");
  let progress = $state<ToolProgress | null>(null);
  let report = $state<GhostReport | null>(null);
  let pending = $state<MiniUser[] | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (job && p.id === `ig:${job}`) progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (busy) return;
    busy = true;
    report = null;
    job = jobId("ghosts");
    try {
      report = await invoke<GhostReport>("tool_ig_ghosts", { slug: slugArg(), postsLimit: posts, commentPages: pages, job });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function remove() {
    if (!pending || busy) return;
    const users = pending;
    pending = null;
    busy = true;
    job = jobId("remove");
    try {
      const r = await invoke<ActionReport>("tool_ig_actions", { slug: slugArg(), action: "remove_follower", users, pacing: DEFAULT_PACING, job });
      const done = new Set(r.done.map((u) => u.pk));
      if (report) report = { ...report, ghosts: report.ghosts.filter((u) => !done.has(u.pk)) };
      showToast("success", `${r.done.length} ${$t("tools.common.done")} · ${$t(`tools.ig.follow.stop_${r.stopped}`)}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  let counts = $derived(Object.fromEntries((report?.top_fans ?? []).map(([u, c]) => [u.pk, c])));
</script>

<div class="tool">
  <IgAccountRow />
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ig.ghosts.posts")}</div><div class="group-row-sub">{$t("tools.ig.ghosts.hint")}</div></div>
        <div class="group-row-trailing btn-row">
          <select class="select" bind:value={posts}><option value={6}>6</option><option value={12}>12</option><option value={24}>24</option><option value={48}>48</option></select>
          <select class="select" bind:value={pages}><option value={0}>{$t("tools.ig.ghosts.likes_only")}</option><option value={2}>{$t("tools.ig.ghosts.comments_some")}</option><option value={10}>{$t("tools.ig.ghosts.comments_more")}</option></select>
          {#if busy}<button class="btn btn-secondary btn-sm" type="button" onclick={() => cancelJob(job)}>{$t("tools.ig.common.cancel")}</button>
          {:else}<button class="btn btn-primary" type="button" disabled={!igState.me} onclick={run}>{$t("tools.ig.ghosts.run")}</button>{/if}
        </div>
      </div>
      {#if busy && progress}<div class="group-row"><div class="group-row-sub mono">{progress.stage} {progress.done}{#if progress.total}/{progress.total}{/if} {progress.message ?? ""}</div></div>{/if}
      {#if report}
        <div class="group-row"><div class="group-row-content"><div class="stats"><div><b>{n(report.followers_total)}</b> {$t("tools.ig.common.followers")}</div><div><b>{n(report.engaged)}</b> {$t("tools.ig.ghosts.engaged")}</div><div><b>{n(report.ghosts.length)}</b> {$t("tools.ig.ghosts.ghosts")}</div><div><b>{report.posts_checked}</b> {$t("tools.ig.common.posts")}</div></div></div></div>
      {/if}
    </div>
  </section>
  {#if report}
    <IgUserList users={report.ghosts} title={$t("tools.ig.ghosts.ghosts")} csvName="ghost-followers" actions={[{ id: "remove", label: $t("tools.ig.follow.remove_follower"), danger: true }]} onaction={(_, u) => (pending = u)} />
    {#if pending}
      <section><div class="group confirm"><div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ig.follow.confirm_remove_follower", { count: pending.length })}</div><div class="group-row-sub">{$t("tools.ig.follow.safety")}</div></div>
        <div class="group-row-trailing btn-row"><button class="btn btn-secondary btn-sm" type="button" onclick={() => (pending = null)}>{$t("tools.ig.common.cancel")}</button><button class="btn btn-primary btn-sm" type="button" onclick={remove}>{$t("tools.ig.follow.go")}</button></div>
      </div></div></section>
    {/if}
    <IgUserList users={report.top_fans.map(([u]) => u)} title={$t("tools.ig.ghosts.top_fans")} csvName="top-fans" {counts} />
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .stats { display: flex; flex-wrap: wrap; gap: var(--space-4); font-size: var(--text-sm); color: var(--text-muted); }
  .stats b { color: var(--text); }
  .confirm { box-shadow: inset 0 0 0 2px var(--accent); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); }
</style>
