<script lang="ts">
  /**
   * Lê o export oficial ("Baixe suas informações", JSON) sem tocar na API:
   * quem não segue de volta, fãs, pedidos pendentes, melhores amigos,
   * bloqueados, deixou de seguir recentemente. Com sessão, dá para
   * transformar em ações.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, openUrl, pickDir, pickFile } from "$lib/tools/rt";
  import { DEFAULT_PACING, exportCsv, fmtDay, igState, jobId, n, profileUrl, slugArg, type ActionReport, type ExportReport, type ExportUser, type MiniUser } from "$lib/tools/ig.svelte";
  import IgAccountRow from "./IgAccountRow.svelte";

  let report = $state<ExportReport | null>(null);
  let busy = $state(false);
  let tab = $state("not_following_back");
  let filter = $state("");
  let selected = $state<Set<string>>(new Set());
  let pending = $state<{ action: string; users: ExportUser[] } | null>(null);

  const TABS = ["not_following_back", "fans", "pending_sent", "close_friends", "recently_unfollowed", "received_requests", "blocked", "restricted", "hide_story_from", "followers", "following"] as const;

  async function open(kind: "zip" | "dir") {
    const p = kind === "zip" ? await pickFile([{ name: "Instagram export", extensions: ["zip"] }]) : await pickDir();
    if (!p) return;
    busy = true;
    report = null;
    try {
      report = await invoke<ExportReport>("tool_ig_export", { path: p });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  let list = $derived.by(() => {
    const src = (report?.[tab as keyof ExportReport] as ExportUser[] | undefined) ?? [];
    const q = filter.trim().toLowerCase();
    return q ? src.filter((u) => u.username.includes(q)) : src;
  });

  $effect(() => {
    tab;
    selected = new Set();
  });

  function toggle(u: string) {
    const s = new Set(selected);
    if (s.has(u)) s.delete(u);
    else s.add(u);
    selected = s;
  }

  async function csv() {
    const p = await exportCsv(`instagram-${tab}`, ["username", "since", "url"], list.map((u) => [u.username, fmtDay(u.timestamp), u.href || profileUrl(u.username)]));
    if (p) showToast("success", $t("tools.common.done") as string);
  }

  async function act() {
    if (!pending || busy) return;
    const { action, users } = pending;
    pending = null;
    busy = true;
    try {
      const resolved = await invoke<MiniUser[]>("tool_ig_resolve_users", { slug: slugArg(), usernames: users.map((u) => u.username), job: jobId("resolve") });
      if (!resolved.length) throw new Error($t("tools.ig.export.none_resolved") as string);
      const r = await invoke<ActionReport>("tool_ig_actions", { slug: slugArg(), action, users: resolved, pacing: DEFAULT_PACING, job: jobId(action) });
      showToast("success", `${r.done.length} ${$t("tools.common.done")} · ${$t(`tools.ig.follow.stop_${r.stopped}`)}`);
      selected = new Set();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  let actionFor = $derived(tab === "not_following_back" || tab === "pending_sent" ? "unfollow" : tab === "fans" ? "remove_follower" : "");
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ig.export.title")}</div><div class="group-row-sub">{$t("tools.ig.export.hint")}</div></div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl("https://accountscenter.instagram.com/info_and_permissions/dyi/")}>{$t("tools.ig.export.request")}</button>
          <button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={() => open("dir")}>{$t("tools.ig.export.folder")}</button>
          <button class="btn btn-primary btn-sm" type="button" disabled={busy} onclick={() => open("zip")}>{busy ? $t("tools.common.working") : $t("tools.ig.export.zip")}</button>
        </div>
      </div>
      {#if report}
        <div class="group-row"><div class="group-row-content"><div class="stats">
          <div><b>{n(report.followers.length)}</b> {$t("tools.ig.common.followers")}</div>
          <div><b>{n(report.following.length)}</b> {$t("tools.ig.common.following")}</div>
          <div><b>{n(report.mutuals)}</b> {$t("tools.ig.follow.mutuals")}</div>
          <div><b>{n(report.not_following_back.length)}</b> {$t("tools.ig.follow.not_back")}</div>
          <div><b>{n(report.fans.length)}</b> {$t("tools.ig.follow.fans")}</div>
          <div><b>{n(report.pending_sent.length)}</b> {$t("tools.ig.export.pending_sent")}</div>
        </div><div class="group-row-sub mono">{report.files_found.join(" · ")}</div></div></div>
      {/if}
    </div>
  </section>

  {#if report}
    {#if report.followers_by_month.length > 1}
      <section>
        <span class="group-label">{$t("tools.ig.export.growth")}</span>
        <div class="bars">
          {#each report.followers_by_month.slice(-24) as [m, c] (m)}
            {@const max = Math.max(...report.followers_by_month.slice(-24).map((x) => x[1]))}
            <div class="bar" title="{m}: {c}"><div class="fill" style:height="{(c / max) * 100}%"></div><span>{m.slice(2)}</span></div>
          {/each}
        </div>
      </section>
    {/if}
    <section>
      <div class="segmented wrap">
        {#each TABS as k (k)}
          {@const count = (report[k] as ExportUser[]).length}
          {#if count || k === "not_following_back"}
            <button class="segmented-btn" class:active={tab === k} type="button" onclick={() => (tab = k)}>{$t(`tools.ig.export.tab_${k}`)} · {count}</button>
          {/if}
        {/each}
      </div>
      <div class="head">
        <input class="input" type="search" bind:value={filter} placeholder={$t("tools.ig.list.filter")} />
        <div class="btn-row">
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => (selected = new Set(list.map((u) => u.username)))}>{$t("tools.ig.grid.select_all")}</button>
          <button class="btn btn-secondary btn-sm" type="button" onclick={csv}>CSV</button>
          {#if actionFor && igState.me}<button class="btn btn-secondary btn-sm" type="button" disabled={!selected.size || busy} onclick={() => (pending = { action: actionFor, users: list.filter((u) => selected.has(u.username)) })}>{$t(`tools.ig.follow.${actionFor}`)} ({selected.size})</button>{/if}
        </div>
      </div>
      <div class="group">
        {#each list.slice(0, 300) as u (u.username)}
          <div class="group-row row">
            {#if actionFor}<input type="checkbox" checked={selected.has(u.username)} onchange={() => toggle(u.username)} />{/if}
            <div class="group-row-content"><div class="group-row-title">@{u.username}</div><div class="group-row-sub">{fmtDay(u.timestamp)}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(u.href || profileUrl(u.username))}>{$t("tools.common.open")}</button></div>
          </div>
        {/each}
        {#if !list.length}<div class="group-row"><div class="group-row-sub">{$t("tools.ig.list.empty")}</div></div>{/if}
        {#if list.length > 300}<div class="group-row"><div class="group-row-sub">+{list.length - 300}</div></div>{/if}
      </div>
    </section>
    {#if actionFor}
      <IgAccountRow />
      {#if pending}
        <section><div class="group confirm"><div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t(`tools.ig.follow.confirm_${pending.action}`, { count: pending.users.length })}</div><div class="group-row-sub">{$t("tools.ig.export.resolve_hint")} · {$t("tools.ig.follow.safety")}</div></div>
          <div class="group-row-trailing btn-row"><button class="btn btn-secondary btn-sm" type="button" onclick={() => (pending = null)}>{$t("tools.ig.common.cancel")}</button><button class="btn btn-primary btn-sm" type="button" onclick={act}>{$t("tools.ig.follow.go")}</button></div>
        </div></div></section>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .stats { display: flex; flex-wrap: wrap; gap: var(--space-4); font-size: var(--text-sm); color: var(--text-muted); margin-bottom: var(--space-1); }
  .stats b { color: var(--text); }
  .wrap { flex-wrap: wrap; margin-bottom: var(--space-3); }
  .head { display: flex; justify-content: space-between; gap: var(--space-2); margin-bottom: var(--space-2); }
  .head .input { max-width: 240px; }
  .row { display: flex; align-items: center; gap: var(--space-2); }
  .bars { display: flex; align-items: flex-end; gap: 4px; height: 120px; padding: var(--space-2); background: var(--surface); border-radius: var(--radius-lg); box-shadow: inset 0 0 0 var(--hairline) var(--content-border); }
  .bar { flex: 1; display: flex; flex-direction: column; justify-content: flex-end; align-items: center; height: 100%; gap: 2px; }
  .fill { width: 100%; background: var(--accent); border-radius: 3px 3px 0 0; min-height: 2px; }
  .bar span { font-size: 9px; color: var(--text-muted); }
  .confirm { box-shadow: inset 0 0 0 2px var(--accent); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); }
</style>
