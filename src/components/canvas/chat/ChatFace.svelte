<script lang="ts">
  import { getContext, onDestroy, onMount, tick } from "svelte";
  import { termScreen } from "$lib/canvas/term/client";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import type { CanvasNode } from "$lib/canvas/model/types";
  import Markdown from "$components/central/threads/Markdown.svelte";
  import { ENGINE_KEY, type CanvasEngine } from "$lib/canvas/engine.svelte";
  import { linkAt, richMarkdown } from "$lib/canvas/notes/rich";
  import { copyText } from "$lib/canvas/notes/rich/clipboard";
  import { openPathOnCanvas, workspaceDir } from "$lib/canvas/notes/rich/open";
  import { chats } from "$lib/canvas/chat/store.svelte";
  import {
    THREADS,
    attachmentSave,
    blobToBase64,
    chatClear,
    chatDeleteEntry,
    chatInterrupt,
    chatSend,
    chatWipe,
    type ChatAttachment,
    type ChatEntry,
  } from "$lib/canvas/chat/client";

  let {
    node,
    workspaceId,
    focused = false,
    termId = "",
    onshell,
    ondata,
  }: {
    node: CanvasNode;
    workspaceId: string;
    focused?: boolean;
    termId?: string;
    onshell?: () => void;
    ondata?: (data: Record<string, unknown>) => void;
  } = $props();

  const DRAG_MIME = "application/x-omniget-file";

  // svelte-ignore state_referenced_locally
  const chat = chats.of(node.id);
  const richEngine = getContext<CanvasEngine | undefined>(ENGINE_KEY);
  const richBase = $derived(
    (typeof node.data?.cwd === "string" && node.data.cwd) || (richEngine ? workspaceDir(richEngine) : null),
  );
  function openRichPath(abs: string, isDir: boolean, line?: number) {
    if (richEngine) openPathOnCanvas(richEngine, node, abs, isDir, line);
  }
  let text = $state("");
  let files = $state<string[]>([]);
  let sending = $state(false);
  let error = $state("");
  let scroller = $state<HTMLDivElement | null>(null);
  let input = $state<HTMLTextAreaElement | null>(null);
  let stick = true;

  const color = $derived(THREADS[chat.thread]?.color ?? THREADS[0].color);
  const showShell = $derived.by(() => {
    const d = (node.data ?? {}) as Record<string, unknown>;
    return d.showShellWindow === true || d.showsShellWindow === true;
  });
  let shellText = $state("");
  let shellTimer: ReturnType<typeof setInterval> | null = null;

  $effect(() => {
    const on = showShell && chat.turnOpen && !!termId;
    if (on && !shellTimer) {
      const pull = () =>
        termScreen(termId, 14)
          .then((s) => (shellText = s.text.replace(/\s+$/, "")))
          .catch(() => {});
      pull();
      shellTimer = setInterval(pull, 700);
    } else if (!on && shellTimer) {
      clearInterval(shellTimer);
      shellTimer = null;
    }
  });

  onDestroy(() => {
    if (shellTimer) clearInterval(shellTimer);
    if (flashTimer) clearTimeout(flashTimer);
  });

  function toggleShellWindow() {
    ondata?.({ showShellWindow: !showShell, showsShellWindow: undefined });
  }

  let flashId = $state<string | null>(null);
  let flashTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    const req = chat.focusRequest;
    if (!req || chat.loading || chat.thread !== req.thread) return;
    chat.focusRequest = null;
    void revealEntry(req.entryId, req.term);
  });

  async function revealEntry(entryId?: string, term?: string) {
    if (!entryId) return;
    for (let i = 0; i < 12 && !chat.entries.some((e) => e.id === entryId) && chat.hasOlder; i++) {
      await chat.older();
    }
    await tick();
    const el = scroller?.querySelector<HTMLElement>(`[data-entry-id="${CSS.escape(entryId)}"]`);
    if (!el) return;
    stick = false;
    el.scrollIntoView({ block: "center" });
    flashId = entryId;
    const marks = term ? markTerm(el, term) : [];
    if (flashTimer) clearTimeout(flashTimer);
    flashTimer = setTimeout(() => {
      flashId = null;
      for (const m of marks) m.replaceWith(document.createTextNode(m.textContent ?? ""));
    }, 4000);
  }

  function markTerm(root: HTMLElement, term: string): HTMLElement[] {
    const fold = (x: string) => x.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
    const words = term.split(/\s+/).map(fold).filter(Boolean);
    const out: HTMLElement[] = [];
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    const texts: Text[] = [];
    for (let n = walker.nextNode(); n; n = walker.nextNode()) texts.push(n as Text);
    for (const w of words) {
      for (const tn of texts) {
        const raw = tn.textContent ?? "";
        const folded = Array.from(raw, (c) => fold(c).charAt(0) || c).join("");
        const at = folded.indexOf(w);
        if (at < 0 || !tn.parentNode) continue;
        const range = document.createRange();
        range.setStart(tn, at);
        range.setEnd(tn, Math.min(raw.length, at + w.length));
        const mark = document.createElement("mark");
        mark.className = "hit";
        try {
          range.surroundContents(mark);
          out.push(mark);
        } catch {
        }
        break;
      }
    }
    return out;
  }
  const agentName = $derived(node.title || $t("canvas.chat.agent", { default: "Agent" }));

  onMount(() => {
    void chat.load(workspaceId).then(scrollToEnd);
  });

  $effect(() => {
    void chat.entries.length;
    if (stick) void tick().then(scrollToEnd);
  });

  $effect(() => {
    if (focused) queueMicrotask(() => input?.focus());
  });

  function scrollToEnd() {
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }

  function onScroll() {
    if (!scroller) return;
    stick = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 40;
    if (scroller.scrollTop < 30 && chat.hasOlder) {
      const h = scroller.scrollHeight;
      void chat.older().then(async () => {
        await tick();
        if (scroller) scroller.scrollTop = scroller.scrollHeight - h;
      });
    }
  }

  async function selectThread(i: number) {
    if (chat.turnOpen && i !== chat.turnThread) return;
    stick = true;
    await chat.select(i);
    if (chat.error) {
      error = chat.error;
      chat.error = "";
    }
  }

  async function interrupt() {
    error = "";
    try {
      await chatInterrupt(workspaceId, node.id);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function send() {
    const body = text.trim();
    if ((!body && !files.length) || sending) return;
    sending = true;
    error = "";
    const clientId = crypto.randomUUID();
    const optimistic: ChatEntry = {
      id: `local-${clientId}`,
      thread: chat.thread,
      role: "user",
      kind: "message",
      ts: new Date().toISOString(),
      text: body,
      clientMessageId: clientId,
    };
    chat.entries = [...chat.entries, optimistic];
    const sent = { text, files };
    text = "";
    files = [];
    stick = true;
    try {
      const e = await chatSend(workspaceId, node.id, chat.thread, body, sent.files, clientId);
      chat.receive(e, true);
    } catch (e) {
      chat.entries = chat.entries.filter((x) => x.id !== optimistic.id);
      text = sent.text;
      files = sent.files;
      error = e instanceof Error ? e.message : String(e);
    } finally {
      sending = false;
    }
  }

  async function clear() {
    try {
      await chatClear(workspaceId, node.id, chat.thread);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  let menuFor = $state<string | null>(null);
  let menuLink = $state<{ url?: string; path?: string } | null>(null);
  let confirmWipe = $state(false);

  function openMenu(ev: MouseEvent, id: string) {
    if (id.startsWith("local-")) return;
    ev.preventDefault();
    ev.stopPropagation();
    menuLink = linkAt(ev.target);
    menuFor = menuFor === id ? null : id;
  }

  function copyLink(value: string) {
    menuFor = null;
    void copyText(value);
  }

  async function copyEntry(e: ChatEntry) {
    menuFor = null;
    await navigator.clipboard?.writeText(e.text).catch(() => {});
  }

  function quoteEntry(e: ChatEntry) {
    menuFor = null;
    const q = e.text
      .split("\n")
      .map((l) => `> ${l}`)
      .join("\n");
    text = text ? `${text}\n${q}\n` : `${q}\n`;
    queueMicrotask(() => input?.focus());
  }

  async function deleteEntry(e: ChatEntry) {
    menuFor = null;
    error = "";
    try {
      await chatDeleteEntry(workspaceId, node.id, e.id, e.thread);
      chat.entries = chat.entries.filter((x) => x.id !== e.id);
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    }
  }

  async function wipe() {
    confirmWipe = false;
    error = "";
    try {
      await chatWipe(workspaceId, node.id);
      chat.entries = [];
      await chat.load(workspaceId, chat.thread);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      error = msg.startsWith("CHAT_TURN_ACTIVE")
        ? $t("canvas.chat.wipeBusy", { default: "The agent is answering. Wait for the turn to end before clearing the history." })
        : msg;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && chat.turnOpen && !e.isComposing) {
      e.preventDefault();
      e.stopPropagation();
      void interrupt();
      return;
    }
    if (e.key === "Escape" && !e.isComposing && workspaceId) {
      void chat.peek(workspaceId).then(() => {
        if (chat.turnOpen) void interrupt();
      });
      return;
    }
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void send();
      return;
    }
    if (e.key === "/" && !text && onshell) {
      e.preventDefault();
      onshell();
      return;
    }
    if ((e.metaKey || e.ctrlKey) && /^[1-7]$/.test(e.key)) {
      e.preventDefault();
      void selectThread(Number(e.key) - 1);
    }
  }

  async function onPaste(e: ClipboardEvent) {
    const items = Array.from(e.clipboardData?.items ?? []).filter((i) => i.kind === "file");
    if (!items.length) return;
    e.preventDefault();
    for (const it of items) {
      const f = it.getAsFile();
      if (!f) continue;
      try {
        const path = await attachmentSave(workspaceId, f.name || "pasted.png", await blobToBase64(f));
        files = [...files, path];
      } catch (err) {
        error = err instanceof Error ? err.message : String(err);
      }
    }
  }

  function dragAttachment(e: DragEvent, a: ChatAttachment) {
    if (!e.dataTransfer) return;
    e.dataTransfer.effectAllowed = "copy";
    e.dataTransfer.setData(DRAG_MIME, JSON.stringify({ path: a.path, name: a.filename, isImage: a.isImage }));
    e.dataTransfer.setData("text/uri-list", `file://${a.path}`);
    e.dataTransfer.setData("text/plain", a.path);
  }

  function time(ts: string): string {
    const d = new Date(ts);
    return Number.isNaN(d.getTime()) ? "" : d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  function src(a: ChatAttachment): string {
    try {
      return convertFileSrc(a.path);
    } catch {
      return a.path;
    }
  }
</script>

<div class="chat-face" style:--thread={color} data-node-body>
  <div class="threads" role="tablist" aria-label={$t("canvas.chat.threads", { default: "Threads" })}>
    {#each THREADS as th, i (th.key)}
      {@const unread = chat.unread[i] ?? 0}
      {@const info = chat.threads.find((x) => x.index === i)}
      <button
        type="button"
        role="tab"
        class="thread"
        class:on={chat.thread === i}
        style:--c={th.color}
        aria-selected={chat.thread === i}
        disabled={chat.turnOpen && chat.turnThread !== i}
        title={$t(`canvas.chat.thread.${th.key}`, { default: th.key })}
        onclick={() => selectThread(i)}
      >
        <span class="dot" class:empty={!info?.count}></span>
        {#if unread}<span class="unread">{unread}</span>{/if}
      </button>
    {/each}
    <span class="spacer"></span>
    <button type="button" class="tool" onclick={clear} title={$t("canvas.chat.clear", { default: "Clear thread" })}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M4 12h16" /></svg>
    </button>
    <button
      type="button"
      class="tool"
      class:danger={confirmWipe}
      disabled={chat.turnOpen}
      onclick={() => (confirmWipe ? wipe() : (confirmWipe = true))}
      onblur={() => (confirmWipe = false)}
      title={confirmWipe
        ? $t("canvas.chat.wipeConfirm", { default: "Click again to delete every thread and attachment of this chat" })
        : $t("canvas.chat.wipe", { default: "Clear Chat History…" })}
      aria-label={$t("canvas.chat.wipe", { default: "Clear Chat History…" })}
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 7h16 M9 7V4h6v3 M6 7l1 13h10l1-13" /></svg>
    </button>
    {#if onshell}
      <button type="button" class="tool" onclick={() => onshell?.()} title={$t("canvas.chat.shell", { default: "Show terminal (⌘J)" })}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 5h16v14H4z M7 9l3 3-3 3 M12 15h5" /></svg>
      </button>
    {/if}
  </div>

  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="log" bind:this={scroller} onscroll={onScroll} onclick={() => (menuFor = null)}>
    {#if chat.hasOlder}
      <button type="button" class="older" onclick={() => chat.older()}>{$t("canvas.chat.older", { default: "Load earlier messages" })}</button>
    {/if}
    {#if !chat.entries.length && !chat.loading}
      <div class="empty">
        <p>{$t("canvas.chat.empty", { default: "Talk to {{name}} here. Replies arrive through omniget canvas say, so the transcript stays clean.", name: agentName })}</p>
      </div>
    {/if}
    {#each chat.entries as e (e.id)}
      {#if e.kind === "divider"}
        <div class="divider"><span>{$t("canvas.chat.cleared", { default: "Cleared" })}</span></div>
      {:else if e.kind === "interrupted"}
        <div class="divider interrupted" data-entry-id={e.id}>
          <span>{e.text === "Interrupted" ? $t("canvas.chat.interrupted", { default: "Interrupted" }) : e.text}</span>
        </div>
      {:else}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="msg"
          class:user={e.role === "user"}
          class:agent={e.role === "agent"}
          class:progress={e.kind === "progress"}
          class:flash={flashId === e.id}
          data-entry-id={e.id}
          oncontextmenu={(ev) => openMenu(ev, e.id)}
        >
          {#if menuFor === e.id}
            <div class="msg-menu" role="menu" data-canvas-chrome>
              {#if menuLink?.url}
                {@const url = menuLink.url}
                <button type="button" role="menuitem" onclick={() => copyLink(url)}>{$t("canvas.rich.copyLink", { default: "Copy link" })}</button>
              {:else if menuLink?.path}
                {@const path = menuLink.path}
                <button type="button" role="menuitem" onclick={() => copyLink(path)}>{$t("canvas.rich.copyPath", { default: "Copy path" })}</button>
              {/if}
              <button type="button" role="menuitem" onclick={() => copyEntry(e)}>{$t("canvas.chat.copy", { default: "Copy" })}</button>
              <button type="button" role="menuitem" onclick={() => quoteEntry(e)}>{$t("canvas.chat.quote", { default: "Quote" })}</button>
              <button type="button" role="menuitem" class="danger" onclick={() => deleteEntry(e)}>{$t("canvas.chat.deleteMessage", { default: "Delete Message" })}</button>
            </div>
          {/if}
          {#if e.role === "agent" && e.kind === "progress"}
            <span class="spinner" aria-hidden="true"></span>
          {/if}
          <div class="bubble">
            {#if e.text}
              {#if e.role === "agent"}
                <div use:richMarkdown={{ baseDir: richBase, onOpenPath: openRichPath, linkMenu: false }}><Markdown text={e.text} /></div>
              {:else}
                <p class="plain">{e.text}</p>
              {/if}
            {/if}
            {#if e.attachments?.length}
              <div class="atts">
                {#each e.attachments as a (a.id)}
                  <div class="att" draggable="true" ondragstart={(ev) => dragAttachment(ev, a)} title={a.path} role="img" aria-label={a.filename}>
                    {#if a.isImage}
                      <img src={src(a)} alt={a.filename} loading="lazy" />
                    {:else}
                      <span class="file-icon" aria-hidden="true">
                        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M6 3h8l4 4v14H6z M14 3v4h4" /></svg>
                      </span>
                    {/if}
                    <span class="fname">{a.filename}</span>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
          <span class="meta">{time(e.ts)}</span>
        </div>
      {/if}
    {/each}
    {#if chat.turnOpen}
      <div class="working">
        <span class="spinner" aria-hidden="true"></span>
        {$t("canvas.chat.working", { default: "{{name}} is working…", name: agentName })}
        {#if onshell}
          <button type="button" class="link" onclick={() => onshell?.()}>{$t("canvas.chat.peek", { default: "Peek at the terminal" })}</button>
        {/if}
        <button type="button" class="link" onclick={interrupt} title={$t("canvas.chat.interruptHint", { default: "Interrupt the agent (Esc)" })}>
          {$t("canvas.chat.interrupt", { default: "Stop" })}
        </button>
        {#if ondata && termId}
          <button
            type="button"
            class="chevron"
            class:open={showShell}
            onclick={toggleShellWindow}
            aria-expanded={showShell}
            title={showShell
              ? $t("canvas.chat.hideShell", { default: "Hide the shell window" })
              : $t("canvas.chat.showShell", { default: "Show the shell under the chat" })}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
          </button>
        {/if}
      </div>
      {#if showShell && termId}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
        <pre class="shell-window" title={$t("canvas.chat.shell", { default: "Show terminal (⌘J)" })} onclick={() => onshell?.()}>{shellText}</pre>
      {/if}
    {/if}
  </div>

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <div class="input">
    {#if files.length}
      <div class="pending">
        {#each files as f, i (f)}
          <span class="chip">
            {f.split(/[\\/]/).pop()}
            <button type="button" aria-label={$t("canvas.chat.remove", { default: "Remove" })} onclick={() => (files = files.filter((_, j) => j !== i))}>×</button>
          </span>
        {/each}
      </div>
    {/if}
    <textarea
      bind:this={input}
      bind:value={text}
      rows="1"
      placeholder={$t("canvas.chat.placeholder", { default: "Message {{name}} (/ for the terminal)", name: agentName })}
      onkeydown={onKey}
      onpaste={onPaste}
    ></textarea>
    <button type="button" class="send" disabled={sending || (!text.trim() && !files.length)} onclick={send} aria-label={$t("canvas.chat.send", { default: "Send" })}>
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 19V5 M5 12l7-7 7 7" /></svg>
    </button>
  </div>
</div>

<style>
  .chat-face {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-height: 0;
    background: var(--surface, #111);
    color: var(--text);
    font-size: 13px;
  }
  .threads {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  .thread {
    all: unset;
    position: relative;
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    cursor: pointer;
  }
  .thread:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .thread.on {
    background: color-mix(in srgb, var(--c) 22%, transparent);
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--c);
  }
  .dot.empty {
    background: transparent;
    box-shadow: inset 0 0 0 2px var(--c);
  }
  .unread {
    position: absolute;
    top: -3px;
    right: -4px;
    min-width: 13px;
    height: 13px;
    padding: 0 3px;
    border-radius: 7px;
    background: var(--danger, #e5484d);
    color: #fff;
    font-size: 9px;
    line-height: 13px;
    text-align: center;
  }
  .spacer {
    flex: 1;
  }
  .tool {
    all: unset;
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    color: var(--text-muted);
    cursor: pointer;
  }
  .tool:hover {
    background: var(--hover, rgba(127, 127, 127, 0.14));
    color: var(--text);
  }
  .shell-window {
    margin: 0;
    padding: 8px 10px;
    max-height: 180px;
    overflow: hidden;
    border-radius: 8px;
    background: #0b0b0c;
    color: #d7d7db;
    font: 11px/1.35 var(--font-mono, ui-monospace, Menlo, monospace);
    white-space: pre;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
  }
  .chevron {
    all: unset;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 5px;
    cursor: pointer;
    color: var(--text-muted);
    transform: rotate(-90deg);
    transition: transform 0.15s ease;
  }
  .chevron.open {
    transform: none;
  }
  .msg.flash .bubble {
    outline: 2px solid var(--thread);
    outline-offset: 2px;
  }
  .msg :global(mark.hit) {
    background: color-mix(in srgb, var(--thread) 45%, transparent);
    color: inherit;
    border-radius: 3px;
  }
  .divider.interrupted span {
    color: var(--danger, #e5484d);
  }
  .log {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    user-select: text;
  }
  .older,
  .link {
    all: unset;
    align-self: center;
    font-size: 12px;
    color: var(--accent);
    cursor: pointer;
  }
  .empty {
    margin: auto;
    max-width: 320px;
    text-align: center;
    color: var(--text-muted);
  }
  .divider {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-muted);
    font-size: 11px;
  }
  .divider::before,
  .divider::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--border);
  }
  .msg-menu {
    position: absolute;
    top: 2px;
    right: 4px;
    z-index: 5;
    display: flex;
    flex-direction: column;
    min-width: 140px;
    padding: 4px;
    border-radius: 8px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
  }
  .msg-menu button {
    all: unset;
    padding: 5px 8px;
    border-radius: 5px;
    font-size: 12px;
    color: var(--text);
    cursor: pointer;
  }
  .msg-menu button:hover {
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .msg-menu .danger,
  .tool.danger {
    color: #ef4444;
  }
  .msg {
    position: relative;
    display: flex;
    align-items: flex-end;
    gap: 6px;
    max-width: 92%;
  }
  .msg.user {
    align-self: flex-end;
    flex-direction: row-reverse;
  }
  .msg.agent {
    align-self: flex-start;
  }
  .bubble {
    padding: 7px 11px;
    border-radius: 12px;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .user .bubble {
    background: color-mix(in srgb, var(--thread) 28%, var(--surface, #111));
    border-bottom-right-radius: 4px;
  }
  .agent .bubble {
    background: var(--hover, rgba(127, 127, 127, 0.1));
    border-bottom-left-radius: 4px;
  }
  .progress .bubble {
    background: transparent;
    color: var(--text-muted);
    font-style: italic;
    padding: 2px 4px;
  }
  .plain {
    margin: 0;
    white-space: pre-wrap;
  }
  .bubble :global(p:first-child) {
    margin-top: 0;
  }
  .bubble :global(p:last-child) {
    margin-bottom: 0;
  }
  .meta {
    font-size: 10px;
    color: var(--text-muted);
    flex: none;
  }
  .atts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }
  .att {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-width: 180px;
    padding: 6px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--surface, #111);
    cursor: grab;
  }
  .att img {
    max-width: 168px;
    max-height: 120px;
    border-radius: 5px;
    object-fit: cover;
  }
  .file-icon {
    color: var(--text-muted);
  }
  .fname {
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .working {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .spinner {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 2px solid var(--thread);
    border-right-color: transparent;
    animation: spin 0.8s linear infinite;
    flex: none;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
  }
  .error {
    margin: 0 12px 4px;
    font-size: 12px;
    color: var(--danger, #e5484d);
  }
  .input {
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
  }
  .pending {
    grid-column: 1 / -1;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 4px 2px 8px;
    border-radius: 99px;
    background: var(--hover, rgba(127, 127, 127, 0.14));
    font-size: 11px;
  }
  .chip button {
    all: unset;
    cursor: pointer;
    padding: 0 4px;
    color: var(--text-muted);
  }
  textarea {
    resize: none;
    max-height: 140px;
    field-sizing: content;
    padding: 7px 10px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--input-bg, transparent);
    color: var(--text);
    font: inherit;
  }
  textarea:focus {
    outline: none;
    border-color: var(--thread);
  }
  .send {
    all: unset;
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--thread);
    color: #fff;
    cursor: pointer;
  }
  .send:disabled {
    opacity: 0.4;
    cursor: default;
  }
</style>
