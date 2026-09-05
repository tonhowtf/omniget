<script lang="ts">
  /**
   * Perfil: visualizador (bio, contagens, foto HD, relação), foto de perfil
   * em alta e download em massa (posts, reels, marcados, salvos).
   * `mode` = "viewer" | "avatar" | "media".
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount, untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openUrl, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";
  import { cancelJob, igState, jobId, n, profileUrl, recall, remember, slugArg, type MediaItem, type UserInfo } from "$lib/tools/ig.svelte";
  import IgAccountRow from "./IgAccountRow.svelte";
  import IgMediaGrid from "./IgMediaGrid.svelte";

  let { mode = "viewer", tab: initialTab = "posts" }: { mode?: "viewer" | "avatar" | "media"; tab?: string } = $props();
  let user = $state("");
  let info = $state<UserInfo | null>(null);
  let busy = $state(false);
  let tab = $state(untrack(() => initialTab));
  let limit = $state(Number(recall("media_limit", "36")));
  let items = $state<MediaItem[]>([]);
  let job = $state("");
  let progress = $state<ToolProgress | null>(null);
  let saved = $state("");
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (job && p.id === `ig:${job}`) progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function lookup() {
    const target = tab === "saved" ? igState.me?.username ?? "" : user.trim();
    if (!target || busy) return;
    busy = true;
    info = null;
    items = [];
    saved = "";
    try {
      info = await invoke<UserInfo>("tool_ig_profile", { slug: slugArg(), user: target });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function saveAvatar() {
    if (!info) return;
    const d = recall("dest") || (await pickDir());
    if (!d) return;
    remember("dest", d);
    busy = true;
    try {
      const dest = `${d}/${info.username}_profile.jpg`;
      saved = await invoke<string>("tool_save_url", { url: info.profile_pic_hd || info.profile_pic_url, dest });
      showToast("success", $t("tools.common.done") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function loadMedia() {
    const target = tab === "saved" ? igState.me?.username ?? "" : user.trim();
    if (!target || busy) return;
    remember("media_limit", String(limit));
    busy = true;
    items = [];
    progress = null;
    job = jobId("media");
    try {
      if (!info || info.username !== target) info = await invoke<UserInfo>("tool_ig_profile", { slug: slugArg(), user: target });
      items = await invoke<MediaItem[]>("tool_ig_profile_media", { slug: slugArg(), user: target, tab, limit, job });
      if (!items.length) showToast("info", $t("tools.ig.download.nothing") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <IgAccountRow />
  <section>
    <div class="group">
      {#if tab !== "saved"}
        <div class="group-row">
          <div class="group-row-content"><input class="input" type="text" bind:value={user} placeholder={$t("tools.ig.common.user_placeholder")} onkeydown={(e) => e.key === "Enter" && (mode === "media" ? loadMedia() : lookup())} /></div>
          <div class="group-row-trailing">
            {#if mode !== "media"}<button class="btn btn-primary" type="button" disabled={busy || !user.trim()} onclick={lookup}>{busy ? $t("tools.common.working") : $t("tools.ig.profile.lookup")}</button>{/if}
          </div>
        </div>
      {/if}
      {#if mode === "media"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.ig.profile.what")}</div></div>
          <div class="group-row-trailing btn-row">
            <select class="select" bind:value={tab}>
              <option value="posts">{$t("tools.ig.profile.tab_posts")}</option>
              <option value="reels">{$t("tools.ig.profile.tab_reels")}</option>
              <option value="tagged">{$t("tools.ig.profile.tab_tagged")}</option>
              <option value="saved">{$t("tools.ig.profile.tab_saved")}</option>
              <option value="stories">{$t("tools.ig.profile.tab_stories")}</option>
              <option value="highlights">{$t("tools.ig.profile.tab_highlights")}</option>
            </select>
            <select class="select" bind:value={limit}>
              <option value={12}>12</option><option value={36}>36</option><option value={100}>100</option><option value={300}>300</option><option value={0}>{$t("tools.ig.profile.all")}</option>
            </select>
            {#if busy}<button class="btn btn-secondary btn-sm" type="button" onclick={() => cancelJob(job)}>{$t("tools.ig.common.cancel")}</button>
            {:else}<button class="btn btn-primary" type="button" disabled={tab !== "saved" && !user.trim()} onclick={loadMedia}>{$t("tools.ig.profile.list")}</button>{/if}
          </div>
        </div>
        {#if busy && progress}<div class="group-row"><div class="group-row-sub mono">{progress.stage} · {progress.done}{#if progress.total}/{progress.total}{/if} {progress.message ?? ""}</div></div>{/if}
      {/if}
    </div>
    {#if mode === "media"}<p class="group-footer">{$t("tools.ig.profile.media_hint")}</p>{/if}
  </section>

  {#if info}
    <section>
      <div class="group">
        <div class="group-row profile">
          <img class="pic" class:big={mode === "avatar"} src={info.profile_pic_hd || info.profile_pic_url} alt="" />
          <div class="group-row-content">
            <div class="group-row-title">@{info.username} {#if info.is_verified}✓{/if} {#if info.is_private}<span class="tag">{$t("tools.ig.list.private")}</span>{/if} {#if info.is_business}<span class="tag tag-accent">{info.category || "business"}</span>{/if}</div>
            <div class="group-row-sub">{info.full_name}</div>
            <div class="stats">
              <div><b>{n(info.media_count)}</b> {$t("tools.ig.common.posts")}</div>
              <div><b>{n(info.follower_count)}</b> {$t("tools.ig.common.followers")}</div>
              <div><b>{n(info.following_count)}</b> {$t("tools.ig.common.following")}</div>
              {#if info.total_clips}<div><b>{n(info.total_clips)}</b> reels</div>{/if}
            </div>
            {#if info.biography}<div class="bio">{info.biography}</div>{/if}
            {#if info.external_url}<button class="link" type="button" onclick={() => openUrl(info!.external_url)}>{info.external_url}</button>{/if}
            <div class="group-row-sub">
              {#if info.is_self}{$t("tools.ig.profile.you")}{:else}{info.followed_by_viewer ? $t("tools.ig.profile.you_follow") : $t("tools.ig.profile.you_dont_follow")} · {info.follows_viewer ? $t("tools.ig.profile.follows_you") : $t("tools.ig.profile.doesnt_follow_you")}{/if}
            </div>
          </div>
          <div class="group-row-trailing btn-row col">
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => openUrl(profileUrl(info!.username))}>{$t("tools.common.open")}</button>
            <button class="btn btn-primary btn-sm" type="button" disabled={busy} onclick={saveAvatar}>{$t("tools.ig.profile.save_avatar")}</button>
            {#if saved}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(saved)}>{$t("tools.common.reveal")}</button>{/if}
          </div>
        </div>
      </div>
      {#if mode === "viewer"}
        <div class="btn-row wrap">
          <a class="btn btn-secondary btn-sm" href="/tools/instagram/ig-stories">{$t("tools.ig.profile.go_stories")}</a>
          <a class="btn btn-secondary btn-sm" href="/tools/instagram/ig-highlights">{$t("tools.ig.profile.go_highlights")}</a>
          <a class="btn btn-secondary btn-sm" href="/tools/instagram/ig-profile-media">{$t("tools.ig.profile.go_media")}</a>
          <a class="btn btn-secondary btn-sm" href="/tools/instagram/ig-analytics">{$t("tools.ig.profile.go_analytics")}</a>
        </div>
      {/if}
    </section>
  {/if}
  <IgMediaGrid {items} jobPrefix="profile" />
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .profile { align-items: flex-start; }
  .pic { width: 72px; height: 72px; border-radius: 50%; object-fit: cover; margin-right: var(--space-3); flex: none; }
  .pic.big { width: 160px; height: 160px; }
  .stats { display: flex; gap: var(--space-4); margin: var(--space-2) 0; font-size: var(--text-sm); color: var(--text-muted); }
  .stats b { color: var(--text); }
  .bio { white-space: pre-wrap; font-size: var(--text-sm); margin-bottom: var(--space-1); }
  .link { border: 0; background: none; color: var(--accent-hi); cursor: pointer; padding: 0; font-size: var(--text-sm); }
  .col { flex-direction: column; align-items: stretch; }
  .wrap { flex-wrap: wrap; margin-top: var(--space-3); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); }
</style>
