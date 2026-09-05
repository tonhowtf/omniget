<script lang="ts">
  /**
   * Stories (inclusive de melhores amigos, sem marcar como visto),
   * highlights e quem viu os seus stories. `mode` = "stories" |
   * "highlights" | "viewers".
   */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/tools/rt";
  import { fmtDate, igState, jobId, n, slugArg, type Highlight, type MediaItem, type MiniUser, type Reel, type TrayEntry, type UserInfo } from "$lib/tools/ig.svelte";
  import IgAccountRow from "./IgAccountRow.svelte";
  import IgMediaGrid from "./IgMediaGrid.svelte";
  import IgUserList from "./IgUserList.svelte";

  let { mode = "stories" }: { mode?: "stories" | "highlights" | "viewers" } = $props();
  let user = $state("");
  let busy = $state(false);
  let reels = $state<Reel[]>([]);
  let highlights = $state<Highlight[]>([]);
  let items = $state<MediaItem[]>([]);
  let tray = $state<TrayEntry[]>([]);
  let myStories = $state<MediaItem[]>([]);
  let viewers = $state<{ total: number; viewers: MiniUser[]; story: MediaItem } | null>(null);
  let activeHl = $state("");

  async function run() {
    if (!user.trim() || busy) return;
    busy = true;
    items = [];
    reels = [];
    highlights = [];
    try {
      if (mode === "stories") {
        reels = await invoke<Reel[]>("tool_ig_stories", { slug: slugArg(), user: user.trim() });
        items = reels.flatMap((r) => r.items);
        if (!items.length) showToast("info", $t("tools.ig.stories.none") as string);
      } else {
        highlights = await invoke<Highlight[]>("tool_ig_highlights", { slug: slugArg(), user: user.trim() });
        if (!highlights.length) showToast("info", $t("tools.ig.stories.no_highlights") as string);
      }
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function openHighlight(h: Highlight) {
    busy = true;
    activeHl = h.id;
    try {
      items = await invoke<MediaItem[]>("tool_ig_highlight_items", { slug: slugArg(), id: h.id });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function allHighlights() {
    busy = true;
    activeHl = "*";
    try {
      items = await invoke<MediaItem[]>("tool_ig_profile_media", { slug: slugArg(), user: user.trim(), tab: "highlights", limit: 0, job: jobId("hl") });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function loadTray() {
    busy = true;
    try {
      tray = await invoke<TrayEntry[]>("tool_ig_stories_tray", { slug: slugArg() });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function loadMine(me: UserInfo | null) {
    if (!me || mode !== "viewers") return;
    busy = true;
    try {
      const r = await invoke<Reel[]>("tool_ig_stories", { slug: slugArg(), user: me.username });
      myStories = r.flatMap((x) => x.items);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function loadViewers(story: MediaItem) {
    busy = true;
    try {
      const r = await invoke<{ total: number; viewers: MiniUser[] }>("tool_ig_story_viewers", { slug: slugArg(), storyPk: story.pk, job: jobId("viewers") });
      viewers = { ...r, story };
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <IgAccountRow onready={loadMine} />
  {#if mode === "viewers"}
    <section>
      <span class="group-label">{$t("tools.ig.stories.mine")}</span>
      <div class="group">
        {#if !myStories.length}
          <div class="group-row"><div class="group-row-sub">{busy ? $t("tools.common.working") : $t("tools.ig.stories.mine_none")}</div></div>
        {/if}
        {#each myStories as s (s.pk)}
          <div class="group-row">
            <img class="mini" src={s.thumbnail} alt="" />
            <div class="group-row-content"><div class="group-row-title">{fmtDate(s.taken_at)}</div><div class="group-row-sub">{s.product_type} · {$t("tools.ig.stories.expires")} {fmtDate(s.expiring_at ?? 0)}</div></div>
            <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={() => loadViewers(s)}>{$t("tools.ig.stories.viewers")}</button></div>
          </div>
        {/each}
      </div>
    </section>
    {#if viewers}
      <IgUserList users={viewers.viewers} title={`${$t("tools.ig.stories.viewers")} · ${n(viewers.total)}`} csvName="story-viewers" />
    {/if}
  {:else}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><input class="input" type="text" bind:value={user} placeholder={$t("tools.ig.common.user_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} /></div>
          <div class="group-row-trailing btn-row">
            {#if mode === "stories"}<button class="btn btn-ghost btn-sm" type="button" disabled={busy} onclick={loadTray}>{$t("tools.ig.stories.tray")}</button>{/if}
            <button class="btn btn-primary" type="button" disabled={busy || !user.trim()} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.ig.stories.fetch")}</button>
          </div>
        </div>
        <div class="group-row"><div class="group-row-sub">{mode === "stories" ? $t("tools.ig.stories.hint") : $t("tools.ig.stories.hl_hint")}</div></div>
      </div>
    </section>
    {#if tray.length}
      <section>
        <span class="group-label">{$t("tools.ig.stories.tray")} · {tray.length}</span>
        <div class="chips">
          {#each tray as e (e.user_id)}
            <button class="chip" type="button" onclick={() => { user = e.username; run(); }}>
              <img src={e.profile_pic_url} alt="" />
              <span>@{e.username}{#if e.close_friends} 💚{/if}</span>
            </button>
          {/each}
        </div>
      </section>
    {/if}
    {#if reels.length}
      <section>
        <div class="group">
          {#each reels as r (r.id)}
            <div class="group-row"><div class="group-row-content"><div class="group-row-title">@{r.username} · {r.items.length} {$t("tools.ig.grid.items")}{#if r.close_friends} <span class="tag tag-success">{$t("tools.ig.stories.close_friends")}</span>{/if}</div><div class="group-row-sub">{$t("tools.ig.stories.not_seen")}</div></div></div>
          {/each}
        </div>
      </section>
    {/if}
    {#if highlights.length}
      <section>
        <div class="head"><span class="group-label">{highlights.length} highlights</span><button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={allHighlights}>{$t("tools.ig.stories.all_highlights")}</button></div>
        <div class="hls">
          {#each highlights as h (h.id)}
            <button class="hl" class:active={activeHl === h.id} type="button" disabled={busy} onclick={() => openHighlight(h)}>
              <img src={h.cover} alt="" />
              <span>{h.title || "—"}</span>
            </button>
          {/each}
        </div>
      </section>
    {/if}
    <IgMediaGrid {items} jobPrefix={mode} />
  {/if}
  {#if igState.me && mode === "stories"}<p class="group-footer">{$t("tools.ig.stories.cf_note")}</p>{/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .head { display: flex; justify-content: space-between; align-items: center; }
  .chips { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .chip { display: inline-flex; align-items: center; gap: var(--space-1); padding: 4px 10px 4px 4px; border-radius: 999px; border: 0; background: var(--surface); box-shadow: inset 0 0 0 var(--hairline) var(--content-border); cursor: pointer; font-size: var(--text-sm); color: var(--text); }
  .chip img { width: 24px; height: 24px; border-radius: 50%; object-fit: cover; }
  .hls { display: flex; flex-wrap: wrap; gap: var(--space-3); }
  .hl { display: flex; flex-direction: column; align-items: center; gap: 4px; width: 84px; border: 0; background: none; cursor: pointer; color: var(--text); font-size: var(--text-xs); }
  .hl img { width: 64px; height: 64px; border-radius: 50%; object-fit: cover; box-shadow: 0 0 0 2px var(--content-border); }
  .hl.active img { box-shadow: 0 0 0 3px var(--accent); }
  .hl span { max-width: 84px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mini { width: 40px; height: 40px; border-radius: var(--radius-sm, 6px); object-fit: cover; margin-right: var(--space-2); }
</style>
