<script lang="ts">
  /** Quem não me segue de volta (estudo 67): auditoria, whitelist e unfollow com jitter e limite diário. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { onToolProgress, openUrl, reveal, saveAs, type ToolProgress } from "$lib/tools/rt";
  import { fmtN, xErr, type XSession, type XUser } from "$lib/tools/x";
  import XSessionRow from "./XSession.svelte";

  type Audit = { me: XUser; following: number; followers: number; mutuals: number; not_following_back: XUser[]; fans: XUser[]; whitelist: string[]; cancelled: boolean; unfollowed_today: number };
  type UnfollowResult = { done: string[]; failed: [string, string][]; stopped: boolean; reason: string };

  let sess = $state<XSession | null>(null);
  let busy = $state<string | null>(null);
  let audit = $state<Audit | null>(null);
  let progress = $state<ToolProgress | null>(null);
  let tab = $state<"nfb" | "fans">("nfb");
  let selected = $state<Set<string>>(new Set());
  let whitelistText = $state("");
  let minDelay = $state(15);
  let maxDelay = $state(40);
  let dailyCap = $state(100);
  let filter = $state("");
  let last = $state<UnfollowResult | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "x-follows" || p.id === "x-unfollow") progress = p;
    });
    whitelistText = (await invoke<string[]>("tool_x_whitelist_get")).join("\n");
  });
  onDestroy(() => unlisten?.());

  async function runAudit() {
    if (busy) return;
    busy = "audit";
    audit = null;
    selected = new Set();
    progress = null;
    try {
      audit = await invoke<Audit>("tool_x_follows_audit", { limit: 0 });
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = null;
    }
  }

  async function saveWhitelist() {
    try {
      const list = await invoke<string[]>("tool_x_whitelist_set", { handles: whitelistText.split(/[\n,\s]+/).filter(Boolean) });
      whitelistText = list.join("\n");
      showToast("success", $t("tools.common.done") as string);
      if (audit) audit = { ...audit, whitelist: list, not_following_back: audit.not_following_back.filter((u) => !list.includes(u.handle.toLowerCase())) };
    } catch (e) {
      showToast("error", xErr(e));
    }
  }

  function toggle(id: string) {
    const s = new Set(selected);
    if (s.has(id)) s.delete(id);
    else s.add(id);
    selected = s;
  }

  let list = $derived.by(() => {
    const src = tab === "nfb" ? audit?.not_following_back ?? [] : audit?.fans ?? [];
    const q = filter.trim().toLowerCase();
    return q ? src.filter((u) => u.handle.toLowerCase().includes(q) || u.name.toLowerCase().includes(q)) : src;
  });

  function selectAll(on: boolean) {
    selected = on ? new Set(list.map((u) => u.id)) : new Set();
  }

  async function unfollow() {
    if (!audit || !selected.size || busy) return;
    const n = selected.size;
    if (!confirm(($t("tools.x.unfollow_confirm", { n }) as string))) return;
    busy = "unfollow";
    last = null;
    progress = null;
    try {
      const ids = audit.not_following_back.filter((u) => selected.has(u.id)).map((u) => u.id);
      last = await invoke<UnfollowResult>("tool_x_unfollow", { ids, minDelay, maxDelay, dailyCap });
      const done = new Set(last.done);
      audit = { ...audit, following: audit.following - done.size, not_following_back: audit.not_following_back.filter((u) => !done.has(u.id)), unfollowed_today: audit.unfollowed_today + done.size };
      selected = new Set();
      showToast(last.stopped ? "info" : "success", `${last.done.length} ${$t("tools.x.unfollowed")}`);
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = null;
    }
  }

  async function exportList(format: "csv" | "json") {
    const dest = await saveAs(`x-${tab === "nfb" ? "not-following-back" : "fans"}.${format}`);
    if (!dest) return;
    try {
      const users = tab === "nfb" ? audit?.not_following_back ?? [] : audit?.fans ?? [];
      await reveal(await invoke<string>("tool_x_export_users", { users, format, dest }));
    } catch (e) {
      showToast("error", xErr(e));
    }
  }

  let stage = $derived.by(() => {
    if (!progress) return "";
    if (progress.stage === "following" || progress.stage === "followers") return `${$t(`tools.x.${progress.stage}`)}: ${progress.done}`;
    if (progress.stage === "waiting") return `${$t("tools.x.waiting")} ${progress.message}s · ${progress.done}/${progress.total}`;
    if (progress.stage === "unfollow") return `${$t("tools.x.unfollowing")} ${progress.done + 1}/${progress.total}`;
    return progress.stage;
  });
</script>

<div class="tool">
  <section>
    <div class="group"><XSessionRow required onchange={(s) => (sess = s)} /></div>
  </section>
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.x.audit")}</div>
          <div class="group-row-sub">{busy === "audit" ? stage : $t("tools.x.audit_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if busy === "audit"}<button class="btn btn-secondary" type="button" onclick={() => invoke("tool_x_cancel", { job: "x-follows" })}>{$t("tools.x.stop")}</button>{/if}
          <button class="btn btn-primary" type="button" disabled={busy !== null || !sess?.logged_in} onclick={runAudit}>{busy === "audit" ? $t("tools.common.working") : $t("tools.x.run_audit")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.x.whitelist")}</div><div class="group-row-sub">{$t("tools.x.whitelist_hint")}</div><textarea class="input area" rows="3" bind:value={whitelistText} placeholder="@amigo&#10;@empresa"></textarea></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={saveWhitelist}>{$t("tools.common.save")}</button></div>
      </div>
    </div>
  </section>

  {#if audit}
    <section>
      <div class="group"><div class="group-row"><div class="tiles">
        <div class="tile"><div class="v">{fmtN(audit.following)}</div><div class="k">{$t("tools.x.following")}</div></div>
        <div class="tile"><div class="v">{fmtN(audit.followers)}</div><div class="k">{$t("tools.x.followers")}</div></div>
        <div class="tile"><div class="v">{fmtN(audit.mutuals)}</div><div class="k">{$t("tools.x.mutuals")}</div></div>
        <div class="tile warn"><div class="v">{fmtN(audit.not_following_back.length)}</div><div class="k">{$t("tools.x.not_following_back")}</div></div>
        <div class="tile"><div class="v">{fmtN(audit.fans.length)}</div><div class="k">{$t("tools.x.fans")}</div></div>
        <div class="tile"><div class="v">{audit.unfollowed_today}</div><div class="k">{$t("tools.x.unfollowed_today")}</div></div>
      </div></div></div>
    </section>

    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content btn-row">
            <div class="segmented"><button class="segmented-btn" class:active={tab === "nfb"} type="button" onclick={() => { tab = "nfb"; selected = new Set(); }}>{$t("tools.x.not_following_back")}</button><button class="segmented-btn" class:active={tab === "fans"} type="button" onclick={() => { tab = "fans"; selected = new Set(); }}>{$t("tools.x.fans")}</button></div>
            <input class="input small" type="search" bind:value={filter} placeholder={$t("tools.x.filter")} />
          </div>
          <div class="group-row-trailing btn-row">
            {#if tab === "nfb"}
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => selectAll(true)}>{$t("tools.x.select_all")}</button>
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => selectAll(false)}>{$t("tools.x.select_none")}</button>
            {/if}
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => exportList("csv")}>CSV</button>
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => exportList("json")}>JSON</button>
          </div>
        </div>
        <div class="list">
          {#each list as u (u.id)}
            <label class="row">
              {#if tab === "nfb"}<input class="checkbox" type="checkbox" checked={selected.has(u.id)} onchange={() => toggle(u.id)} />{/if}
              {#if u.avatar}<img class="avatar" src={u.avatar} alt="" loading="lazy" />{/if}
              <span class="who"><b>{u.name}</b> <span class="dim">@{u.handle}</span>{#if u.verified} ✓{/if}{#if u.protected} 🔒{/if}<br /><span class="dim small">{fmtN(u.followers)} {$t("tools.x.followers")} · {fmtN(u.posts)} {$t("tools.x.posts")}{#if u.bio} · {u.bio.slice(0, 80)}{/if}</span></span>
              <button class="link" type="button" onclick={() => openUrl(`https://x.com/${u.handle}`)}>↗</button>
            </label>
          {:else}
            <div class="group-row"><div class="group-row-sub">{$t("tools.x.empty_list")}</div></div>
          {/each}
        </div>
      </div>
    </section>

    {#if tab === "nfb"}
      <section>
        <div class="group">
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.x.unfollow_safety")}</div><div class="group-row-sub">{$t("tools.x.unfollow_safety_hint")}</div></div>
            <div class="group-row-trailing btn-row">
              <label class="opt">{$t("tools.x.delay")} <input class="input tiny" type="number" min="3" max="600" bind:value={minDelay} /> – <input class="input tiny" type="number" min="3" max="600" bind:value={maxDelay} /> s</label>
              <label class="opt">{$t("tools.x.daily_cap")} <input class="input tiny" type="number" min="1" max="2000" bind:value={dailyCap} /></label>
            </div>
          </div>
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub">{busy === "unfollow" ? stage : `${selected.size} ${$t("tools.x.selected")}`}</div></div>
            <div class="group-row-trailing btn-row">
              {#if busy === "unfollow"}<button class="btn btn-secondary" type="button" onclick={() => invoke("tool_x_cancel", { job: "x-unfollow" })}>{$t("tools.x.stop")}</button>{/if}
              <button class="btn btn-destructive" type="button" disabled={busy !== null || !selected.size} onclick={unfollow}>{busy === "unfollow" ? $t("tools.common.working") : `${$t("tools.x.unfollow_selected")} (${selected.size})`}</button>
            </div>
          </div>
          {#if last}
            <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{last.done.length} {$t("tools.x.unfollowed")} · {last.failed.length} {$t("tools.common.failed")}{#if last.stopped} · {$t("tools.x.stopped")}: {last.reason}{/if}</div></div></div>
          {/if}
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .area { width: 100%; margin-top: var(--space-1); font-family: var(--font-mono); font-size: var(--text-xs); }
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); gap: var(--space-2); width: 100%; }
  .tile { padding: var(--space-2) var(--space-3); border-radius: var(--radius-md); background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .tile.warn { background: color-mix(in srgb, var(--warning) 14%, transparent); }
  .tile .v { font-family: var(--font-display); font-size: var(--text-lg); font-weight: 700; color: var(--text); }
  .tile .k { font-size: var(--text-xs); color: var(--text-muted); }
  .list { max-height: 480px; overflow: auto; }
  .row { display: flex; align-items: center; gap: var(--space-2); padding: var(--space-2) var(--space-4); border-top: var(--hairline) solid var(--content-border); cursor: pointer; }
  .avatar { width: 32px; height: 32px; border-radius: 50%; flex-shrink: 0; }
  .who { flex: 1; min-width: 0; line-height: 1.3; font-size: var(--text-sm); }
  .dim { color: var(--text-muted); }
  .small { font-size: var(--text-xs); }
  .link { background: none; border: 0; color: var(--accent-hi); cursor: pointer; font-size: var(--text-base); }
  .opt { display: inline-flex; align-items: center; gap: var(--space-1); font-size: var(--text-sm); white-space: nowrap; }
  .tiny { width: 64px; }
  .small.input, .input.small { max-width: 220px; }
</style>
