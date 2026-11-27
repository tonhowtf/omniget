<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, tick } from "svelte";
  import { t, locale } from "$lib/i18n";

  type ChatMessage = { id: number; t: string; name: string; text: string; author: boolean; idea: boolean };
  type ChatPage = { messages: ChatMessage[]; last: number; frozen: boolean; gone: number[] };
  type ChatAnswer = { ok: boolean; state: string; id?: number | null };

  const AUTO_KEY = "omniget.community.autoconnect";
  const MAX_TEXT = 600;
  const SHOWN = 200;
  const AVATAR_TONES = [
    "var(--info)",
    "var(--success)",
    "var(--purple)",
    "var(--teal)",
    "var(--warning)",
    "var(--accent)",
  ];

  function readAuto(): boolean {
    try { return localStorage.getItem(AUTO_KEY) === "1"; } catch { return false; }
  }
  function writeAuto(on: boolean) {
    try { on ? localStorage.setItem(AUTO_KEY, "1") : localStorage.removeItem(AUTO_KEY); } catch { }
  }

  let autoConnect = $state(readAuto());
  let connected = $state(false);
  let status = $state<"idle" | "loading" | "live" | "offline">("idle");
  let messages = $state<ChatMessage[]>([]);
  let frozen = $state(false);
  let draft = $state("");
  let sending = $state(false);
  let notice = $state<{ kind: "ok" | "error"; text: string } | null>(null);
  let reported = $state<Set<number>>(new Set());
  let listEl = $state<HTMLOListElement | null>(null);
  let inputEl = $state<HTMLTextAreaElement | null>(null);

  let session = 0;
  let last = 0;

  const STATE_KEYS: Record<string, string> = {
    slow: "community.chat.state_slow",
    limit: "community.chat.state_limit",
    frozen: "community.chat.state_frozen",
    ticket: "community.chat.state_error",
    error: "community.chat.state_error",
  };

  function avatarTone(name: string, official: boolean): string {
    if (official) return "var(--accent)";
    let hash = 0;
    for (let i = 0; i < name.length; i += 1) hash = (hash * 31 + name.charCodeAt(i)) >>> 0;
    return AVATAR_TONES[hash % AVATAR_TONES.length];
  }

  function initials(name: string): string {
    const clean = name.replace(/^@/, "").trim();
    if (!clean) return "?";
    const parts = clean.split(/[\s_-]+/).filter(Boolean);
    if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
    return clean.slice(0, 2).toUpperCase();
  }

  function splitIdea(text: string): { title: string; body: string } {
    const idx = text.indexOf("\n");
    if (idx < 0) return { title: text, body: "" };
    return { title: text.slice(0, idx).trim(), body: text.slice(idx + 1).trim() };
  }

  function nearBottom(): boolean {
    if (!listEl) return true;
    return listEl.scrollHeight - listEl.scrollTop - listEl.clientHeight < 80;
  }

  async function scrollToEnd(force = false) {
    const stick = force || nearBottom();
    await tick();
    if (stick && listEl) listEl.scrollTop = listEl.scrollHeight;
  }

  function merge(page: ChatPage, first: boolean) {
    frozen = page.frozen;
    const gone = new Set(page.gone ?? []);
    const known = new Set(messages.map((m) => m.id));
    const fresh = page.messages.filter((m) => !known.has(m.id));
    messages = [...messages.filter((m) => !gone.has(m.id)), ...fresh].slice(-SHOWN);
    if (page.last > last) last = page.last;
    if (fresh.length || first) void scrollToEnd(first);
  }

  async function poll(id: number) {
    let backoff = 2000;
    let first = true;
    while (connected && id === session) {
      const started = Date.now();
      try {
        const page = await invoke<ChatPage>("community_chat_messages", { after: last, wait: !first });
        if (id !== session) return;
        merge(page, first);
        const wasFirst = first;
        first = false;
        status = "live";
        backoff = 2000;
        if (!wasFirst && Date.now() - started < 500) {
          await new Promise((r) => setTimeout(r, 1000));
        }
      } catch {
        if (id !== session) return;
        status = "offline";
        await new Promise((r) => setTimeout(r, backoff));
        backoff = Math.min(backoff * 2, 30000);
      }
    }
  }

  function connect() {
    if (connected) return;
    connected = true;
    status = "loading";
    messages = [];
    last = 0;
    session += 1;
    void poll(session);
  }

  function disconnect() {
    connected = false;
    session += 1;
    status = "idle";
    messages = [];
    notice = null;
  }

  function toggleAuto(on: boolean) {
    autoConnect = on;
    writeAuto(on);
  }

  onMount(() => {
    if (readAuto()) connect();
    return () => { connected = false; session += 1; };
  });

  async function send(e?: Event) {
    e?.preventDefault();
    const text = draft.trim();
    if (!text || sending || frozen) return;
    sending = true;
    notice = null;
    try {
      const answer = await invoke<ChatAnswer>("community_chat_send", { text, lang: $locale ?? "en", title: null });
      if (answer.ok) {
        draft = "";
        void scrollToEnd(true);
      } else {
        notice = { kind: "error", text: $t(STATE_KEYS[answer.state] ?? "community.chat.state_error") };
      }
    } catch {
      notice = { kind: "error", text: $t("community.chat.state_offline") };
    } finally {
      sending = false;
      await tick();
      inputEl?.focus();
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) void send(e);
  }

  async function report(m: ChatMessage) {
    if (reported.has(m.id)) return;
    try {
      const answer = await invoke<ChatAnswer>("community_chat_report", { id: m.id });
      if (answer.ok) {
        reported = new Set([...reported, m.id]);
        notice = { kind: "ok", text: $t("community.chat.reported") };
      }
    } catch {
      notice = { kind: "error", text: $t("community.chat.state_offline") };
    }
  }

  function localTime(utc: string): string {
    const d = new Date(utc.replace(" ", "T") + ":00Z");
    if (Number.isNaN(d.getTime())) return utc;
    const sameDay = d.toDateString() === new Date().toDateString();
    return sameDay
      ? d.toLocaleTimeString($locale ?? undefined, { hour: "2-digit", minute: "2-digit" })
      : d.toLocaleString($locale ?? undefined, { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" });
  }

  let remaining = $derived(MAX_TEXT - draft.length);
</script>

<header class="community-head">
  <div class="head-title-row">
    <h1>{$t("community.tab.chat")}</h1>
    <span class="badge live-pill">
      <span class="pill-dot" aria-hidden="true"></span>
      {$t("community.chat.live_badge")}
    </span>
  </div>
  <p>{$t("community.lead")}</p>
</header>

{#if !connected}
  <section class="surface-card chat-shell" aria-labelledby="chat-gate-title">
    <div class="chat-bar">
      <span class="chat-room-icon" aria-hidden="true">
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>
      </span>
      <h2 id="chat-gate-title">{$t("community.chat.gate_title")}</h2>
      <span class="chat-host">getomniget.com</span>
    </div>

    <div class="chat-preview" aria-hidden="true">
      <div class="preview-bubble">
        <span class="msg-avatar" style:--av-tone="var(--info)">AK</span>
        <div class="bubble-card">
          <div class="bubble-head"><span class="chat-name">amber-kite</span></div>
          <p>{$t("community.chat.preview_sample_1")}</p>
        </div>
      </div>
      <div class="preview-bubble official">
        <span class="msg-avatar" style:--av-tone="var(--accent)">TO</span>
        <div class="bubble-card">
          <div class="bubble-head">
            <span class="chat-name">tonhowtf</span>
            <span class="tag tag-accent">{$t("community.chat.official")}</span>
          </div>
          <p>{$t("community.chat.preview_sample_2")}</p>
        </div>
      </div>
    </div>

    <div class="chat-gate-body">
      <div class="chat-fact-pills">
        <span class="fact-pill">{$t("community.chat.fact_anonymous_short")}</span>
        <span class="fact-pill">{$t("community.chat.fact_public_short")}</span>
        <span class="fact-pill">{$t("community.chat.fact_local_short")}</span>
      </div>
      <ul class="chat-facts">
        <li>{$t("community.chat.fact_anonymous")}</li>
        <li>{$t("community.chat.fact_public")}</li>
        <li>{$t("community.chat.fact_connection")}</li>
      </ul>
      <div class="chat-gate-actions">
        <button type="button" class="btn btn-primary" onclick={connect}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>
          {$t("community.chat.connect")}
        </button>
        <label class="chat-auto">
          <input type="checkbox" checked={autoConnect} onchange={(e) => toggleAuto(e.currentTarget.checked)} />
          {$t("community.chat.autoconnect")}
        </label>
      </div>
    </div>
  </section>
{:else}
  <section class="surface-card chat-room" aria-labelledby="chat-room-title">
    <div class="chat-bar">
      <span class="chat-room-icon" aria-hidden="true">
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>
      </span>
      <h2 id="chat-room-title">{$t("community.chat.room_title")}</h2>
      {#if messages.length > 0}
        <span class="chat-count">{$t("community.chat.message_count", { count: messages.length })}</span>
      {/if}
      <span class="chat-status" data-status={status} role="status">
        <span class="chat-dot" aria-hidden="true"></span>
        {status === "live" ? $t("community.chat.status_live") : status === "offline" ? $t("community.chat.status_offline") : $t("community.chat.status_loading")}
      </span>
      <button type="button" class="btn btn-secondary btn-sm" onclick={disconnect}>{$t("community.chat.disconnect")}</button>
    </div>

    <ol class="chat-list" bind:this={listEl} aria-live="polite" aria-relevant="additions" aria-label={$t("community.chat.room_title")}>
      {#if status === "loading" && messages.length === 0}
        <li class="chat-empty">{$t("community.chat.status_loading")}</li>
      {:else if messages.length === 0}
        <li class="chat-empty">{$t("community.chat.empty")}</li>
      {/if}
      {#each messages as m (m.id)}
        <li class="chat-msg" class:official={m.author} class:idea={m.idea}>
          <span class="msg-avatar" style:--av-tone={avatarTone(m.name, m.author)} aria-hidden="true">{initials(m.name)}</span>
          <div class="chat-bubble">
            <div class="chat-meta">
              <span class="chat-name">{m.name}</span>
              {#if m.author}<span class="tag tag-accent">{$t("community.chat.official")}</span>{/if}
              {#if m.idea}<span class="tag tag-idea">{$t("community.chat.idea")}</span>{/if}
              <time datetime={m.t.replace(" ", "T") + "Z"}>{localTime(m.t)}</time>
              {#if !m.author}
                <button type="button" class="chat-report" disabled={reported.has(m.id)} onclick={() => report(m)}>
                  {reported.has(m.id) ? $t("community.chat.reported_short") : $t("community.chat.report")}
                </button>
              {/if}
            </div>
            {#if m.idea}
              {@const parts = splitIdea(m.text)}
              <p class="chat-idea-title">{parts.title}</p>
              {#if parts.body}<p class="chat-text">{parts.body}</p>{/if}
            {:else}
              <p class="chat-text">{m.text}</p>
            {/if}
          </div>
        </li>
      {/each}
    </ol>

    <form class="chat-compose" onsubmit={send}>
      <label class="visually-hidden" for="chat-input">{$t("community.chat.placeholder")}</label>
      <textarea
        id="chat-input"
        class="input chat-input"
        bind:this={inputEl}
        bind:value={draft}
        maxlength={MAX_TEXT}
        rows="2"
        placeholder={frozen ? $t("community.chat.state_frozen") : $t("community.chat.placeholder")}
        disabled={frozen}
        onkeydown={onKeydown}
      ></textarea>
      <div class="chat-compose-row">
        <span class="chat-hint">
          {#if notice}
            <span class:chat-error={notice.kind === "error"} role="alert">{notice.text}</span>
          {:else if remaining < 100}
            {$t("community.chat.remaining", { num: remaining })}
          {:else}
            {$t("community.chat.hint")}
          {/if}
        </span>
        <button type="submit" class="btn btn-primary btn-sm" disabled={sending || frozen || !draft.trim()}>
          <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/></svg>
          {sending ? $t("community.chat.sending") : $t("community.chat.send")}
        </button>
      </div>
    </form>
  </section>
{/if}

<p class="community-more">
  {$t("community.more")}
  <a href="https://discord.gg/jgdxyPy7Vn" target="_blank" rel="noopener">Discord</a>
  ·
  <a href="https://getomniget.com/contact" target="_blank" rel="noopener">getomniget.com/contact</a>
</p>

<style>
  .community-head {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .head-title-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-3);
  }
  .community-head h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    line-height: var(--leading-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
  }
  .live-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-xs);
    font-weight: 600;
    padding: 3px 10px;
    border-radius: var(--radius-full);
    background: var(--surface-hi);
    color: var(--text-muted);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .pill-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--success);
  }
  .community-head p {
    margin: 0;
    max-width: 62ch;
    color: var(--text-muted);
    font-size: var(--text-base);
    line-height: var(--leading-base);
  }
  h2 {
    margin: 0;
    font-size: var(--text-md);
    line-height: var(--leading-md);
    font-weight: 600;
  }
  .chat-shell,
  .chat-room {
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 0;
    overflow: hidden;
  }
  .chat-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-block-end: 1px solid var(--separator);
    background: color-mix(in srgb, var(--surface-hi) 45%, var(--surface));
  }
  .chat-bar h2 { flex: 1; }
  .chat-room-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-md);
    background: var(--fill-2);
    color: var(--accent-text, var(--accent));
    flex-shrink: 0;
  }
  .chat-host,
  .chat-count {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .chat-preview {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border-block-end: 1px solid var(--separator);
    background: color-mix(in srgb, var(--bg) 55%, var(--surface));
    opacity: 0.78;
    user-select: none;
  }
  .preview-bubble {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
  }
  .bubble-card {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .preview-bubble.official .bubble-card {
    background: color-mix(in srgb, var(--accent) 8%, var(--surface));
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--accent) 30%, var(--content-border));
  }
  .bubble-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: 2px;
  }
  .bubble-card p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    line-height: 1.5;
  }
  .chat-gate-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
  }
  .chat-fact-pills {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .fact-pill {
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text);
    background: var(--surface-hi);
    padding: 4px 10px;
    border-radius: var(--radius-full);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .chat-facts {
    margin: 0;
    padding-inline-start: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--text-muted);
    font-size: var(--text-sm);
    line-height: var(--leading-base);
  }
  .chat-gate-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3) var(--space-5);
    padding-block-start: var(--space-1);
  }
  .chat-auto {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
    cursor: pointer;
  }
  .chat-status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-xs);
    font-weight: 500;
    padding: 3px 9px;
    border-radius: var(--radius-full);
    background: var(--surface-hi);
    color: var(--text-muted);
  }
  .chat-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-dim);
  }
  .chat-status[data-status="live"] .chat-dot { background: var(--success); }
  .chat-status[data-status="offline"] .chat-dot { background: var(--error, var(--danger)); }
  .chat-list {
    list-style: none;
    margin: 0;
    padding: var(--space-4);
    height: min(56vh, 520px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    background: color-mix(in srgb, var(--bg) 45%, var(--surface));
  }
  .chat-empty {
    margin: auto;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .chat-msg {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
  }
  .msg-avatar {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-full);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.02em;
    flex-shrink: 0;
    background: color-mix(in srgb, var(--av-tone) 20%, var(--surface-hi));
    color: var(--text);
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--av-tone) 45%, transparent);
    margin-top: 2px;
  }
  .chat-bubble {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .chat-msg.official .chat-bubble {
    background: color-mix(in srgb, var(--accent) 9%, var(--surface));
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--accent) 36%, var(--content-border));
  }
  .chat-msg.idea .chat-bubble {
    background: color-mix(in srgb, var(--info) 8%, var(--surface));
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--info) 34%, var(--content-border));
  }
  .chat-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .chat-name { font-size: var(--text-sm); font-weight: 600; color: var(--text); }
  .tag-idea {
    background: color-mix(in srgb, var(--info) 18%, transparent);
    color: var(--info);
  }
  .chat-idea-title {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
  }
  .chat-report {
    margin-inline-start: auto;
    border: 0;
    padding: 0;
    background: none;
    color: var(--text-dim);
    font: inherit;
    cursor: pointer;
    opacity: 0;
  }
  .chat-msg:hover .chat-report,
  .chat-report:focus-visible,
  .chat-report:disabled { opacity: 1; }
  .chat-report:hover:not(:disabled) { color: var(--text); text-decoration: underline; }
  .chat-report:focus-visible { outline: var(--focus-ring); }
  .chat-text {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-size: var(--text-base);
    line-height: var(--leading-base);
    color: var(--text);
  }
  .chat-compose {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-block-start: 1px solid var(--separator);
    background: var(--surface);
  }
  .chat-input {
    height: auto;
    min-height: calc(var(--control-h) * 1.8);
    padding: var(--space-2) var(--space-3);
    resize: vertical;
    line-height: var(--leading-base);
  }
  .chat-compose-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .chat-hint { font-size: var(--text-xs); color: var(--text-dim); }
  .chat-error { color: var(--error, var(--danger)); }
  .community-more {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .community-more a { color: var(--text); text-decoration: underline; text-underline-offset: 2px; }
  .visually-hidden {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  @media (prefers-reduced-motion: no-preference) {
    .chat-report { transition: opacity var(--duration-fast) var(--ease-out); }
  }
</style>
