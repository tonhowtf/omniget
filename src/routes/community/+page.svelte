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
      try {
        const page = await invoke<ChatPage>("community_chat_messages", { after: last, wait: !first });
        if (id !== session) return;
        merge(page, first);
        first = false;
        status = "live";
        backoff = 2000;
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
  <h1>{$t("community.title")}</h1>
  <p>{$t("community.lead")}</p>
</header>

{#if !connected}
  <section class="surface-card chat-gate" aria-labelledby="chat-gate-title">
    <h2 id="chat-gate-title">{$t("community.chat.gate_title")}</h2>
    <ul class="chat-facts">
      <li>{$t("community.chat.fact_anonymous")}</li>
      <li>{$t("community.chat.fact_public")}</li>
      <li>{$t("community.chat.fact_connection")}</li>
    </ul>
    <div class="chat-gate-actions">
      <button type="button" class="btn btn-primary" onclick={connect}>{$t("community.chat.connect")}</button>
      <label class="chat-auto">
        <input type="checkbox" checked={autoConnect} onchange={(e) => toggleAuto(e.currentTarget.checked)} />
        {$t("community.chat.autoconnect")}
      </label>
    </div>
  </section>
{:else}
  <section class="surface-card chat-room" aria-labelledby="chat-room-title">
    <div class="chat-bar">
      <h2 id="chat-room-title">{$t("community.chat.room_title")}</h2>
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
        <li class="chat-msg" class:official={m.author}>
          <div class="chat-meta">
            <span class="chat-name">{m.name}</span>
            {#if m.author}<span class="tag tag-accent">{$t("community.chat.official")}</span>{/if}
            {#if m.idea}<span class="tag">{$t("community.chat.idea")}</span>{/if}
            <time datetime={m.t.replace(" ", "T") + "Z"}>{localTime(m.t)}</time>
            {#if !m.author}
              <button type="button" class="chat-report" disabled={reported.has(m.id)} onclick={() => report(m)}>
                {reported.has(m.id) ? $t("community.chat.reported_short") : $t("community.chat.report")}
              </button>
            {/if}
          </div>
          <p class="chat-text">{m.text}</p>
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
  .community-head h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    line-height: var(--leading-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
  }
  .community-head p {
    margin: var(--space-1) 0 0;
    max-width: 60ch;
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
  .chat-gate {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-5);
  }
  .chat-facts {
    margin: 0;
    padding-inline-start: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--text-muted);
    font-size: var(--text-base);
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
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-block-end: 1px solid var(--separator);
  }
  .chat-bar h2 { flex: 1; }
  .chat-status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .chat-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-dim);
  }
  .chat-status[data-status="live"] .chat-dot { background: var(--success); }
  .chat-status[data-status="offline"] .chat-dot { background: var(--danger); }
  .chat-list {
    list-style: none;
    margin: 0;
    padding: var(--space-3) var(--space-4);
    height: min(56vh, 520px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .chat-empty {
    margin: auto;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .chat-msg { display: flex; flex-direction: column; gap: 2px; }
  .chat-msg.official .chat-text {
    border-inline-start: 2px solid var(--accent);
    padding-inline-start: var(--space-2);
  }
  .chat-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .chat-name { font-weight: 600; color: var(--text); }
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
  }
  .chat-compose {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-block-start: 1px solid var(--separator);
  }
  .chat-input {
    height: auto;
    min-height: calc(var(--control-h) * 1.6);
    padding: var(--space-2);
    resize: vertical;
    line-height: var(--leading-base);
  }
  .chat-compose-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .chat-hint { font-size: var(--text-sm); color: var(--text-dim); }
  .chat-error { color: var(--danger); }
  .community-more {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .community-more a { color: var(--accent-text, var(--accent)); }
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
