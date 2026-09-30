<script lang="ts">
  // "Save to..." : Watch later plus the account playlists, each a checkbox,
  // and "New playlist". Without an account only Watch later (local) shows.
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TIcon from "../TIcon.svelte";
  import Popup from "./Popup.svelte";
  import { ensureSession, loggedIn } from "./session.svelte";
  import { errText } from "./util";

  let {
    videoId,
    anchor = null,
    saved = null,
    onclose,
    meta = null,
  }: {
    videoId: string;
    anchor?: HTMLElement | null;
    saved?: boolean | null;
    onclose: () => void;
    /** Shown in the local Watch later when there is no account. */
    meta?: { title?: string | null; channel_id?: string | null; channel_name?: string | null; avatar?: string | null } | null;
  } = $props();

  type Row = { id: string; title: string; contains: boolean };

  let rows = $state<Row[] | null>(null);
  let wl = $state(false);
  let noAccount = $state(false);
  let creating = $state(false);
  let name = $state("");
  let working = $state<string | null>(null);

  $effect(() => {
    wl = !!saved;
    void (async () => {
      await ensureSession();
      if (!loggedIn()) {
        noAccount = true;
        rows = [];
        return;
      }
      try {
        const list = await api.savePlaylists(videoId);
        const w = list.find((p) => p.id === "WL");
        if (w) wl = w.contains;
        rows = list.filter((p) => p.id !== "WL");
      } catch {
        rows = [];
      }
    })();
  });

  async function toggleWl() {
    const next = !wl;
    wl = next;
    try {
      await api.action(next ? "watch_later_add" : "watch_later_remove", { video_id: videoId, playlist_id: "WL", ...(meta ?? {}) });
      showToast("success", $t(next ? "tube.save.added_wl" : "tube.save.removed_wl") as string);
    } catch (e) {
      wl = !next;
      showToast("error", errText(e));
    }
  }

  async function toggle(r: Row) {
    if (working) return;
    working = r.id;
    const next = !r.contains;
    r.contains = next;
    try {
      await api.action(next ? "playlist_add" : "playlist_remove", { video_id: videoId, playlist_id: r.id });
      showToast("success", $t(next ? "tube.save.added" : "tube.save.removed", { name: r.title }) as string);
    } catch (e) {
      r.contains = !next;
      showToast("error", errText(e));
    } finally {
      working = null;
    }
  }

  async function create(e: SubmitEvent) {
    e.preventDefault();
    const title = name.trim();
    if (!title) return;
    try {
      const id = await api.createPlaylist(title, [videoId]);
      rows = [...(rows ?? []), { id, title, contains: true }];
      name = "";
      creating = false;
      showToast("success", $t("tube.save.added", { name: title }) as string);
    } catch (err) {
      showToast("error", errText(err));
    }
  }
</script>

<Popup {anchor} {onclose} modal={!anchor} class="yt-menu save" label={$t("tube.save.title") as string} width={300}>
  <header>
    <span>{$t("tube.save.title")}</span>
    <button class="yt-btn yt-btn--icon x" onclick={onclose} aria-label={$t("tube.common.close") as string}><TIcon name="close" size={20} /></button>
  </header>
  <button class="yt-menu-item" role="menuitemcheckbox" aria-checked={wl} onclick={toggleWl}>
    <span class="box" class:on={wl}>{#if wl}<TIcon name="check" size={16} stroke={3} />{/if}</span>
    <span class="grow">{$t("tube.save.watch_later")}</span>
    <TIcon name="clock" size={18} />
  </button>
  {#if rows === null}
    {#each Array(3) as _, i (i)}
      <div class="skrow"><span class="yt-skel"></span></div>
    {/each}
  {:else}
    {#each rows as r (r.id)}
      <button class="yt-menu-item" role="menuitemcheckbox" aria-checked={r.contains} disabled={working === r.id} onclick={() => toggle(r)}>
        <span class="box" class:on={r.contains}>{#if r.contains}<TIcon name="check" size={16} stroke={3} />{/if}</span>
        <span class="grow yt-clamp-1">{r.title}</span>
      </button>
    {/each}
  {/if}
  {#if noAccount}
    <p class="note">{$t("tube.save.no_account")}</p>
  {:else if creating}
    <form class="new" onsubmit={create}>
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={name} placeholder={$t("tube.save.new_name") as string} maxlength="150" autofocus />
      <div class="new-actions">
        <button type="button" class="yt-btn yt-btn--ghost yt-btn--s" onclick={() => (creating = false)}>{$t("tube.common.cancel")}</button>
        <button type="submit" class="yt-btn yt-btn--s yt-btn--inverted" disabled={!name.trim()}>{$t("tube.save.create")}</button>
      </div>
    </form>
  {:else}
    <div class="yt-menu-sep"></div>
    <button class="yt-menu-item" role="menuitem" onclick={() => (creating = true)}>
      <TIcon name="plus" size={20} />
      <span>{$t("tube.save.new_playlist")}</span>
    </button>
  {/if}
</Popup>

<style>
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 8px 4px 16px;
    font: var(--yt-type-card-title);
  }
  .x {
    width: 36px;
    height: 36px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .box {
    display: grid;
    place-items: center;
    flex: none;
    width: 20px;
    height: 20px;
    border-radius: 4px;
    box-shadow: inset 0 0 0 2px var(--yt-text-2);
    transition: background var(--yt-dur-fast), box-shadow var(--yt-dur-fast);
  }
  .box.on {
    color: var(--yt-on-inverted);
    background: var(--yt-bg-inverted);
    box-shadow: none;
  }
  .skrow {
    height: 36px;
    padding: 0 16px;
    display: flex;
    align-items: center;
  }
  .skrow .yt-skel {
    width: 60%;
    height: 14px;
  }
  .note {
    margin: 8px 16px 4px;
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .new {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 16px 4px;
  }
  .new input {
    height: 32px;
    padding: 0 2px;
    border: 0;
    border-bottom: 1px solid var(--yt-outline);
    font: var(--yt-type-meta);
    color: var(--yt-text);
    background: transparent;
    outline: none;
  }
  .new input:focus {
    border-bottom: 2px solid var(--yt-text);
  }
  .new-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
