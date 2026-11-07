<script lang="ts">
  import { tick, untrack } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "./api";
  import type { WaChat, WaMessage } from "./api";
  import { onDownloadProgress, wa } from "./session.svelte";
  import WaIcon from "./WaIcon.svelte";
  import MediaDialog from "./MediaDialog.svelte";
  import ExportDialog from "./ExportDialog.svelte";

  let {
    chat,
    online,
    outputDir = $bindable(""),
  }: { chat: WaChat; online: boolean; outputDir: string } = $props();

  const PAGE = 80;

  let items = $state<WaMessage[]>([]);
  let loading = $state(true);
  let loadingOlder = $state(false);
  let reachedStart = $state(false);
  let error = $state<string | null>(null);
  let draft = $state("");
  let sending = $state(false);
  let sendError = $state<string | null>(null);
  let olderAsked = $state(false);
  let olderError = $state<string | null>(null);
  let busy = $state<Record<string, "loading" | string>>({});
  let showMedia = $state(false);
  let showExport = $state(false);
  let scroller: HTMLDivElement | undefined = $state();
  let currentJid = "";

  function errorLabel(raw: string): string {
    const k = api.knownError(raw);
    return k ? $t(`whatsapp.errors.${k}`) : raw;
  }

  async function scrollToEnd() {
    await tick();
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }

  async function loadLatest(jid: string, keepScroll = false) {
    try {
      const rows = await api.messages(jid, null, PAGE);
      if (jid !== currentJid) return;
      const nearBottom = !scroller || scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 120;
      if (keepScroll && items.length > PAGE) {
        const known = new Set(items.map((m) => m.id));
        const fresh = rows.filter((m) => !known.has(m.id));
        items = [...items, ...fresh];
      } else {
        items = rows;
        reachedStart = rows.length < PAGE;
      }
      error = null;
      if (!keepScroll || nearBottom) await scrollToEnd();
    } catch (e) {
      if (jid === currentJid) error = api.errText(e);
    } finally {
      if (jid === currentJid) loading = false;
    }
  }

  async function loadOlder() {
    if (loadingOlder || reachedStart || items.length === 0) return;
    loadingOlder = true;
    const jid = currentJid;
    const prevHeight = scroller?.scrollHeight ?? 0;
    try {
      const rows = await api.messages(jid, items[0].ts, PAGE);
      if (jid !== currentJid) return;
      const known = new Set(items.map((m) => m.id));
      items = [...rows.filter((m) => !known.has(m.id)), ...items];
      reachedStart = rows.length < PAGE;
      await tick();
      if (scroller) scroller.scrollTop = scroller.scrollHeight - prevHeight;
    } catch (e) {
      error = api.errText(e);
    } finally {
      loadingOlder = false;
    }
  }

  async function askPhone() {
    olderError = null;
    try {
      await api.requestOlder(chat.jid);
      olderAsked = true;
    } catch (e) {
      olderError = api.errText(e);
    }
  }

  $effect(() => {
    const jid = chat.jid;
    if (jid === currentJid) return;
    currentJid = jid;
    items = [];
    loading = true;
    reachedStart = false;
    olderAsked = false;
    olderError = null;
    sendError = null;
    busy = {};
    untrack(() => {
      void loadLatest(jid);
      readOnPhone(jid);
    });
  });

  function readOnPhone(jid: string) {
    if (!online || document.visibilityState !== "visible") return;
    void api.markRead(jid).catch(() => {});
  }

  $effect(() => {
    void wa.messageVersion;
    void wa.chatsVersion;
    untrack(() => {
      if (currentJid && !loading) void loadLatest(currentJid, true);
      if (currentJid && wa.lastMessageChat === currentJid) readOnPhone(currentJid);
    });
  });

  $effect(() =>
    onDownloadProgress((p) => {
      if (p.job_id !== null || p.chat !== currentJid) return;
      if (p.stage === "start") busy = { ...busy, [p.message_id]: "loading" };
      if (p.stage === "retrying") busy = { ...busy, [p.message_id]: "retrying" };
      if (p.stage === "error") busy = { ...busy, [p.message_id]: p.error ?? "error" };
      if (p.stage === "done") {
        const { [p.message_id]: _, ...rest } = busy;
        busy = rest;
        items = items.map((m) => (m.id === p.message_id ? { ...m, saved_path: p.path ?? m.saved_path } : m));
      }
    }),
  );

  async function send(event?: SubmitEvent) {
    event?.preventDefault();
    const text = draft.trim();
    if (!text || sending) return;
    sending = true;
    sendError = null;
    try {
      const row = await api.sendText(chat.jid, text);
      draft = "";
      if (!items.some((m) => m.id === row.id)) items = [...items, row];
      await scrollToEnd();
    } catch (e) {
      sendError = api.errText(e);
    } finally {
      sending = false;
    }
  }

  function composerKey(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void send();
    }
  }

  async function download(m: WaMessage) {
    busy = { ...busy, [m.id]: "loading" };
    try {
      const saved = await api.downloadMedia(chat.jid, m.id, outputDir || null);
      const { [m.id]: _, ...rest } = busy;
      busy = rest;
      items = items.map((x) => (x.id === m.id ? { ...x, saved_path: saved.path } : x));
    } catch (e) {
      busy = { ...busy, [m.id]: api.errText(e) };
    }
  }

  function onScroll() {
    if (scroller && scroller.scrollTop < 60) void loadOlder();
  }

  function time(ts: number): string {
    return new Date(ts * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  function day(ts: number): string {
    return new Date(ts * 1000).toLocaleDateString([], { weekday: "long", day: "numeric", month: "long", year: "numeric" });
  }

  function iconFor(kind: string): string {
    return { image: "photo", video: "video", audio: "music", voice: "mic", document: "file", sticker: "sticker", location: "pin", contact: "contact", poll: "poll" }[kind] ?? "info";
  }

  function mediaMeta(m: WaMessage): string {
    const parts: string[] = [];
    if (m.file_name) parts.push(m.file_name);
    if (m.duration) parts.push(api.formatDuration(m.duration));
    if (m.size) parts.push(api.formatBytes(m.size));
    return parts.join(" · ");
  }
</script>

<section class="view">
  <header class="view-head">
    <div class="view-title">
      <h2>{chat.name}</h2>
      <p>{chat.is_group ? $t("whatsapp.chat.group") : chat.jid.split("@")[0]} · {$t("whatsapp.chat.stored", { count: chat.message_count })}</p>
    </div>
    <div class="view-actions">
      <button class="btn btn-secondary btn-sm" onclick={() => (showMedia = true)} disabled={chat.media_count === 0} title={chat.media_count === 0 ? $t("whatsapp.media.none") : undefined}>
        <WaIcon name="download" /> {$t("whatsapp.chat.download_media")}
      </button>
      <button class="btn btn-secondary btn-sm" onclick={() => (showExport = true)} disabled={chat.message_count === 0}>
        <WaIcon name="share" /> {$t("whatsapp.chat.export")}
      </button>
    </div>
  </header>

  <div class="msgs" bind:this={scroller} onscroll={onScroll}>
    {#if loading}
      <div class="msgs-state"><span class="spinner"></span></div>
    {:else if error}
      <div class="msgs-state">
        <div class="notice notice-warning"><div class="notice-body"><div class="notice-text">{error}</div></div></div>
        <button class="btn btn-ghost btn-sm" onclick={() => void loadLatest(chat.jid)}>{$t("whatsapp.retry")}</button>
      </div>
    {:else}
      <div class="older">
        {#if loadingOlder}
          <span class="spinner"></span>
        {:else if reachedStart}
          {#if olderAsked}
            <span class="older-hint">{$t("whatsapp.chat.older_requested")}</span>
          {:else}
            <button class="btn btn-ghost btn-sm" onclick={askPhone} disabled={!online || items.length === 0}>
              <WaIcon name="history" /> {$t("whatsapp.chat.ask_older")}
            </button>
            {#if olderError}<span class="older-hint err">{errorLabel(olderError)}</span>{/if}
          {/if}
        {:else}
          <button class="btn btn-ghost btn-sm" onclick={() => void loadOlder()}>{$t("whatsapp.chat.show_older")}</button>
        {/if}
      </div>

      {#if items.length === 0}
        <div class="msgs-state">
          <div class="empty-state-title">{$t("whatsapp.chat.empty_title")}</div>
          <div class="empty-state-hint">{$t("whatsapp.chat.empty_hint")}</div>
        </div>
      {/if}

      {#each items as m, i (m.id)}
        {#if i === 0 || day(items[i - 1].ts) !== day(m.ts)}
          <div class="day"><span>{day(m.ts)}</span></div>
        {/if}
        <div class="msg" class:me={m.from_me}>
          {#if !m.from_me && chat.is_group}
            <div class="msg-who">{m.sender_name}</div>
          {/if}
          {#if m.deleted}
            <div class="deleted"><WaIcon name="x" size={12} /> {m.from_me ? $t("whatsapp.chat.deleted_by_you") : $t("whatsapp.chat.deleted")}</div>
          {:else if api.isMedia(m.kind)}
            {@const state = busy[m.id]}
            <div class="media">
              {#if m.thumb}
                <img class="thumb" src={m.thumb} alt="" />
              {:else}
                <span class="media-icon"><WaIcon name={iconFor(m.kind)} size={20} /></span>
              {/if}
              <div class="media-info">
                <span class="media-kind">{$t(`whatsapp.kind.${m.kind}`)}</span>
                {#if mediaMeta(m)}<span class="media-meta">{mediaMeta(m)}</span>{/if}
                {#if state === "retrying"}
                  <span class="media-wait">{$t("whatsapp.media.retrying")}</span>
                {:else if state && state !== "loading"}
                  <span class="media-err">{errorLabel(state)}</span>
                {/if}
              </div>
              <div class="media-act">
                {#if m.saved_path}
                  <button class="btn btn-ghost btn-sm" onclick={() => void api.openPath(m.saved_path ?? "").catch(() => {})}>{$t("whatsapp.media.open")}</button>
                  <button class="btn btn-ghost btn-sm btn-icon" aria-label={$t("whatsapp.media.reveal")} title={$t("whatsapp.media.reveal")} onclick={() => void api.reveal(m.saved_path ?? "").catch(() => {})}><WaIcon name="folder" /></button>
                {:else if state === "loading" || state === "retrying"}
                  <span class="spinner"></span>
                {:else}
                  <button class="btn btn-ghost btn-sm btn-icon" aria-label={$t("whatsapp.media.download")} title={online ? $t("whatsapp.media.download") : $t("whatsapp.errors.offline")} disabled={!online} onclick={() => void download(m)}><WaIcon name="download" /></button>
                {/if}
              </div>
            </div>
          {:else if m.kind !== "text"}
            <div class="special"><WaIcon name={iconFor(m.kind)} /> <span>{$t(`whatsapp.kind.${m.kind}`)}</span></div>
          {/if}
          {#if m.text && !m.deleted}<div class="msg-text">{m.text}</div>{/if}
          <div class="msg-time">{#if m.edited && !m.deleted}<span class="edited">{$t("whatsapp.chat.edited")}</span>{/if}{time(m.ts)}</div>
        </div>
      {/each}
    {/if}
  </div>

  <form class="composer" onsubmit={send}>
    <textarea
      class="input composer-input"
      rows="1"
      bind:value={draft}
      onkeydown={composerKey}
      placeholder={online ? $t("whatsapp.chat.composer") : $t("whatsapp.chat.composer_offline")}
      aria-label={$t("whatsapp.chat.composer")}
      disabled={!online || sending}
    ></textarea>
    <button class="btn btn-primary btn-icon" type="submit" aria-label={$t("whatsapp.chat.send")} disabled={!online || sending || !draft.trim()}>
      {#if sending}<span class="spinner"></span>{:else}<WaIcon name="send" size={16} />{/if}
    </button>
  </form>
  {#if sendError}
    <div class="send-err" role="alert">{errorLabel(sendError)}</div>
  {/if}
</section>

{#if showMedia}
  <MediaDialog {chat} {online} bind:outputDir onclose={() => (showMedia = false)} />
{/if}
{#if showExport}
  <ExportDialog {chat} onclose={() => (showExport = false)} />
{/if}

<style>
  .view {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }

  .view-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: var(--hairline) solid var(--separator);
  }

  .view-title {
    flex: 1;
    min-width: 0;
  }

  .view-title h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .view-title p {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .view-actions {
    display: flex;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .msgs {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-3) var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .msgs-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-7) var(--space-4);
    text-align: center;
  }

  .older {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-1);
    padding-bottom: var(--space-2);
  }

  .older-hint {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .older-hint.err {
    color: var(--warning);
  }

  .day {
    display: flex;
    justify-content: center;
    margin: var(--space-2) 0;
  }

  .day span {
    padding: 2px var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .msg {
    align-self: flex-start;
    max-width: min(560px, 78%);
    padding: var(--space-2) var(--space-3) 4px;
    border-radius: var(--radius-lg);
    background: var(--fill-1);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .msg.me {
    align-self: flex-end;
    background: color-mix(in srgb, var(--success) 18%, var(--fill-1));
  }

  .msg-who {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--accent-text, var(--accent));
  }

  .msg-text {
    font-size: var(--text-base);
    line-height: 1.45;
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .msg-time {
    align-self: flex-end;
    font-size: var(--text-caption);
    color: var(--text-muted);
  }

  .media {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 220px;
  }

  .thumb {
    width: 56px;
    height: 56px;
    object-fit: cover;
    border-radius: var(--radius-md);
    flex-shrink: 0;
  }

  .media-icon {
    width: 40px;
    height: 40px;
    border-radius: var(--radius-md);
    background: var(--fill-2);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-dim);
    flex-shrink: 0;
  }

  .media-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .media-kind {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }

  .media-meta {
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .media-wait {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .deleted {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-base);
    font-style: italic;
    color: var(--text-muted);
  }

  .edited {
    margin-right: var(--space-1);
  }

  .media-err {
    font-size: var(--text-xs);
    color: var(--warning);
  }

  .media-act {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }

  .special {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .composer {
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-top: var(--hairline) solid var(--separator);
  }

  .composer-input {
    flex: 1;
    min-height: var(--control-h-lg);
    max-height: 140px;
    resize: none;
    field-sizing: content;
    padding-block: 8px;
    font: inherit;
  }

  .send-err {
    padding: 0 var(--space-4) var(--space-3);
    font-size: var(--text-xs);
    color: var(--warning);
  }
</style>
