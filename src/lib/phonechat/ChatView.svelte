<script lang="ts">
  import { tick, untrack } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "./api";
  import type { WaChat, WaMessage, WaScheduled } from "./api";
  import { onDownloadProgress, wa } from "./session.svelte";
  import WaIcon from "./WaIcon.svelte";
  import Dots from "./Dots.svelte";
  import MediaDialog from "./MediaDialog.svelte";
  import ExportDialog from "./ExportDialog.svelte";

  let {
    chat,
    online,
    outputDir = $bindable(""),
  }: { chat: WaChat; online: boolean; outputDir: string } = $props();

  const PAGE = 80;
  const QUICK_REACTIONS = ["👍", "❤️", "😂", "😮", "😢", "🙏"];

  type Bubble = WaMessage & { pending?: boolean; failed?: string; replyId?: string | null };

  let items = $state<Bubble[]>([]);
  let pendingSeq = 0;
  const isDraft = (m: Bubble) => m.id.startsWith("pending-");
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
  let scheduleOpen = $state(false);
  let scheduleAt = $state("");
  let pending = $state<WaScheduled[]>([]);
  let replyTo = $state<WaMessage | null>(null);
  let pickerFor = $state<string | null>(null);
  let composer: HTMLTextAreaElement | undefined = $state();
  let scroller: HTMLDivElement | undefined = $state();
  let currentJid = "";

  function errorLabel(raw: string): string {
    const k = api.knownError(raw);
    return k ? $t(`phonechat.errors.${k}`) : raw;
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
        const latest = new Map(rows.map((m) => [m.id, m]));
        const known = new Set(items.map((m) => m.id));
        const fresh = rows.filter((m) => !known.has(m.id));
        items = [...items.map((m) => latest.get(m.id) ?? m), ...fresh];
      } else {
        items = [...rows, ...items.filter(isDraft)];
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
    replyTo = null;
    pickerFor = null;
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

  async function loadPending(jid: string) {
    try {
      const rows = await api.scheduled(false);
      if (jid === currentJid) pending = rows.filter((r) => r.chat === jid);
    } catch {
      pending = [];
    }
  }

  $effect(() => {
    void wa.scheduledVersion;
    const jid = chat.jid;
    untrack(() => void loadPending(jid));
  });

  function localInput(d: Date): string {
    const off = d.getTimezoneOffset() * 60000;
    return new Date(d.getTime() - off).toISOString().slice(0, 16);
  }

  function toggleSchedule() {
    scheduleOpen = !scheduleOpen;
    if (scheduleOpen && !scheduleAt) scheduleAt = localInput(new Date(Date.now() + 15 * 60000));
    if (!scheduleOpen) scheduleAt = "";
  }

  async function cancelPending(id: number) {
    try {
      await api.cancelScheduled(id);
      pending = pending.filter((p) => p.id !== id);
    } catch (e) {
      sendError = api.errText(e);
    }
  }

  function when(ts: number): string {
    return new Date(ts * 1000).toLocaleString([], { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  }

  async function send(event?: SubmitEvent) {
    event?.preventDefault();
    const text = draft.trim();
    if (!text || sending) return;
    sending = true;
    sendError = null;
    try {
      if (scheduleOpen && scheduleAt) {
        const at = Math.floor(new Date(scheduleAt).getTime() / 1000);
        const row = await api.schedule(chat.jid, text, at, replyTo?.id);
        draft = "";
        replyTo = null;
        scheduleOpen = false;
        scheduleAt = "";
        pending = [...pending, row].sort((a, b) => a.at - b.at);
        return;
      }
      sending = false;
      sendNow(text);
    } catch (e) {
      sendError = api.errText(e);
    } finally {
      sending = false;
    }
  }

  function sendNow(text: string) {
    sendError = null;
    const bubble: Bubble = {
      id: `pending-${++pendingSeq}`,
      chat: chat.jid,
      from_me: true,
      sender: null,
      sender_name: null,
      ts: Math.floor(Date.now() / 1000),
      kind: "text",
      text,
      mime: null,
      size: null,
      file_name: null,
      duration: null,
      thumb: null,
      saved_path: null,
      edited: false,
      deleted: false,
      seq: 0,
      quote_id: replyTo?.id ?? null,
      reactions: [],
      pending: true,
      replyId: replyTo?.id ?? null,
    };
    items = [...items, bubble];
    draft = "";
    replyTo = null;
    void scrollToEnd();
    void deliver(bubble);
  }

  async function deliver(bubble: Bubble) {
    const jid = bubble.chat;
    try {
      const row = await api.sendText(jid, bubble.text ?? "", bubble.replyId);
      if (jid !== currentJid) return;
      const rest = items.filter((m) => m.id !== bubble.id);
      items = rest.some((m) => m.id === row.id) ? rest : [...rest, row];
    } catch (e) {
      if (jid !== currentJid) return;
      items = items.map((m) => (m.id === bubble.id ? { ...m, pending: false, failed: api.errText(e) } : m));
    }
  }

  function retry(bubble: Bubble) {
    items = items.map((m) => (m.id === bubble.id ? { ...m, pending: true, failed: undefined } : m));
    void deliver(bubble);
  }

  function discard(bubble: Bubble) {
    items = items.filter((m) => m.id !== bubble.id);
  }

  let attaching = $state(false);
  let newTag = $state("");

  async function attach() {
    sendError = null;
    try {
      const path = await open({ multiple: false, directory: false });
      if (typeof path !== "string") return;
      attaching = true;
      const jid = chat.jid;
      const caption = draft.trim();
      const row = await api.sendFile(jid, path, caption);
      if (jid !== currentJid) return;
      if (caption && draft.trim() === caption) draft = "";
      if (!items.some((m) => m.id === row.id)) items = [...items, row];
      await scrollToEnd();
    } catch (e) {
      sendError = api.errText(e);
    } finally {
      attaching = false;
    }
  }

  let suggestion = $state<string | null>(null);

  async function loadSuggestion(jid: string) {
    try {
      const text = await api.draft(jid);
      if (jid === currentJid) suggestion = text;
    } catch {
      suggestion = null;
    }
  }

  $effect(() => {
    void wa.draftVersion;
    const jid = chat.jid;
    untrack(() => void loadSuggestion(jid));
  });

  function dropSuggestion(use: boolean) {
    if (use && suggestion) {
      draft = suggestion;
      composer?.focus();
    }
    suggestion = null;
    void api.draftClear(chat.jid).catch(() => {});
  }

  async function setField(key: string, value: string) {
    sendError = null;
    try {
      await api.field(chat.jid, key, value);
      if (value) newTag = "";
    } catch (e) {
      sendError = api.errText(e);
    }
  }

  function addDetail() {
    const typed = newTag.trim();
    if (!typed) return;
    const cut = typed.indexOf("=");
    if (cut > 0) void setField(typed.slice(0, cut), typed.slice(cut + 1));
    else void setTag(typed, true);
  }

  async function setTag(name: string, on: boolean) {
    sendError = null;
    try {
      await api.tag(chat.jid, name, on);
      if (on) newTag = "";
    } catch (e) {
      sendError = api.errText(e);
    }
  }

  function startReply(m: WaMessage) {
    replyTo = m;
    pickerFor = null;
    composer?.focus();
  }

  async function react(m: WaMessage, emoji: string) {
    pickerFor = null;
    sendError = null;
    const mine = m.reactions.find((r) => r.mine);
    try {
      await api.react(chat.jid, m.id, mine?.emoji === emoji ? "" : emoji);
    } catch (e) {
      sendError = api.errText(e);
    }
  }

  function who(m: WaMessage): string {
    return m.from_me ? ($t("phonechat.you") as string) : (m.sender_name ?? chat.name);
  }

  function snippet(m: WaMessage): string {
    return m.text || ($t(`phonechat.kind.${m.kind}`) as string);
  }

  function composerKey(e: KeyboardEvent) {
    if (e.key === "Escape" && replyTo) {
      e.preventDefault();
      replyTo = null;
      return;
    }
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
      <p>{chat.is_group ? $t("phonechat.chat.group") : chat.jid.split("@")[0]} · {$t("phonechat.chat.stored", { count: chat.message_count })}</p>
      <form class="tags" onsubmit={(e) => (e.preventDefault(), addDetail())}>
        {#each chat.tags as name (name)}
          <span class="tag-chip">{name}<button type="button" aria-label={$t("phonechat.chat.tag_remove", { tag: name })} title={$t("phonechat.chat.tag_remove", { tag: name })} onclick={() => void setTag(name, false)}><WaIcon name="x" size={10} /></button></span>
        {/each}
        {#each chat.fields as [key, value] (key)}
          <span class="tag-chip field"><span class="field-key">{key}</span>{value}<button type="button" aria-label={$t("phonechat.chat.field_remove", { key })} title={$t("phonechat.chat.field_remove", { key })} onclick={() => void setField(key, "")}><WaIcon name="x" size={10} /></button></span>
        {/each}
        <span class="tag-new" title={$t("phonechat.chat.field_hint")}><WaIcon name="tag" size={11} /><input bind:value={newTag} maxlength="530" placeholder={$t("phonechat.chat.tag_add")} aria-label={$t("phonechat.chat.tag_add")} /></span>
      </form>
    </div>
    <div class="view-actions">
      <button class="btn btn-secondary btn-sm" onclick={() => (showMedia = true)} disabled={chat.media_count === 0} title={chat.media_count === 0 ? $t("phonechat.media.none") : undefined}>
        <WaIcon name="download" /> {$t("phonechat.chat.download_media")}
      </button>
      <button class="btn btn-secondary btn-sm" onclick={() => (showExport = true)} disabled={chat.message_count === 0}>
        <WaIcon name="share" /> {$t("phonechat.chat.export")}
      </button>
    </div>
  </header>

  <div class="msgs" bind:this={scroller} onscroll={onScroll}>
    {#if loading}
      <div class="msgs-state"><span class="spinner"></span></div>
    {:else if error}
      <div class="msgs-state">
        <div class="notice notice-warning"><div class="notice-body"><div class="notice-text">{error}</div></div></div>
        <button class="btn btn-ghost btn-sm" onclick={() => void loadLatest(chat.jid)}>{$t("phonechat.retry")}</button>
      </div>
    {:else}
      <div class="older">
        {#if loadingOlder}
          <span class="spinner"></span>
        {:else if reachedStart}
          {#if olderAsked}
            <span class="older-hint">{$t("phonechat.chat.older_requested")}</span>
          {:else}
            <button class="btn btn-ghost btn-sm" onclick={askPhone} disabled={!online || items.length === 0}>
              <WaIcon name="history" /> {$t("phonechat.chat.ask_older")}
            </button>
            {#if olderError}<span class="older-hint err">{errorLabel(olderError)}</span>{/if}
          {/if}
        {:else}
          <button class="btn btn-ghost btn-sm" onclick={() => void loadOlder()}>{$t("phonechat.chat.show_older")}</button>
        {/if}
      </div>

      {#if items.length === 0}
        <div class="msgs-state">
          <div class="empty-state-title">{$t("phonechat.chat.empty_title")}</div>
          <div class="empty-state-hint">{$t("phonechat.chat.empty_hint")}</div>
        </div>
      {/if}

      {#each items as m, i (m.id)}
        {#if i === 0 || day(items[i - 1].ts) !== day(m.ts)}
          <div class="day"><span>{day(m.ts)}</span></div>
        {/if}
        <div class="msg" class:me={m.from_me} class:pending={m.pending} class:failed={!!m.failed}>
          {#if !m.from_me && chat.is_group}
            <div class="msg-who">{m.sender_name}</div>
          {/if}
          {#if m.quote_id && !m.deleted}
            {@const quoted = items.find((x) => x.id === m.quote_id)}
            <div class="quote">
              {#if quoted && !quoted.deleted}
                <span class="quote-who">{who(quoted)}</span>
                <span class="quote-text">{snippet(quoted)}</span>
              {:else}
                <span class="quote-text">{$t("phonechat.chat.quoted")}</span>
              {/if}
            </div>
          {/if}
          {#if m.deleted}
            <div class="deleted"><WaIcon name="x" size={12} /> {m.from_me ? $t("phonechat.chat.deleted_by_you") : $t("phonechat.chat.deleted")}</div>
          {:else if api.isMedia(m.kind)}
            {@const state = busy[m.id]}
            <div class="media">
              {#if m.thumb}
                <img class="thumb" src={m.thumb} alt="" />
              {:else}
                <span class="media-icon"><WaIcon name={iconFor(m.kind)} size={20} /></span>
              {/if}
              <div class="media-info">
                <span class="media-kind">{$t(`phonechat.kind.${m.kind}`)}</span>
                {#if mediaMeta(m)}<span class="media-meta">{mediaMeta(m)}</span>{/if}
                {#if state === "retrying"}
                  <span class="media-wait">{$t("phonechat.media.retrying")}</span>
                {:else if state && state !== "loading"}
                  <span class="media-err">{errorLabel(state)}</span>
                {/if}
              </div>
              <div class="media-act">
                {#if m.saved_path}
                  <button class="btn btn-ghost btn-sm" onclick={() => void api.openPath(m.saved_path ?? "").catch(() => {})}>{$t("phonechat.media.open")}</button>
                  <button class="btn btn-ghost btn-sm btn-icon" aria-label={$t("phonechat.media.reveal")} title={$t("phonechat.media.reveal")} onclick={() => void api.reveal(m.saved_path ?? "").catch(() => {})}><WaIcon name="folder" /></button>
                {:else if state === "loading" || state === "retrying"}
                  <span class="spinner"></span>
                {:else}
                  <button class="btn btn-ghost btn-sm btn-icon" aria-label={$t("phonechat.media.download")} title={online ? $t("phonechat.media.download") : $t("phonechat.errors.offline")} disabled={!online} onclick={() => void download(m)}><WaIcon name="download" /></button>
                {/if}
              </div>
            </div>
          {:else if m.kind !== "text"}
            <div class="special"><WaIcon name={iconFor(m.kind)} /> <span>{$t(`phonechat.kind.${m.kind}`)}</span></div>
          {/if}
          {#if m.text && !m.deleted}<div class="msg-text">{m.text}</div>{/if}
          {#if m.failed}
            <div class="msg-failed" role="alert">
              <span>{$t("phonechat.chat.send_failed")}: {errorLabel(m.failed)}</span>
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => retry(m)}>{$t("phonechat.chat.send_retry")}</button>
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => discard(m)}>{$t("phonechat.chat.send_discard")}</button>
            </div>
          {:else}
            <div class="msg-time">{#if m.edited && !m.deleted}<span class="edited">{$t("phonechat.chat.edited")}</span>{/if}{#if m.pending}<Dots label={$t("phonechat.chat.sending") as string} />{:else}{time(m.ts)}{/if}</div>
          {/if}
          {#if m.reactions.length}
            <div class="reactions">
              {#each m.reactions as r (r.emoji)}
                <button class="reaction" class:mine={r.mine} type="button" disabled={!online} title={r.mine ? $t("phonechat.chat.reaction_remove") : $t("phonechat.chat.react")} onclick={() => void react(m, r.emoji)}>
                  <span>{r.emoji}</span>{#if r.count > 1}<span class="reaction-count">{r.count}</span>{/if}
                </button>
              {/each}
            </div>
          {/if}
          {#if !m.deleted && !isDraft(m)}
            <div class="msg-tools" class:open={pickerFor === m.id}>
              {#if pickerFor === m.id}
                {#each QUICK_REACTIONS as emoji (emoji)}
                  <button class="tool emoji" type="button" aria-label={emoji} onclick={() => void react(m, emoji)}>{emoji}</button>
                {/each}
              {/if}
              <button class="tool" type="button" disabled={!online} aria-expanded={pickerFor === m.id} aria-label={$t("phonechat.chat.react")} title={$t("phonechat.chat.react")} onclick={() => (pickerFor = pickerFor === m.id ? null : m.id)}>
                <WaIcon name="smile" size={14} />
              </button>
              <button class="tool" type="button" aria-label={$t("phonechat.chat.reply")} title={$t("phonechat.chat.reply")} onclick={() => startReply(m)}>
                <WaIcon name="reply" size={14} />
              </button>
            </div>
          {/if}
        </div>
      {/each}
    {/if}
  </div>

  {#if pending.length}
    <ul class="pending" aria-label={$t("phonechat.chat.scheduled_title")}>
      {#each pending as p (p.id)}
        <li>
          <WaIcon name="clock" size={13} />
          <span class="pending-when">{when(p.at)}</span>
          <span class="pending-text">{p.text}</span>
          <button class="btn btn-ghost btn-icon btn-sm" type="button" aria-label={$t("phonechat.chat.scheduled_cancel")} title={$t("phonechat.chat.scheduled_cancel")} onclick={() => cancelPending(p.id)}>
            <WaIcon name="x" size={13} />
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  {#if scheduleOpen}
    <div class="schedule-row">
      <label for="wa-schedule-at">{$t("phonechat.chat.schedule_at")}</label>
      <input id="wa-schedule-at" class="input" type="datetime-local" bind:value={scheduleAt} min={localInput(new Date())} />
    </div>
  {/if}
  {#if suggestion}
    <div class="reply-bar suggestion">
      <WaIcon name="bolt" size={14} />
      <div class="reply-body">
        <span class="quote-who">{$t("phonechat.chat.draft_title")}</span>
        <span class="suggestion-text">{suggestion}</span>
      </div>
      <button class="btn btn-secondary btn-sm" type="button" onclick={() => dropSuggestion(true)}>{$t("phonechat.chat.draft_use")}</button>
      <button class="btn btn-ghost btn-icon btn-sm" type="button" aria-label={$t("phonechat.chat.draft_dismiss")} title={$t("phonechat.chat.draft_dismiss")} onclick={() => dropSuggestion(false)}>
        <WaIcon name="x" size={13} />
      </button>
    </div>
  {/if}
  {#if replyTo}
    <div class="reply-bar">
      <WaIcon name="reply" size={14} />
      <div class="reply-body">
        <span class="quote-who">{$t("phonechat.chat.replying_to", { name: who(replyTo) })}</span>
        <span class="quote-text">{snippet(replyTo)}</span>
      </div>
      <button class="btn btn-ghost btn-icon btn-sm" type="button" aria-label={$t("phonechat.chat.reply_cancel")} title={$t("phonechat.chat.reply_cancel")} onclick={() => (replyTo = null)}>
        <WaIcon name="x" size={13} />
      </button>
    </div>
  {/if}
  <form class="composer" onsubmit={send}>
    <button class="btn btn-ghost btn-icon" class:active={scheduleOpen} type="button" aria-pressed={scheduleOpen} aria-label={$t("phonechat.chat.schedule")} title={$t("phonechat.chat.schedule")} onclick={toggleSchedule} disabled={sending}>
      <WaIcon name="clock" size={16} />
    </button>
    <button class="btn btn-ghost btn-icon" type="button" aria-label={$t("phonechat.chat.attach")} title={$t("phonechat.chat.attach")} onclick={() => void attach()} disabled={!online || attaching || sending}>
      {#if attaching}<span class="spinner"></span>{:else}<WaIcon name="clip" size={16} />{/if}
    </button>
    <textarea
      class="input composer-input"
      rows="1"
      bind:value={draft}
      bind:this={composer}
      onkeydown={composerKey}
      placeholder={online ? $t("phonechat.chat.composer") : $t("phonechat.chat.composer_offline")}
      aria-label={$t("phonechat.chat.composer")}
      disabled={(!online && !scheduleOpen) || sending}
    ></textarea>
    <button class="btn btn-primary btn-icon" type="submit" aria-label={scheduleOpen ? $t("phonechat.chat.schedule") : $t("phonechat.chat.send")} disabled={(!online && !scheduleOpen) || sending || !draft.trim() || (scheduleOpen && !scheduleAt)}>
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

  .msg {
    position: relative;
  }

  .tags {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    margin-top: 4px;
  }

  .tag-chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 1px 4px 1px var(--space-2);
    border-radius: 999px;
    background: var(--fill-2);
    font-size: var(--text-xs);
    color: var(--text);
  }

  .tag-chip button {
    display: flex;
    padding: 2px;
    border: 0;
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }

  .tag-chip button:hover {
    color: var(--text);
  }

  .field-key {
    margin-inline-end: 4px;
    color: var(--text-dim);
  }

  .suggestion-text {
    font-size: var(--text-sm);
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 4.5em;
    overflow-y: auto;
  }

  .tag-new {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--text-muted);
  }

  .tag-new input {
    width: 120px;
    border: 0;
    background: transparent;
    font: inherit;
    font-size: var(--text-xs);
    color: var(--text);
    outline: none;
  }

  .tag-new:focus-within {
    color: var(--accent);
  }

  .msg.pending {
    opacity: 0.72;
  }

  .msg.failed {
    background: color-mix(in srgb, var(--warning) 14%, var(--fill-1));
  }

  .msg-failed {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-xs);
    color: var(--warning);
  }

  .msg {
    transition: opacity 140ms ease-out;
  }

  @media (prefers-reduced-motion: reduce) {
    .msg {
      transition: none;
    }
  }

  .msg-tools {
    position: absolute;
    top: -14px;
    inset-inline-end: var(--space-2);
    display: none;
    align-items: center;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--surface-raised, var(--fill-2));
    border: var(--hairline) solid var(--separator);
  }

  .msg:hover .msg-tools,
  .msg:focus-within .msg-tools,
  .msg-tools.open {
    display: flex;
  }

  .tool {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }

  .tool:hover:not(:disabled) {
    background: var(--fill-1);
    color: var(--text);
  }

  .tool:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .tool.emoji {
    font-size: var(--text-base);
  }

  .quote,
  .reply-body {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .quote {
    padding: 4px var(--space-2);
    border-inline-start: 3px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--fill-2);
  }

  .quote-who {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--accent-text, var(--accent));
  }

  .quote-text {
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .reactions {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding-bottom: 4px;
  }

  .reaction {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 1px 6px;
    border: var(--hairline) solid var(--separator);
    border-radius: 999px;
    background: var(--fill-2);
    font: inherit;
    font-size: var(--text-sm);
    color: var(--text);
    cursor: pointer;
  }

  .reaction.mine {
    border-color: var(--accent);
  }

  .reaction:disabled {
    cursor: default;
  }

  .reaction-count {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .reply-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4) 0;
    color: var(--text-dim);
    border-top: var(--hairline) solid var(--separator);
  }

  .reply-body {
    flex: 1;
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

  .pending {
    list-style: none;
    margin: 0;
    padding: var(--space-2) var(--space-4) 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    border-top: var(--hairline) solid var(--separator);
  }

  .pending li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-dim);
    min-width: 0;
  }

  .pending-when {
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }

  .pending-text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }

  .schedule-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4) 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .composer .active {
    color: var(--accent);
  }

  .send-err {
    padding: 0 var(--space-4) var(--space-3);
    font-size: var(--text-xs);
    color: var(--warning);
  }
</style>
