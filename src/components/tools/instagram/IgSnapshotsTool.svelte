<script lang="ts">
  /** Quem deixou de me seguir: snapshots locais e a diferença entre dois. */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/tools/rt";
  import { cancelJob, fmtDate, igState, jobId, slugArg, type SnapshotDiff, type SnapshotMeta, type UserInfo } from "$lib/tools/ig.svelte";
  import IgAccountRow from "./IgAccountRow.svelte";
  import IgUserList from "./IgUserList.svelte";

  let snaps = $state<SnapshotMeta[]>([]);
  let from = $state("");
  let to = $state("");
  let diff = $state<SnapshotDiff | null>(null);
  let busy = $state(false);
  let job = $state("");

  async function load(me: UserInfo | null) {
    if (!me) return;
    snaps = await invoke<SnapshotMeta[]>("tool_ig_snapshots", { slug: slugArg() });
    if (snaps.length >= 2) {
      to = snaps[0].file;
      from = snaps[1].file;
      await compare();
    } else if (snaps.length === 1) {
      to = snaps[0].file;
    }
  }

  async function take() {
    if (busy) return;
    busy = true;
    job = jobId("snap");
    try {
      await invoke<SnapshotMeta>("tool_ig_snapshot_take", { slug: slugArg(), job });
      showToast("success", $t("tools.ig.snap.saved") as string);
      await load(igState.me);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function compare() {
    if (!from || !to || from === to) return;
    diff = await invoke<SnapshotDiff>("tool_ig_snapshot_diff", { from, to });
  }

  async function remove(file: string) {
    await invoke("tool_ig_snapshot_delete", { file });
    await load(igState.me);
  }
</script>

<div class="tool">
  <IgAccountRow onready={load} />
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ig.snap.title")}</div><div class="group-row-sub">{$t("tools.ig.snap.hint")}</div></div>
        <div class="group-row-trailing btn-row">
          {#if busy}<button class="btn btn-secondary btn-sm" type="button" onclick={() => cancelJob(job)}>{$t("tools.ig.common.cancel")}</button>{/if}
          <button class="btn btn-primary" type="button" disabled={busy || !igState.me} onclick={take}>{busy ? $t("tools.common.working") : $t("tools.ig.snap.take")}</button>
        </div>
      </div>
      {#each snaps as s (s.file)}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{fmtDate(s.taken_at)}</div><div class="group-row-sub">{s.followers} {$t("tools.ig.common.followers")} · {s.following} {$t("tools.ig.common.following")}</div></div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-ghost btn-sm" class:active={from === s.file} type="button" onclick={() => { from = s.file; compare(); }}>A</button>
            <button class="btn btn-ghost btn-sm" class:active={to === s.file} type="button" onclick={() => { to = s.file; compare(); }}>B</button>
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => remove(s.file)}>×</button>
          </div>
        </div>
      {/each}
    </div>
  </section>
  {#if diff}
    <section><span class="group-label">{fmtDate(diff.from)} → {fmtDate(diff.to)}</span></section>
    <IgUserList users={diff.lost_followers} title={$t("tools.ig.snap.lost")} csvName="lost-followers" />
    <IgUserList users={diff.new_followers} title={$t("tools.ig.snap.new")} csvName="new-followers" />
    {#if diff.lost_following.length || diff.new_following.length}
      <IgUserList users={diff.lost_following} title={$t("tools.ig.snap.lost_following")} csvName="unfollowed" />
      <IgUserList users={diff.new_following} title={$t("tools.ig.snap.new_following")} csvName="new-following" />
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .active { background: var(--accent-soft); color: var(--accent-hi); }
</style>
