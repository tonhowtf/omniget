<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgIcon from "./TgIcon.svelte";
  import OutputDir from "./OutputDir.svelte";
  import TgModal from "./TgModal.svelte";
  import Player from "./Player.svelte";
  import ContinueWatching from "./ContinueWatching.svelte";
  import WatchRuleEditor from "./WatchRuleEditor.svelte";
  import ChatDownloadDialog from "./ChatDownloadDialog.svelte";
  import UploadPanel from "./UploadPanel.svelte";
  import ForwardDialog from "./ForwardDialog.svelte";
  import TranscriptPanel from "./TranscriptPanel.svelte";
  import ScanList from "./ScanList.svelte";
  import {
    tg,
    tg3,
    addedCount,
    errText,
    formatBytes,
    formatDate,
    isPlayable,
    type PlaybackEntry,
    type TelegramChat,
    type TelegramMediaItem,
  } from "./api";
  import { dateToUnix } from "./filters";
  import {
    chatsState,
    loadChats,
    thumbnail,
    batches,
    trackBatch,
    batchCounts,
    fileStatus,
    chatRef,
    resumeOf,
    loadRecent,
    failText,
  } from "./session.svelte";

  let {
    outputDir = $bindable(""),
    onClone,
  }: { outputDir: string; onClone?: (chat: TelegramChat) => void } = $props();

  type PlayerOpen = { chat: TelegramChat; queue: TelegramMediaItem[]; start: number; startAt?: number; transcript?: boolean };
  type Dialog = "watch" | "chat_dl" | "upload" | "forward" | null;
  let player = $state<PlayerOpen | null>(null);
  let dialog = $state<Dialog>(null);

  const FILTERS = ["all", "photo", "video", "document", "audio"] as const;
  type Filter = (typeof FILTERS)[number];
  const PAGE = 60;

  let query = $state("");
  let selected = $state<TelegramChat | null>(null);
  let filter = $state<Filter>("all");
  let items = $state<TelegramMediaItem[]>([]);
  let loading = $state(false);
  let hasMore = $state(true);
  let error = $state<string | null>(null);
  let picked = $state<Record<number, boolean>>({});
  let mediaQuery = $state("");
  let searching = $state(false);
  let loadSeq = 0;
  // Extra search filters (period and size), folded under a button.
  let showFilters = $state(false);
  let sFrom = $state("");
  let sTo = $state("");
  let sMinMb = $state("");
  let sMaxMb = $state("");
  let transcribing = $state<TelegramMediaItem | null>(null);
  let addingWatch = $state(false);

  const MB = 1024 * 1024;
  const mbOf = (v: string) => {
    const n = Number(String(v).replace(",", "."));
    return Number.isFinite(n) && n > 0 ? Math.round(n * MB) : undefined;
  };
  let filterCount = $derived([sFrom, sTo, sMinMb, sMaxMb].filter((v) => String(v).trim()).length);

  let chats = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return q ? chatsState.chats.filter((c) => c.title.toLowerCase().includes(q)) : chatsState.chats;
  });
  let pickedItems = $derived(items.filter((i) => picked[i.message_id]));
  let pickedBytes = $derived(pickedItems.reduce((s, i) => s + (i.file_size || 0), 0));
  let activeBatches = $derived(Object.values(batches).filter((b) => !b.finished).reverse());
  let playable = $derived(items.filter(isPlayable));

  onMount(() => {
    if (!chatsState.loaded) void loadChats(true);
  });

  function openChat(c: TelegramChat) {
    if (selected && selected.id === c.id && selected.chat_type === c.chat_type) return;
    selected = c;
    mediaQuery = "";
    void loadMedia(true);
  }

  function setFilter(f: Filter) {
    filter = f;
    if (selected) void loadMedia(true);
  }

  async function loadMedia(reset: boolean) {
    if (!selected) return;
    const seq = ++loadSeq;
    const chat = selected;
    loading = true;
    error = null;
    if (reset) {
      items = [];
      picked = {};
      hasMore = true;
      searching = false;
    }
    try {
      const page = await tg.listMedia(chat, filter === "all" ? null : filter, reset ? 0 : items.length, PAGE);
      if (seq !== loadSeq) return;
      const seen = new Set(items.map((i) => i.message_id));
      items = reset ? page : [...items, ...page.filter((i) => !seen.has(i.message_id))];
      hasMore = page.length >= PAGE;
    } catch (e) {
      if (seq === loadSeq) error = errText(e);
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  async function search() {
    if (!selected) return;
    const q = mediaQuery.trim();
    if (!q && !filterCount) return void loadMedia(true);
    const seq = ++loadSeq;
    loading = true;
    error = null;
    picked = {};
    try {
      const mediaType = filter === "all" ? null : filter;
      const res = filterCount
        ? await tg3.searchMedia(selected, {
            query: q,
            mediaType,
            fromDate: dateToUnix(sFrom),
            toDate: dateToUnix(sTo, true),
            minSize: mbOf(sMinMb),
            maxSize: mbOf(sMaxMb),
            limit: 200,
          })
        : await tg.searchMedia(selected, q, mediaType, 100);
      if (seq !== loadSeq) return;
      items = res;
      hasMore = false;
      searching = true;
    } catch (e) {
      if (seq === loadSeq) error = errText(e);
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  function toggle(id: number) {
    picked[id] = !picked[id];
  }

  // With a selection going, a click adds to it; otherwise videos and audio open the player.
  function activate(item: TelegramMediaItem, e: MouseEvent) {
    if (!selected) return;
    if (pickedItems.length || e.metaKey || e.ctrlKey || e.shiftKey || !isPlayable(item)) {
      toggle(item.message_id);
      return;
    }
    const queue = playable;
    player = { chat: selected, queue, start: Math.max(0, queue.findIndex((q) => q.message_id === item.message_id)) };
  }

  function openRecent(r: PlaybackEntry) {
    const chat = chatRef(r.chat_id, r.chat_type, r.chat_title);
    const same = selected && selected.id === chat.id && selected.chat_type === chat.chat_type;
    const at = same ? playable.findIndex((q) => q.message_id === r.message_id) : -1;
    if (at >= 0) {
      player = { chat, queue: playable, start: at };
      return;
    }
    const single: TelegramMediaItem = {
      message_id: r.message_id,
      file_name: r.file_name || `#${r.message_id}`,
      file_size: 0,
      media_type: r.media_type || "video",
      date: 0,
    };
    player = { chat, queue: [single], start: 0 };
  }

  function clearFilters() {
    sFrom = "";
    sTo = "";
    sMinMb = "";
    sMaxMb = "";
    if (searching) void search();
  }

  // A timestamp in the transcript dialog opens the player right there.
  function playAt(item: TelegramMediaItem, sec: number) {
    if (!selected) return;
    transcribing = null;
    const queue = playable;
    const at = queue.findIndex((q) => q.message_id === item.message_id);
    player = at >= 0
      ? { chat: selected, queue, start: at, startAt: sec, transcript: true }
      : { chat: selected, queue: [item], start: 0, startAt: sec, transcript: true };
  }

  async function addFolderToWatch() {
    if (!outputDir) {
      showToast("info", $t("telegram.common.choose_folder_first") as string);
      return;
    }
    addingWatch = true;
    try {
      const n = addedCount(await tg3.addToWatch({ folder: outputDir }));
      showToast("success", (n == null ? $t("telegram.watch_lib.added_folder") : $t("telegram.watch_lib.added", { count: n })) as string);
    } catch (e) {
      showToast("error", failText(e));
    } finally {
      addingWatch = false;
    }
  }

  function closePlayer() {
    player = null;
    void loadRecent();
  }

  async function downloadOne(item: TelegramMediaItem) {
    const chat = player?.chat ?? selected;
    if (!chat) return;
    if (!outputDir) {
      showToast("info", $t("telegram.common.choose_folder_first") as string);
      return;
    }
    try {
      const id = await tg.downloadBatch(chat, [item], outputDir);
      trackBatch(id, chat.title, [item.message_id]);
      showToast("success", $t("telegram.browser.download_started", { count: 1 }) as string);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  function selectAll() {
    const next: Record<number, boolean> = {};
    for (const i of items) next[i.message_id] = true;
    picked = next;
  }

  async function download() {
    if (!selected || !pickedItems.length) return;
    if (!outputDir) {
      showToast("info", $t("telegram.common.choose_folder_first") as string);
      return;
    }
    try {
      const list = [...pickedItems];
      const id = await tg.downloadBatch(selected, list, outputDir);
      trackBatch(id, selected.title, list.map((i) => i.message_id));
      showToast("success", $t("telegram.browser.download_started", { count: list.length }) as string);
      picked = {};
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function cancel(id: number) {
    try {
      await tg.cancelBatch(id);
      const b = batches[id];
      if (b) {
        b.cancelled = true;
        b.finished = true;
      }
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  function typeIcon(kind: string): string {
    if (kind === "photo") return "photo";
    if (kind === "video" || kind === "gif") return "video";
    if (kind === "audio" || kind === "voice") return "music";
    return "file";
  }

  function initials(title: string): string {
    return (title.trim()[0] ?? "?").toUpperCase();
  }

  // Loads the preview only once the tile scrolls into view.
  function thumb(node: HTMLElement, args: { chat: TelegramChat; item: TelegramMediaItem }) {
    if (!["photo", "video", "gif"].includes(args.item.media_type)) return;
    let done = false;
    const io = new IntersectionObserver((entries) => {
      if (done || !entries.some((e) => e.isIntersecting)) return;
      done = true;
      io.disconnect();
      void thumbnail(args.chat, args.item.message_id).then((url) => {
        if (url) node.style.backgroundImage = `url("${url}")`;
        if (url) node.classList.add("has-thumb");
      });
    }, { rootMargin: "200px" });
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }
</script>

<div class="wrap">
<ContinueWatching onopen={openRecent} />
<div class="browser">
  <aside class="chats">
    <input class="input input-search" type="search" bind:value={query} placeholder={$t("telegram.browser.search_chats")} />
    <div class="chat-list">
      {#each chats as c (`${c.chat_type}:${c.id}`)}
        <button class="chat" class:sel={selected?.id === c.id && selected?.chat_type === c.chat_type} onclick={() => openChat(c)}>
          <span class="avatar" data-type={c.chat_type}>{initials(c.title)}</span>
          <span class="chat-body">
            <span class="chat-title">{c.title}</span>
            <span class="chat-sub">{$t(`telegram.browser.chat_type.${c.chat_type}`)}{c.last_message ? ` · ${c.last_message}` : ""}</span>
          </span>
          {#if c.is_pinned}<TgIcon name="pin" size={12} />{/if}
        </button>
      {:else}
        {#if chatsState.loading}
          {#each [0, 1, 2, 3, 4] as i (i)}<div class="chat-skel"><span class="skeleton circle"></span><span class="skeleton"></span></div>{/each}
        {:else if chatsState.error}
          <p class="field-error pad">{chatsState.error}</p>
          <button class="btn btn-secondary btn-sm" onclick={() => void loadChats(true)}>{$t("telegram.common.retry")}</button>
        {:else}
          <p class="dim pad">{$t("telegram.browser.no_chats")}</p>
        {/if}
      {/each}
      {#if chatsState.next && !query}
        <button class="btn btn-ghost btn-sm more" onclick={() => void loadChats()} disabled={chatsState.loading}>
          {#if chatsState.loading}<span class="spinner"></span>{/if}{$t("telegram.browser.more_chats")}
        </button>
      {/if}
    </div>
  </aside>

  <section class="media">
    {#if !selected}
      <div class="empty-state">
        <div class="empty-state-title">{$t("telegram.browser.pick_chat")}</div>
        <div class="empty-state-hint">{$t("telegram.browser.pick_chat_hint")}</div>
      </div>
    {:else}
      <header class="m-head">
        <h3 title={selected.title}>{selected.title}</h3>
        <div class="segmented" role="tablist">
          {#each FILTERS as f (f)}
            <button class="segmented-btn" role="tab" aria-selected={filter === f} onclick={() => setFilter(f)}>{$t(`telegram.browser.filter.${f}`)}</button>
          {/each}
        </div>
      </header>
      <div class="m-actions">
        <button class="btn btn-tinted btn-sm" onclick={() => (dialog = "watch")}><TgIcon name="bolt" /> {$t("telegram.browser.save_new")}</button>
        <button class="btn btn-ghost btn-sm" onclick={() => (dialog = "chat_dl")}><TgIcon name="download" /> {$t("telegram.browser.download_all")}</button>
        <button class="btn btn-ghost btn-sm" onclick={() => (dialog = "upload")}><TgIcon name="upload" /> {$t("telegram.browser.send_files")}</button>
        {#if onClone && selected.chat_type !== "private"}
          <button class="btn btn-ghost btn-sm" onclick={() => selected && onClone?.(selected)}><TgIcon name="clone" /> {$t("telegram.browser.clone_chat")}</button>
        {/if}
      </div>
      <div class="m-tools">
        <form class="m-search" onsubmit={(e) => { e.preventDefault(); void search(); }}>
          <input class="input input-search" type="search" bind:value={mediaQuery} placeholder={$t("telegram.browser.search_media")} />
        </form>
        <button class="btn btn-ghost btn-sm" class:on={showFilters || filterCount > 0} onclick={() => (showFilters = !showFilters)} aria-expanded={showFilters} title={$t("telegram.search.filters")}>
          <TgIcon name="filter" /> {filterCount ? $t("telegram.search.filters_n", { count: filterCount }) : $t("telegram.search.filters")}
        </button>
        <button class="btn btn-ghost btn-sm" onclick={selectAll} disabled={!items.length}>{$t("telegram.browser.select_all")}</button>
        {#if pickedItems.length}<button class="btn btn-ghost btn-sm" onclick={() => (picked = {})}>{$t("telegram.browser.clear")}</button>{/if}
      </div>

      {#if showFilters}
        <form class="s-filters" onsubmit={(e) => { e.preventDefault(); void search(); }}>
          <label class="field">
            <span class="field-label">{$t("telegram.chat_dl.from_date")}</span>
            <input class="input" type="date" bind:value={sFrom} />
          </label>
          <label class="field">
            <span class="field-label">{$t("telegram.chat_dl.to_date")}</span>
            <input class="input" type="date" bind:value={sTo} />
          </label>
          <label class="field">
            <span class="field-label">{$t("telegram.filters.min_size")}</span>
            <input class="input" inputmode="decimal" bind:value={sMinMb} placeholder="MB" />
          </label>
          <label class="field">
            <span class="field-label">{$t("telegram.filters.max_size")}</span>
            <input class="input" inputmode="decimal" bind:value={sMaxMb} placeholder="MB" />
          </label>
          <div class="s-f-actions">
            <button type="submit" class="btn btn-primary btn-sm"><TgIcon name="search" /> {$t("telegram.search.apply")}</button>
            {#if filterCount}<button type="button" class="btn btn-ghost btn-sm" onclick={clearFilters}>{$t("telegram.search.clear")}</button>{/if}
          </div>
          <p class="s-f-hint">{$t("telegram.search.hint")}</p>
        </form>
      {/if}

      {#if error}<p class="field-error">{error}</p>{/if}

      <div class="grid">
        {#each items as item (item.message_id)}
          {@const st = fileStatus(item.message_id)}
          {@const canPlay = isPlayable(item)}
          {@const rs = canPlay ? resumeOf(selected, item.message_id) : undefined}
          <div class="tile" class:picked={picked[item.message_id]}>
            <button class="tile-main" onclick={(e) => activate(item, e)} title={item.caption || item.file_name} aria-label={canPlay && !pickedItems.length ? $t("telegram.browser.play", { name: item.file_name }) : item.file_name}>
              <span class="thumb" use:thumb={{ chat: selected, item }}>
                <TgIcon name={typeIcon(item.media_type)} size={22} />
                {#if canPlay}<span class="play-badge"><TgIcon name="play" size={13} /></span>{/if}
                {#if rs && rs.duration_sec > 0}
                  <span class="resume-bar" class:done={rs.finished}><span style:width="{rs.finished ? 100 : (rs.position_sec / rs.duration_sec) * 100}%"></span></span>
                {/if}
                {#if st && st.status !== "queued"}
                  <span class="tile-prog" class:done={st.status === "done" || st.status === "duplicate" || st.status.startsWith("skipped") || st.status === "resumed_skip"} class:err={st.status === "error"} style:width="{st.status === 'error' ? 100 : st.percent}%"></span>
                {/if}
              </span>
              <span class="t-name">{item.file_name}</span>
              <span class="t-meta">{formatBytes(item.file_size)} · {formatDate(item.date)}</span>
            </button>
            {#if canPlay}
              <button class="tr-chip" onclick={() => (transcribing = item)} title={$t("telegram.transcript.button")} aria-label={$t("telegram.transcript.for", { name: item.file_name })}>
                <TgIcon name="captions" size={12} />
              </button>
            {/if}
            <button class="check" onclick={() => toggle(item.message_id)} aria-pressed={!!picked[item.message_id]} aria-label={$t("telegram.browser.select_item", { name: item.file_name })}>
              {#if picked[item.message_id]}<TgIcon name="check" size={11} />{/if}
            </button>
          </div>
        {/each}
      </div>

      {#if loading}
        <p class="dim center"><span class="spinner"></span> {$t("telegram.common.loading")}</p>
      {:else if !items.length && !error}
        <p class="dim center">{searching ? $t("telegram.browser.no_results") : $t("telegram.browser.no_media")}</p>
      {:else if hasMore && !searching}
        <button class="btn btn-secondary btn-sm more" onclick={() => void loadMedia(false)}>{$t("telegram.browser.more_media")}</button>
      {/if}

      <footer class="m-foot">
        <span class="sel-info">
          {pickedItems.length ? $t("telegram.browser.selected", { count: pickedItems.length, size: formatBytes(pickedBytes) }) : $t("telegram.browser.none_selected")}
        </span>
        <OutputDir bind:value={outputDir} />
        <button class="btn btn-ghost btn-sm" onclick={addFolderToWatch} disabled={addingWatch || !outputDir} title={$t("telegram.watch_lib.add_this_folder")} aria-label={$t("telegram.watch_lib.add_this_folder")}>
          {#if addingWatch}<span class="spinner"></span>{:else}<TgIcon name="tv" />{/if}
        </button>
        {#if pickedItems.length}
          <button class="btn btn-secondary" onclick={() => (dialog = "forward")}><TgIcon name="forward" /> {$t("telegram.browser.forward")}</button>
        {/if}
        <button class="btn btn-primary" onclick={download} disabled={!pickedItems.length}>
          <TgIcon name="download" /> {$t("telegram.browser.download")}
        </button>
      </footer>
    {/if}

    <ScanList />

    {#if activeBatches.length}
      <div class="batches">
        {#each activeBatches as b (b.id)}
          {@const c = batchCounts(b)}
          <div class="batch">
            <span class="b-title">{b.chatTitle}</span>
            <span class="b-count">{$t("telegram.browser.batch_progress", { done: c.done, total: b.total })}{c.failed ? ` · ${$t("telegram.browser.batch_failed", { count: c.failed })}` : ""}</span>
            <div class="progress"><div class="progress-fill" style:width="{c.percent}%"></div></div>
            <button class="btn btn-ghost btn-sm" onclick={() => cancel(b.id)}>{$t("telegram.common.cancel")}</button>
          </div>
        {/each}
      </div>
    {/if}
  </section>
</div>
</div>

{#if player}
  <Player chat={player.chat} queue={player.queue} start={player.start} startAt={player.startAt} transcript={player.transcript} onclose={closePlayer} onDownload={downloadOne} />
{/if}

{#if selected && transcribing}
  {@const it = transcribing}
  <TgModal title={$t("telegram.transcript.title")} subtitle={it.caption || it.file_name} width={620} onclose={() => (transcribing = null)}>
    <div class="tr-modal">
      <TranscriptPanel chat={selected} messageId={it.message_id} auto onseek={(sec) => playAt(it, sec)} />
    </div>
  </TgModal>
{/if}

{#if selected && dialog === "watch"}
  <WatchRuleEditor presetChat={selected} onclose={() => (dialog = null)} />
{:else if selected && dialog === "chat_dl"}
  <ChatDownloadDialog chat={selected} bind:outputDir onclose={() => (dialog = null)} />
{:else if selected && dialog === "upload"}
  <TgModal title={$t("telegram.upload.title_to", { chat: selected.title })} subtitle={$t("telegram.upload.hint")} width={600} onclose={() => (dialog = null)}>
    <UploadPanel chat={selected} fixedChat />
  </TgModal>
{:else if selected && dialog === "forward"}
  <ForwardDialog from={selected} messageIds={pickedItems.map((i) => i.message_id)} onclose={() => (dialog = null)} ondone={() => (picked = {})} />
{/if}

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-height: 0;
    height: 100%;
  }

  .wrap > .browser {
    flex: 1;
  }

  .m-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .tile {
    position: relative;
    min-width: 0;
  }

  .tr-chip {
    position: absolute;
    top: 6px;
    left: 6px;
    z-index: 1;
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    border-radius: var(--radius-full);
    display: grid;
    place-items: center;
    color: #fff;
    background: rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(6px);
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--duration-fast) var(--ease-out);
  }

  .tile:hover .tr-chip,
  .tr-chip:focus-visible {
    opacity: 1;
  }

  .tr-chip:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  .tr-modal {
    display: flex;
    flex-direction: column;
    max-height: 60vh;
    min-height: 120px;
  }

  .s-filters {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: var(--space-2) var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--fill-1);
  }

  .s-filters .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .s-f-actions {
    grid-column: 1 / -1;
    display: flex;
    gap: var(--space-2);
  }

  .s-f-hint {
    grid-column: 1 / -1;
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .m-tools .on {
    color: var(--accent);
  }

  @media (max-width: 760px) {
    .s-filters {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }

  .tile-main {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
    padding: 0;
    border: none;
    background: none;
    font: inherit;
    text-align: left;
    color: var(--text);
    cursor: pointer;
    min-width: 0;
  }

  .tile-main:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
    border-radius: var(--radius-md);
  }

  .play-badge {
    position: absolute;
    left: 6px;
    bottom: 8px;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-full);
    display: grid;
    place-items: center;
    color: #fff;
    background: rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(6px);
    transition: transform var(--duration-fast) var(--ease-out);
  }

  .tile-main:hover .play-badge {
    transform: scale(1.1);
  }

  .resume-bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 3px;
    background: rgba(255, 255, 255, 0.3);
  }

  .resume-bar span {
    display: block;
    height: 100%;
    background: var(--danger);
  }

  .resume-bar.done span {
    background: rgba(255, 255, 255, 0.7);
  }

  .browser {
    display: grid;
    grid-template-columns: 260px minmax(0, 1fr);
    gap: var(--space-4);
    min-height: 0;
    height: 100%;
  }

  .chats {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
  }

  .chat-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .chat {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px var(--space-2);
    border: none;
    border-radius: var(--radius-md);
    background: none;
    font: inherit;
    text-align: left;
    color: var(--text);
    cursor: pointer;
  }

  .chat:hover {
    background: var(--fill-1);
  }

  .chat.sel {
    background: var(--accent-soft);
  }

  .avatar {
    width: 30px;
    height: 30px;
    border-radius: var(--radius-full);
    display: grid;
    place-items: center;
    flex-shrink: 0;
    font-size: var(--text-sm);
    font-weight: 650;
    color: #fff;
    background: linear-gradient(160deg, #5aa9ff, #1e6fe8);
  }

  .avatar[data-type="channel"] {
    background: linear-gradient(160deg, #ffb340, #f28500);
  }

  .avatar[data-type="group"],
  .avatar[data-type="supergroup"] {
    background: linear-gradient(160deg, #67d27e, #2f9e52);
  }

  .chat-body {
    min-width: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
  }

  .chat-title,
  .chat-sub {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .chat-title {
    font-size: var(--text-base);
    font-weight: 550;
  }

  .chat-sub {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .chat-skel {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    padding: 6px var(--space-2);
  }

  .chat-skel .circle {
    width: 30px;
    height: 30px;
  }

  .chat-skel span:last-child {
    flex: 1;
    height: 12px;
  }

  .media {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
  }

  .m-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .m-head h3 {
    margin: 0;
    min-width: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .m-tools {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }

  .m-search {
    flex: 1;
  }

  .m-search .input {
    width: 100%;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(132px, 1fr));
    gap: var(--space-3);
  }

  .thumb {
    position: relative;
    aspect-ratio: 1;
    border-radius: var(--radius-md);
    background: var(--fill-1) center / cover no-repeat;
    display: grid;
    place-items: center;
    color: var(--text-dim);
    overflow: hidden;
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .thumb:global(.has-thumb) > :global(.tg-icon) {
    opacity: 0;
  }

  .tile.picked .thumb {
    box-shadow: inset 0 0 0 2px var(--accent);
  }

  .check {
    position: absolute;
    top: 6px;
    right: 6px;
    z-index: 1;
    padding: 0;
    border: none;
    cursor: pointer;
    opacity: 0.85;
    width: 20px;
    height: 20px;
    border-radius: var(--radius-full);
    display: grid;
    place-items: center;
    background: color-mix(in srgb, #000 35%, transparent);
    box-shadow: inset 0 0 0 1.5px #fff;
    color: var(--on-accent);
  }

  .tile:hover .check,
  .check:focus-visible {
    opacity: 1;
  }

  .check:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  .tile.picked .check {
    background: var(--accent);
    box-shadow: none;
  }

  .tile-prog {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 3px;
    background: var(--accent);
    transition: width var(--duration-base) var(--ease-out);
  }

  .tile-prog.done {
    background: var(--success);
  }

  .tile-prog.err {
    background: var(--danger);
  }

  .t-name,
  .t-meta {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .t-name {
    font-size: var(--text-sm);
  }

  .t-meta {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .m-foot {
    position: sticky;
    bottom: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    margin-top: auto;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .sel-info {
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .batches {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .batch {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 140px auto;
    align-items: center;
    gap: var(--space-3);
    font-size: var(--text-sm);
  }

  .b-title {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    color: var(--text);
  }

  .b-count {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .more {
    align-self: center;
  }

  .dim {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .pad {
    padding: var(--space-2);
  }

  .center {
    text-align: center;
  }

  @media (max-width: 760px) {
    .browser {
      grid-template-columns: 1fr;
    }
    .chat-list {
      max-height: 240px;
    }
  }
</style>
