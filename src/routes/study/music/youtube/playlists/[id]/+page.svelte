<script lang="ts">
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    studyYoutubeUserPlaylistGet,
    studyYoutubeUserPlaylistRename,
    studyYoutubeUserPlaylistDelete,
    studyYoutubeUserPlaylistReorder,
    studyYoutubeUserPlaylistRemoveItem,
    studyYoutubeUserPlaylistAddItem,
    studyYoutubeSearch,
    type YoutubeUserPlaylistDetail,
    type YoutubeUserPlaylistItem,
    type YoutubeSearchVideoItem,
  } from "$lib/study-bridge";
  import MediaHero from "$lib/study-music-components/MediaHero.svelte";
  import TrackListRow from "$lib/study-music-components/TrackListRow.svelte";
  import DraggableList from "$lib/study-music-components/DraggableList.svelte";
  import EmptyPlaceholder from "$lib/study-music-components/EmptyPlaceholder.svelte";
  import YoutubeSkeleton from "$lib/study-music-youtube-components/YoutubeSkeleton.svelte";
  import YoutubeError from "$lib/study-music-youtube-components/YoutubeError.svelte";
  import { playYoutubeVideoItem } from "$lib/study-music/youtube-play-helper";
  import { musicPlayer } from "$lib/study-music/player-store.svelte";

  let playlistId = $derived(parseInt($page.params.id ?? "0", 10));
  let detail = $state<YoutubeUserPlaylistDetail | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  let renaming = $state(false);
  let renameValue = $state("");

  let confirmingDelete = $state(false);
  let pendingDelete = $state(false);

  let addOpen = $state(false);
  let addQuery = $state("");
  let addResults = $state<YoutubeSearchVideoItem[]>([]);
  let addLoading = $state(false);
  let addTimer: ReturnType<typeof setTimeout> | null = null;

  type RowItem = YoutubeUserPlaylistItem & { id: string };

  const rowItems = $derived<RowItem[]>(
    (detail?.items ?? []).map((it) => ({ ...it, id: it.video_id })),
  );

  async function reload() {
    if (!playlistId) return;
    loading = true;
    error = null;
    try {
      detail = await studyYoutubeUserPlaylistGet({ playlistId });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function asSearchVideo(it: YoutubeUserPlaylistItem): YoutubeSearchVideoItem {
    return {
      kind: "video",
      video_id: it.video_id,
      title: it.title ?? "",
      channel_id: null,
      channel_title: it.channel_title,
      thumbnail_url: it.thumbnail_url,
      duration_text: null,
      published_time_text: null,
      view_count_text: null,
      short_description: null,
      is_live: false,
    };
  }

  async function playFromIndex(idx: number, shuffle = false) {
    if (!detail || detail.items.length === 0) return;
    let queue = detail.items.map(asSearchVideo);
    let target = queue[idx];
    if (shuffle) {
      const rest = queue.slice();
      for (let i = rest.length - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [rest[i], rest[j]] = [rest[j], rest[i]];
      }
      queue = rest;
      target = queue[0];
    }
    try {
      await playYoutubeVideoItem(target, queue);
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    }
  }

  function startRename() {
    if (!detail) return;
    renameValue = detail.summary.title;
    renaming = true;
  }

  async function commitRename() {
    if (!detail || !renaming) return;
    const title = renameValue.trim();
    if (title.length === 0 || title === detail.summary.title) {
      renaming = false;
      return;
    }
    try {
      await studyYoutubeUserPlaylistRename({
        playlistId: detail.summary.id,
        title,
      });
      detail = { ...detail, summary: { ...detail.summary, title } };
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    } finally {
      renaming = false;
    }
  }

  function cancelRename() {
    renaming = false;
  }

  async function onReorder(orderedIds: Array<string | number>) {
    if (!detail) return;
    const videoIds = orderedIds.map(String);
    const next = [...detail.items];
    next.sort((a, b) => videoIds.indexOf(a.video_id) - videoIds.indexOf(b.video_id));
    next.forEach((it, idx) => (it.position = idx));
    detail = { ...detail, items: next };
    try {
      await studyYoutubeUserPlaylistReorder({
        playlistId: detail.summary.id,
        videoIds,
      });
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
      await reload();
    }
  }

  async function removeItem(videoId: string) {
    if (!detail) return;
    try {
      await studyYoutubeUserPlaylistRemoveItem({
        playlistId: detail.summary.id,
        videoId,
      });
      detail = {
        ...detail,
        items: detail.items.filter((it) => it.video_id !== videoId),
        summary: { ...detail.summary, item_count: Math.max(0, detail.summary.item_count - 1) },
      };
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    }
  }

  async function performDelete() {
    if (!detail || pendingDelete) return;
    pendingDelete = true;
    try {
      await studyYoutubeUserPlaylistDelete({ playlistId: detail.summary.id });
      confirmingDelete = false;
      void goto("/study/music/youtube/playlists");
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    } finally {
      pendingDelete = false;
    }
  }

  function openAdd() {
    addOpen = true;
    addQuery = "";
    addResults = [];
  }

  function closeAdd() {
    addOpen = false;
    if (addTimer) {
      clearTimeout(addTimer);
      addTimer = null;
    }
  }

  function scheduleAddSearch() {
    if (addTimer) clearTimeout(addTimer);
    addTimer = setTimeout(() => {
      void runAddSearch(addQuery);
    }, 320);
  }

  async function runAddSearch(query: string) {
    const q = query.trim();
    if (q.length < 2) {
      addResults = [];
      return;
    }
    addLoading = true;
    try {
      const res = await studyYoutubeSearch({ query: q, filter: "videos" });
      const flat: YoutubeSearchVideoItem[] = [];
      for (const group of res.groups) {
        for (const item of group.items) {
          if (item.kind === "video") flat.push(item);
        }
      }
      addResults = flat.slice(0, 25);
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    } finally {
      addLoading = false;
    }
  }

  async function addFromYoutube(item: YoutubeSearchVideoItem) {
    if (!detail) return;
    try {
      const res = await studyYoutubeUserPlaylistAddItem({
        playlistId: detail.summary.id,
        item: {
          video_id: item.video_id,
          title: item.title,
          channel_title: item.channel_title ?? undefined,
          thumbnail_url: item.thumbnail_url ?? undefined,
        },
      });
      detail = {
        ...detail,
        items: [
          ...detail.items,
          {
            video_id: item.video_id,
            position: res.position,
            added_at: Math.floor(Date.now() / 1000),
            title: item.title,
            channel_title: item.channel_title,
            thumbnail_url: item.thumbnail_url,
            duration_ms: null,
          },
        ],
        summary: { ...detail.summary, item_count: detail.summary.item_count + 1 },
      };
      showToast("success", $t("study.music.add_to_playlist_added"));
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    }
  }

  $effect(() => {
    if (playlistId) void reload();
  });
</script>

<section class="page">
  <button class="back" type="button" onclick={() => goto("/study/music/youtube/playlists")}>
    ← {$t("study.music.back")}
  </button>

  {#if loading && !detail}
    <div class="hero-skel"><YoutubeSkeleton kind="card" count={1} /></div>
    <div class="list-skel"><YoutubeSkeleton kind="row" count={6} /></div>
  {:else if error && !detail}
    <YoutubeError error={error} onRetry={() => { error = null; void reload(); }} />
  {:else if detail}
    {@const count = detail.summary.item_count}
    <div class="hero-wrap">
      <MediaHero
        coverUrl={detail.items[0]?.thumbnail_url ?? null}
        eyebrow={$t("study.music.eyebrow_playlist")}
        title={renaming ? "" : detail.summary.title}
        subtitle={detail.summary.description}
        stats={[
          { label: "", value: count === 1 ? $t("study.music.track_count_one") : $t("study.music.tracks_count_n", { count }) },
        ]}
        actions={count > 0
          ? [
              { id: "play", label: $t("study.music.play_all"), primary: true, onClick: () => void playFromIndex(0, false) },
              { id: "shuffle", label: $t("study.music.shuffle_play"), onClick: () => void playFromIndex(0, true) },
              { id: "add", label: $t("study.music.add_to_playlist"), onClick: openAdd },
              { id: "delete", label: $t("study.music.my_playlists_delete_confirm_action"), onClick: () => (confirmingDelete = true) },
            ]
          : [
              { id: "add", label: $t("study.music.add_to_playlist"), primary: true, onClick: openAdd },
              { id: "delete", label: $t("study.music.my_playlists_delete_confirm_action"), onClick: () => (confirmingDelete = true) },
            ]}
      />
      <div class="title-overlay">
        {#if renaming}
          <input
            class="title-input"
            type="text"
            bind:value={renameValue}
            placeholder={$t("study.music.rename_inline_placeholder")}
            onblur={commitRename}
            onkeydown={(e) => {
              if (e.key === "Enter") { e.preventDefault(); void commitRename(); }
              else if (e.key === "Escape") cancelRename();
            }}
            autofocus
            maxlength="120"
            aria-label={$t("study.music.my_playlists_rename_aria")}
          />
        {:else}
          <button
            type="button"
            class="title-button"
            onclick={startRename}
            aria-label={$t("study.music.my_playlists_rename_aria")}
          >
            <span class="visually-hidden">{detail.summary.title}</span>
            <span class="pen" aria-hidden="true">✎</span>
          </button>
        {/if}
      </div>
    </div>

    {#if count === 0}
      <EmptyPlaceholder
        title={$t("study.music.playlist_empty")}
        ctaLabel={$t("study.music.playlist_empty_cta")}
        onCta={openAdd}
      />
    {:else}
      <DraggableList
        items={rowItems}
        onReorder={onReorder}
        item={(it, idx) => itemSnippet(it, idx)}
      />
    {/if}
  {/if}
</section>

{#snippet itemSnippet(it: RowItem, idx: number)}
  <TrackListRow
    index={idx}
    coverUrl={it.thumbnail_url}
    title={it.title ?? it.video_id}
    subtitle={it.channel_title}
    durationText={null}
    isPlaying={musicPlayer.currentTrack?.youtube_video_id === it.video_id}
    onPlay={() => void playFromIndex(idx, false)}
  >
    {#snippet actions()}
      <button
        type="button"
        class="row-remove"
        title={$t("study.music.remove_from_playlist")}
        aria-label={$t("study.music.remove_from_playlist_aria", { title: it.title ?? it.video_id })}
        onclick={(e) => { e.stopPropagation(); void removeItem(it.video_id); }}
      >×</button>
    {/snippet}
  </TrackListRow>
{/snippet}

{#if confirmingDelete && detail}
  {@const count = detail.summary.item_count}
  <div class="modal-bg" role="dialog" aria-modal="true">
    <div class="modal">
      <h2>{$t("study.music.my_playlists_delete_confirm_title")}</h2>
      <p>
        {count > 0
          ? $t("study.music.my_playlists_delete_confirm_body", { count })
          : $t("study.music.my_playlists_delete_confirm_empty")}
      </p>
      <div class="modal-actions">
        <button type="button" class="ghost" onclick={() => (confirmingDelete = false)} disabled={pendingDelete}>
          {$t("study.music.my_playlists_delete_confirm_cancel")}
        </button>
        <button type="button" class="danger" onclick={performDelete} disabled={pendingDelete}>
          {$t("study.music.my_playlists_delete_confirm_action")}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if addOpen}
  <div class="modal-bg" role="dialog" aria-modal="true" aria-label={$t("study.music.add_to_playlist_modal_title")}>
    <div class="modal modal-wide">
      <header class="modal-head">
        <h2>{$t("study.music.add_to_playlist_modal_title")}</h2>
        <button type="button" class="close" onclick={closeAdd} aria-label={$t("study.music.add_to_playlist_close")}>×</button>
      </header>
      <input
        type="text"
        placeholder={$t("study.music.add_to_playlist_search_placeholder")}
        bind:value={addQuery}
        oninput={scheduleAddSearch}
        autofocus
      />
      {#if addLoading}
        <YoutubeSkeleton kind="row" count={4} />
      {:else if addQuery.trim().length < 2}
        <p class="hint">{$t("study.music.add_to_playlist_search_short")}</p>
      {:else if addResults.length === 0}
        <p class="hint">{$t("study.music.empty_search_no_results")}</p>
      {:else}
        <ul class="add-list">
          {#each addResults as item (item.video_id)}
            <li>
              <button type="button" class="add-row" onclick={() => void addFromYoutube(item)}>
                {#if item.thumbnail_url}
                  <img src={item.thumbnail_url} alt="" loading="lazy" />
                {/if}
                <div class="add-meta">
                  <span class="add-title">{item.title}</span>
                  <span class="add-channel">{item.channel_title ?? ""}</span>
                </div>
                <span class="add-cta" aria-hidden="true">+</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  </div>
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px 20px 32px;
  }
  .back {
    align-self: flex-start;
    padding: 6px 10px;
    background: transparent;
    border: 0;
    color: var(--tertiary);
    cursor: pointer;
    font-size: 13px;
  }
  .back:hover { color: var(--secondary); }
  .hero-wrap { position: relative; }
  .title-overlay {
    position: absolute;
    top: 24px;
    left: 240px;
    right: 24px;
    pointer-events: none;
  }
  .title-button {
    pointer-events: auto;
    background: transparent;
    border: 0;
    color: rgba(255, 255, 255, 0.85);
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 6px;
    font-size: 14px;
  }
  .title-button:hover { background: rgba(255, 255, 255, 0.1); }
  .pen { font-size: 16px; }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
  .title-input {
    pointer-events: auto;
    width: 100%;
    max-width: 480px;
    padding: 8px 12px;
    font-size: 32px;
    font-weight: 800;
    background: rgba(0, 0, 0, 0.35);
    color: #fff;
    border: none;
    border-radius: 8px;
  }
  .hero-skel,
  .list-skel { display: flex; flex-direction: column; gap: 8px; }
  .row-remove {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: 0;
    background: transparent;
    color: rgba(255, 255, 255, 0.55);
    cursor: pointer;
    font-size: 18px;
    line-height: 1;
  }
  .row-remove:hover {
    background: rgba(255, 255, 255, 0.1);
    color: #fff;
  }
  .modal-bg {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: grid;
    place-items: center;
    z-index: 100;
    padding: 24px;
  }
  .modal {
    background: var(--background);
    border: none;
    border-radius: 16px;
    padding: 22px;
    width: min(420px, 100%);
    display: flex;
    flex-direction: column;
    gap: 14px;
    box-shadow: 0 24px 48px rgba(0, 0, 0, 0.5);
  }
  .modal-wide { width: min(560px, 100%); max-height: 80vh; }
  .modal-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .modal h2 { margin: 0; font-size: 18px; font-weight: 800; color: var(--secondary); }
  .modal p { margin: 0; color: var(--secondary); font-size: 14px; line-height: 1.5; }
  .modal input[type="text"] {
    padding: 9px 12px;
    border-radius: 8px;
    border: none;
    background: color-mix(in oklab, var(--button) 30%, transparent);
    color: var(--secondary);
    font: inherit;
    font-size: 14px;
  }
  .close {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: transparent;
    border: 0;
    color: var(--tertiary);
    cursor: pointer;
    font-size: 20px;
    line-height: 1;
  }
  .close:hover { color: var(--secondary); }
  .modal-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 4px;
  }
  .modal-actions button {
    padding: 9px 18px;
    border-radius: 999px;
    border: 0;
    font: inherit;
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
  }
  .ghost { background: transparent; color: var(--secondary); }
  .danger { background: #d33; color: #fff; }
  .add-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow-y: auto;
    max-height: 48vh;
  }
  .add-row {
    display: grid;
    grid-template-columns: 88px 1fr auto;
    gap: 10px;
    align-items: center;
    background: transparent;
    border: 0;
    padding: 4px;
    text-align: left;
    cursor: pointer;
    border-radius: 8px;
    width: 100%;
    color: inherit;
  }
  .add-row:hover { background: rgba(255, 255, 255, 0.04); }
  .add-row img {
    width: 88px;
    height: 50px;
    object-fit: cover;
    border-radius: 4px;
  }
  .add-meta { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .add-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .add-channel { font-size: 11px; color: var(--tertiary); }
  .add-cta {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
    display: grid;
    place-items: center;
    font-size: 16px;
    font-weight: 800;
  }
  .hint { color: var(--tertiary); font-size: 13px; }
</style>
