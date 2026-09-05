<script lang="ts">
  /**
   * Quem não me segue de volta, fãs (me seguem, eu não sigo), mútuos e
   * lista branca. Ações em massa (deixar de seguir, remover seguidor)
   * com ritmo humano, teto diário e cancelamento.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, type ToolProgress } from "$lib/tools/rt";
  import { cancelJob, DEFAULT_PACING, fmtDate, igState, jobId, n, recall, remember, slugArg, type ActionReport, type FollowAnalysis, type MiniUser, type Pacing, type UserInfo } from "$lib/tools/ig.svelte";
  import IgAccountRow from "./IgAccountRow.svelte";
  import IgUserList from "./IgUserList.svelte";

  let { mode = "unfollowers" }: { mode?: "unfollowers" | "fans" | "mutuals" | "whitelist" } = $props();
  let target = $state("");
  let analysis = $state<FollowAnalysis | null>(null);
  let whitelist = $state<MiniUser[]>([]);
  let busy = $state(false);
  let job = $state("");
  let progress = $state<ToolProgress | null>(null);
  let report = $state<ActionReport | null>(null);
  let today = $state(0);
  let pacing = $state<Pacing>({ ...DEFAULT_PACING, ...(JSON.parse(recall("pacing", "{}") || "{}") as Partial<Pacing>) });
  let showPacing = $state(false);
  let confirmAction = $state<{ id: string; users: MiniUser[] } | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (job && p.id === `ig:${job}`) progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function ready(me: UserInfo | null) {
    if (!me) return;
    try {
      whitelist = (await invoke<{ users: MiniUser[] }>("tool_ig_whitelist_get", { slug: slugArg() })).users;
      today = await invoke<number>("tool_ig_actions_today", { slug: slugArg() });
    } catch {
      /* sem sessão */
    }
  }

  let wlSet = $derived(new Set(whitelist.map((u) => u.pk)));

  async function scan() {
    if (busy) return;
    busy = true;
    analysis = null;
    report = null;
    progress = null;
    job = jobId("scan");
    try {
      analysis = await invoke<FollowAnalysis>("tool_ig_follow_lists", { slug: slugArg(), user: target.trim() || null, limit: 0, job });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function toggleWhitelist(u: MiniUser) {
    const next = wlSet.has(u.pk) ? whitelist.filter((x) => x.pk !== u.pk) : [...whitelist, u];
    try {
      whitelist = (await invoke<{ users: MiniUser[] }>("tool_ig_whitelist_set", { slug: slugArg(), users: next })).users;
      if (analysis) analysis = { ...analysis, not_following_back: analysis.not_following_back.filter((x) => !wlSet.has(x.pk) || x.pk === u.pk) };
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function addToWhitelist(users: MiniUser[]) {
    const known = new Set(whitelist.map((u) => u.pk));
    const next = [...whitelist, ...users.filter((u) => !known.has(u.pk))];
    whitelist = (await invoke<{ users: MiniUser[] }>("tool_ig_whitelist_set", { slug: slugArg(), users: next })).users;
    showToast("success", $t("tools.common.done") as string);
  }

  async function runAction(id: string, users: MiniUser[]) {
    if (id === "whitelist") return addToWhitelist(users);
    if (id === "unwhitelist") {
      const drop = new Set(users.map((u) => u.pk));
      whitelist = (await invoke<{ users: MiniUser[] }>("tool_ig_whitelist_set", { slug: slugArg(), users: whitelist.filter((u) => !drop.has(u.pk)) })).users;
      return;
    }
    confirmAction = { id, users };
  }

  async function confirmed() {
    if (!confirmAction || busy) return;
    const { id, users } = confirmAction;
    confirmAction = null;
    remember("pacing", JSON.stringify(pacing));
    busy = true;
    report = null;
    progress = null;
    job = jobId(id);
    try {
      report = await invoke<ActionReport>("tool_ig_actions", { slug: slugArg(), action: id, users, pacing: $state.snapshot(pacing), job });
      today = report.actions_today;
      if (analysis) {
        const done = new Set(report.done.map((u) => u.pk));
        analysis = {
          ...analysis,
          not_following_back: analysis.not_following_back.filter((u) => !done.has(u.pk)),
          fans: analysis.fans.filter((u) => !done.has(u.pk)),
          following: analysis.following.filter((u) => !done.has(u.pk)),
          followers: analysis.followers.filter((u) => !done.has(u.pk)),
        };
      }
      showToast(report.stopped === "finished" ? "success" : "info", `${report.done.length} ${$t("tools.common.done")} · ${$t(`tools.ig.follow.stop_${report.stopped}`)}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  const eta = (count: number) => Math.round((count * (pacing.delay_min_ms + pacing.delay_max_ms)) / 2000 + Math.floor(count / Math.max(1, pacing.pause_every)) * (pacing.pause_ms / 1000));
</script>

<div class="tool">
  <IgAccountRow onready={ready} />
  {#if mode !== "whitelist"}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><input class="input" type="text" bind:value={target} placeholder={$t("tools.ig.follow.target_placeholder")} onkeydown={(e) => e.key === "Enter" && scan()} /></div>
          <div class="group-row-trailing btn-row">
            {#if busy}<button class="btn btn-secondary btn-sm" type="button" onclick={() => cancelJob(job)}>{$t("tools.ig.common.cancel")}</button>
            {:else}<button class="btn btn-primary" type="button" disabled={!igState.me} onclick={scan}>{$t("tools.ig.follow.scan")}</button>{/if}
          </div>
        </div>
        <div class="group-row"><div class="group-row-sub">{$t("tools.ig.follow.scan_hint")}{#if busy && progress} · {progress.stage} {progress.done}{/if}</div></div>
        {#if analysis}
          <div class="group-row">
            <div class="group-row-content"><div class="stats">
              <div><b>{n(analysis.followers_count)}</b> {$t("tools.ig.common.followers")}</div>
              <div><b>{n(analysis.following_count)}</b> {$t("tools.ig.common.following")}</div>
              <div><b>{n(analysis.not_following_back.length)}</b> {$t("tools.ig.follow.not_back")}</div>
              <div><b>{n(analysis.fans.length)}</b> {$t("tools.ig.follow.fans")}</div>
              <div><b>{n(analysis.mutuals.length)}</b> {$t("tools.ig.follow.mutuals")}</div>
              {#if analysis.whitelisted}<div><b>{analysis.whitelisted}</b> {$t("tools.ig.follow.whitelisted")}</div>{/if}
            </div></div>
          </div>
        {/if}
      </div>
    </section>
  {/if}

  {#if mode === "whitelist" || (analysis && !target.trim())}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.ig.follow.pacing")}</div><div class="group-row-sub">{$t("tools.ig.follow.pacing_hint", { today, cap: pacing.daily_cap })}</div></div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => (showPacing = !showPacing)}>{showPacing ? "▴" : "▾"}</button></div>
        </div>
        {#if showPacing}
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.ig.follow.delay")}</div></div><div class="group-row-trailing btn-row"><input class="input input-number" type="number" min="2" bind:value={() => pacing.delay_min_ms / 1000, (v) => (pacing.delay_min_ms = Math.max(2, Number(v)) * 1000)} /> – <input class="input input-number" type="number" min="3" bind:value={() => pacing.delay_max_ms / 1000, (v) => (pacing.delay_max_ms = Math.max(3, Number(v)) * 1000)} /> s</div></div>
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.ig.follow.pause")}</div></div><div class="group-row-trailing btn-row"><input class="input input-number" type="number" min="1" bind:value={pacing.pause_every} /> × <input class="input input-number" type="number" min="0" bind:value={() => pacing.pause_ms / 60000, (v) => (pacing.pause_ms = Math.max(0, Number(v)) * 60000)} /> min</div></div>
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.ig.follow.daily_cap")}</div></div><div class="group-row-trailing"><input class="input input-number" type="number" min="1" max="500" bind:value={pacing.daily_cap} /></div></div>
        {/if}
      </div>
    </section>
  {/if}

  {#if mode === "whitelist"}
    <IgUserList users={whitelist} title={$t("tools.ig.follow.whitelist")} csvName="whitelist" actions={[{ id: "unwhitelist", label: $t("tools.common.remove") }]} onaction={runAction} />
    <p class="group-footer">{$t("tools.ig.follow.whitelist_hint")}</p>
  {:else if analysis}
    {#if mode === "unfollowers"}
      <IgUserList users={analysis.not_following_back} title={$t("tools.ig.follow.not_back")} csvName="not-following-back" whitelist={wlSet} ontogglewhitelist={toggleWhitelist}
        actions={target.trim() ? [] : [{ id: "whitelist", label: $t("tools.ig.follow.add_whitelist") }, { id: "unfollow", label: $t("tools.ig.follow.unfollow"), danger: true }]} onaction={runAction} />
    {:else if mode === "fans"}
      <IgUserList users={analysis.fans} title={$t("tools.ig.follow.fans")} csvName="fans" actions={target.trim() ? [] : [{ id: "remove_follower", label: $t("tools.ig.follow.remove_follower"), danger: true }]} onaction={runAction} />
    {:else}
      <IgUserList users={analysis.mutuals} title={$t("tools.ig.follow.mutuals")} csvName="mutuals" whitelist={wlSet} ontogglewhitelist={target.trim() ? undefined : toggleWhitelist} />
    {/if}
  {/if}

  {#if busy && report === null && progress && (progress.stage === "unfollow" || progress.stage === "remove_follower" || progress.stage === "pause" || progress.stage === "waiting")}
    <section><div class="group"><div class="group-row">
      <div class="group-row-content"><div class="group-row-title">{$t(`tools.ig.follow.stage_${progress.stage}`)} {progress.done}/{progress.total} {progress.message ?? ""}</div></div>
      <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => cancelJob(job)}>{$t("tools.ig.common.cancel")}</button></div>
    </div></div></section>
  {/if}

  {#if report}
    <section>
      <span class="group-label">{$t("tools.ig.follow.report")}</span>
      <div class="group">
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{report.done.length} {$t("tools.common.done")} · {report.failed.length} {$t("tools.common.failed")} · {report.remaining.length} {$t("tools.ig.follow.remaining")}</div><div class="group-row-sub">{$t(`tools.ig.follow.stop_${report.stopped}`)} · {$t("tools.ig.follow.today", { today: report.actions_today })}</div></div></div>
        {#each report.failed.slice(0, 20) as [u, e] (u.pk)}<div class="group-row"><div class="group-row-sub">@{u.username}: {e}</div></div>{/each}
      </div>
    </section>
  {/if}

  {#if confirmAction}
    <section>
      <div class="group confirm">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t(`tools.ig.follow.confirm_${confirmAction.id}`, { count: confirmAction.users.length })}</div>
            <div class="group-row-sub">{$t("tools.ig.follow.confirm_hint", { minutes: Math.ceil(eta(confirmAction.users.length) / 60), today, cap: pacing.daily_cap })}</div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => (confirmAction = null)}>{$t("tools.ig.common.cancel")}</button>
            <button class="btn btn-primary btn-sm" type="button" onclick={confirmed}>{$t("tools.ig.follow.go")}</button>
          </div>
        </div>
      </div>
    </section>
  {/if}
  {#if igState.me && mode !== "whitelist"}<p class="group-footer">{$t("tools.ig.follow.safety")} · {$t("tools.ig.follow.last_scan")}: {analysis ? fmtDate(Date.now() / 1000) : "—"}</p>{/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .stats { display: flex; flex-wrap: wrap; gap: var(--space-4); font-size: var(--text-sm); color: var(--text-muted); }
  .stats b { color: var(--text); }
  .confirm { box-shadow: inset 0 0 0 2px var(--accent); }
  .input-number { width: 64px; }
</style>
