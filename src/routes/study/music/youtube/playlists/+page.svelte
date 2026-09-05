<script lang="ts">
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    studyYoutubeUserPlaylistList,
    studyYoutubeUserPlaylistCreate,
    type YoutubeUserPlaylistSummary,
  } from "$lib/study-bridge";
  import EmptyPlaceholder from "$lib/study-music-components/EmptyPlaceholder.svelte";
  import YoutubeSkeleton from "$lib/study-music-youtube-components/YoutubeSkeleton.svelte";
  import YoutubeError from "$lib/study-music-youtube-components/YoutubeError.svelte";

  let playlists = $state<YoutubeUserPlaylistSummary[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let creating = $state(false);
  let newTitle = $state("");
  let newDescription = $state("");
  let formOpen = $state(false);

  async function reload() {
    loading = true;
    error = null;
    try {
      const res = await studyYoutubeUserPlaylistList();
      playlists = res.playlists;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function submitCreate(event: Event) {
    event.preventDefault();
    const title = newTitle.trim();
    if (!title || creating) return;
    creating = true;
    try {
      const res = await studyYoutubeUserPlaylistCreate({
        title,
        description: newDescription.trim() || undefined,
      });
      newTitle = "";
      newDescription = "";
      formOpen = false;
      void goto(`/study/music/youtube/playlists/${res.id}`);
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  function cancelCreate() {
    formOpen = false;
    newTitle = "";
    newDescription = "";
  }

  $effect(() => {
    void reload();
  });
</script>

<section class="page">
  <header class="page-header">
    <div>
      <h1>{$t("study.music.my_playlists_title")}</h1>
      <p class="subtitle">{$t("study.music.my_playlists_subtitle")}</p>
    </div>
    <button class="create-btn" type="button" onclick={() => (formOpen = true)}>
      + {$t("study.music.my_playlists_create")}
    </button>
  </header>

  {#if loading && playlists.length === 0}
    <div class="grid-skel"><YoutubeSkeleton kind="card" count={4} /></div>
  {:else if error && playlists.length === 0}
    <YoutubeError error={error} onRetry={() => { error = null; void reload(); }} />
  {:else if playlists.length === 0}
    <EmptyPlaceholder
      title={$t("study.music.my_playlists_empty_title")}
      body={$t("study.music.my_playlists_empty_body")}
      ctaLabel={$t("study.music.my_playlists_create")}
      onCta={() => (formOpen = true)}
    />
  {:else}
    <ul class="grid">
      {#each playlists as p (p.id)}
        <li>
          <a class="card" href={`/study/music/youtube/playlists/${p.id}`} aria-label={$t("study.music.my_playlists_open_aria")}>
            <div class="thumb" aria-hidden="true">
              <span class="initial">{p.title.slice(0, 1).toUpperCase()}</span>
            </div>
            <div class="meta">
              <span class="card-title" title={p.title}>{p.title}</span>
              <span class="card-count">{p.item_count === 1 ? $t("study.music.track_count_one") : $t("study.music.tracks_count_n", { count: p.item_count })}</span>
            </div>
          </a>
        </li>
      {/each}
    </ul>
  {/if}
</section>

{#if formOpen}
  <div class="modal-bg" role="dialog" aria-modal="true" aria-label={$t("study.music.my_playlists_create_title")}>
    <form class="modal" onsubmit={submitCreate}>
      <h2>{$t("study.music.my_playlists_create_title")}</h2>
      <label>
        <span>{$t("study.music.my_playlists_form_name")}</span>
        <input
          bind:value={newTitle}
          type="text"
          required
          autofocus
          maxlength="120"
          placeholder={$t("study.music.my_playlists_form_name_placeholder")}
        />
      </label>
      <label>
        <span>{$t("study.music.my_playlists_form_description")}</span>
        <textarea
          bind:value={newDescription}
          rows="3"
          maxlength="500"
        ></textarea>
      </label>
      <div class="modal-actions">
        <button type="button" class="ghost" onclick={cancelCreate} disabled={creating}>
          {$t("study.music.my_playlists_form_cancel")}
        </button>
        <button type="submit" class="primary" disabled={creating || newTitle.trim().length === 0}>
          {$t("study.music.my_playlists_form_save")}
        </button>
      </div>
    </form>
  </div>
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px 20px 32px;
  }
  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
  }
  h1 { margin: 0; font-size: 24px; font-weight: 800; color: var(--secondary); }
  .subtitle { margin: 4px 0 0; color: var(--tertiary); font-size: 13px; }
  .create-btn {
    padding: 10px 18px;
    border-radius: 999px;
    background: var(--accent);
    color: var(--on-accent);
    border: 0;
    font: inherit;
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
  }
  .create-btn:hover { filter: brightness(1.08); }
  .grid {
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 14px;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    text-decoration: none;
    color: inherit;
    padding: 12px;
    background: color-mix(in oklab, var(--button) 30%, transparent);
    border-radius: 12px;
    transition: background 160ms ease;
  }
  .card:hover { background: color-mix(in oklab, var(--button) 50%, transparent); }
  .thumb {
    aspect-ratio: 1;
    border-radius: 8px;
    background: linear-gradient(135deg, color-mix(in oklab, var(--accent) 30%, transparent), color-mix(in oklab, var(--accent) 5%, transparent));
    display: grid;
    place-items: center;
  }
  .initial {
    font-size: 44px;
    font-weight: 800;
    color: var(--accent);
  }
  .meta { display: flex; flex-direction: column; gap: 2px; }
  .card-title {
    font-size: 14px;
    font-weight: 700;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card-count { font-size: 12px; color: var(--tertiary); }
  .grid-skel {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 14px;
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
  .modal h2 { margin: 0; font-size: 18px; font-weight: 800; color: var(--secondary); }
  .modal label { display: flex; flex-direction: column; gap: 6px; font-size: 13px; color: var(--secondary); }
  .modal input,
  .modal textarea {
    padding: 9px 12px;
    border-radius: 8px;
    border: none;
    background: color-mix(in oklab, var(--button) 30%, transparent);
    color: var(--secondary);
    font: inherit;
    font-size: 14px;
    resize: vertical;
  }
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
  .ghost {
    background: transparent;
    color: var(--secondary);
  }
  .primary {
    background: var(--accent);
    color: var(--on-accent);
  }
  .primary:disabled,
  .ghost:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
  @media (prefers-reduced-motion: reduce) {
    .card { transition: none; }
  }
</style>
