<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import TerminalView from "$components/central/terminal/TerminalView.svelte";
  import type { CanvasNode } from "$lib/canvas/model/types";
  import type { Lod } from "$lib/canvas/lod";
  import { agentStates, normaliseState, stateCss } from "$lib/canvas/state/agents.svelte";
  import { terminalIdOf } from "$lib/canvas/term/gl-types";
  import { termLive, type AgentStateName } from "$lib/canvas/term/state.svelte";
  import { gridSizeFor, TERM_HEADER_H, termFontScale, termPadding, TERM_FONT_SIZE } from "$lib/canvas/term/metrics";
  import { canvasPrefs } from "$lib/canvas/prefs.svelte";
  import { termGlAvailable } from "$lib/canvas/term/pass";
  import { termHub } from "$lib/canvas/term/hub";
  import { attachTermInput, attachTermPointer, cursorBox, type TermCommand, type TermInputHandle } from "$lib/canvas/term/input";
  import {
    errorText,
    termClose,
    termList,
    termResize,
    termScrollLock,
    termSearch,
    termSearchClear,
    termSearchCount,
    type AgentKind,
  } from "$lib/canvas/term/client";
  import { invoke } from "@tauri-apps/api/core";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { agentCommand, openTerminal } from "$lib/canvas/spawn";
  import { nodeForPath } from "$lib/canvas/files/open";
  import { roles } from "$lib/canvas/roles/store.svelte";
  import { chats } from "$lib/canvas/chat/store.svelte";
  import RoleBadge from "$components/canvas/roles/RoleBadge.svelte";
  import MaestroBadge from "$components/canvas/maestro/MaestroBadge.svelte";
  import ChatFace from "$components/canvas/chat/ChatFace.svelte";

  let {
    node,
    selected = false,
    focused = false,
    zoom = 1,
    lod = "near",
    workspaceId = "",
    onpatch,
    onfocusrequest,
    onexit,
  }: {
    node: CanvasNode;
    selected?: boolean;
    focused?: boolean;
    zoom?: number;
    lod?: Lod;
    workspaceId?: string;
    onpatch?: (patch: Partial<CanvasNode>) => void;
    onfocusrequest?: () => void;
    onexit?: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  const termId = terminalIdOf(node);
  const live = termLive(termId);
  const useGl = termGlAvailable();
  const engine = useEngine();
  let fallbackReady = $state(false);

  let body = $state<HTMLDivElement | null>(null);
  let ta = $state<HTMLTextAreaElement | null>(null);
  let input: TermInputHandle | null = null;
  let detachPointer: (() => void) | null = null;
  let opening = $state(false);
  let sentCols = 0;
  let sentRows = 0;
  let resizeTimer: ReturnType<typeof setTimeout> | null = null;
  let caretTimer: ReturnType<typeof setInterval> | null = null;
  let caret = $state({ x: 0, y: 0, w: 8, h: 16, font: 13 });

  const data = $derived((node.data ?? {}) as Record<string, unknown>);
  const agent = $derived((typeof data.agent === "string" ? data.agent : "shell") as AgentKind);
  const command = $derived(agentCommand(node));
  const cwd = $derived(typeof data.cwd === "string" ? data.cwd : null);
  const role = $derived(roles.byId(typeof data.roleId === "string" ? data.roleId : null));
  const isManager = $derived(!!data.isManager);
  const chatFace = $derived(chats.isChat(node.id));
  const fontScale = $derived(termFontScale(node));

  let findOpen = $state(false);
  let findQuery = $state("");
  let findCount = $state<number | null>(null);
  let findMiss = $state(false);
  let findInput = $state<HTMLInputElement | null>(null);
  let findTimer: ReturnType<typeof setTimeout> | null = null;
  let scrollLocked = $state(false);

  function openFind() {
    findOpen = true;
    queueMicrotask(() => {
      findInput?.focus();
      findInput?.select();
    });
  }

  async function findStep(backwards: boolean) {
    const q = findQuery;
    if (!q) {
      findCount = null;
      findMiss = false;
      await termSearchClear(termId).catch(() => {});
      return;
    }
    try {
      const [hit, count] = await Promise.all([termSearch(termId, q, backwards), termSearchCount(termId, q)]);
      findCount = count;
      findMiss = !hit;
    } catch (e) {
      live.error = errorText(e);
    }
  }

  function onFindInput() {
    if (findTimer) clearTimeout(findTimer);
    findTimer = setTimeout(() => void findStep(true), 120);
  }

  function onFindKey(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      void findStep(!e.shiftKey);
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeFind();
    }
  }

  function closeFind() {
    findOpen = false;
    findQuery = "";
    findCount = null;
    findMiss = false;
    void termSearchClear(termId).catch(() => {});
    setTimeout(focusInput, 0);
  }

  function setFontSize(size: number | null) {
    const appearance = { ...((data.appearance as Record<string, unknown> | undefined) ?? {}) };
    if (size === null) delete appearance.fontSize;
    else appearance.fontSize = Math.max(7, Math.min(36, Math.round(size)));
    onpatch?.({ data: { ...node.data, appearance } });
  }

  function onCommand(cmd: TermCommand) {
    switch (cmd) {
      case "find":
        openFind();
        return;
      case "scroll-lock":
        void termScrollLock(termId)
          .then((on) => (scrollLocked = on))
          .catch((e) => (live.error = errorText(e)));
        return;
      case "font-up":
        setFontSize(TERM_FONT_SIZE * fontScale + 1);
        return;
      case "font-down":
        setFontSize(TERM_FONT_SIZE * fontScale - 1);
        return;
      case "font-reset":
        setFontSize(null);
        return;
    }
  }
  // svelte-ignore state_referenced_locally
  const chatState = chats.of(node.id);
  const chatUnread = $derived(chatState.totalUnread());

  const agentState = $derived.by((): AgentStateName => {
    const sig = agentStates.of(node);
    if (sig && sig.state !== "unknown") {
      return sig.state === "waitingPeer" ? "waiting_peer" : (sig.state as AgentStateName);
    }
    return live.agent;
  });
  const quietAttention = $derived(
    !canvasPrefs().notifications.badges_for_non_maestro &&
      !isManager &&
      (agentState === "success" || agentState === "failed" || agentState === "blocked"),
  );
  const badgeState = $derived<AgentStateName>(quietAttention ? "idle" : agentState);
  const stateColor = $derived(stateCss(normaliseState(badgeState)));
  const title = $derived(node.title || live.title || $t(`canvas.term.agent.${agent}`));
  const card = $derived(lod === "far" || live.lod === "card");

  const SYMBOLS: Record<AgentStateName, string> = {
    idle: "○",
    thinking: "◐",
    working: "●",
    waiting_peer: "⇄",
    blocked: "!",
    success: "✓",
    failed: "✕",
    compacting: "⇊",
    looping: "↻",
    unknown: "?",
  };

  function canvasEnv(): Record<string, string> {
    const env: Record<string, string> = {
      OMNIGET_CANVAS: "1",
      OMNIGET_CANVAS_NODE: node.id,
      OMNIGET_CANVAS_NAME: node.title ?? "",
      OMNIGET_TERMINAL_ID: termId,
    };
    if (workspaceId) env.OMNIGET_CANVAS_WORKSPACE = workspaceId;
    const extra = data.env;
    if (extra && typeof extra === "object") {
      for (const [k, v] of Object.entries(extra as Record<string, unknown>)) {
        if (typeof v === "string") env[k] = v;
      }
    }
    return env;
  }

  async function open(restore: boolean) {
    if (opening) return;
    opening = true;
    live.error = "";
    const size = gridSizeFor(node.w, node.h, fontScale);
    try {
      const info = await openTerminal(node, workspaceId || engine.store.workspaceId, {
        floorId: engine.store.floorId,
        restore,
      });
      fallbackReady = true;
      setTimeout(() => {
        void termList()
          .then((l) => {
            if (l.find((t) => t.id === termId)?.alive) live.exited = null;
          })
          .catch(() => {});
      }, 1500);
      sentCols = info.cols;
      sentRows = info.rows;
      live.exited = info.alive ? null : { code: info.exit_code, signal: info.exit_signal };
      if (info.osc_title && !live.title) live.title = info.osc_title;
      if (info.cols !== size.cols || info.rows !== size.rows) scheduleResize();
    } catch (e) {
      live.error = errorText(e);
    } finally {
      opening = false;
    }
  }

  function scheduleResize() {
    if (resizeTimer) clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      resizeTimer = null;
      const { cols, rows } = gridSizeFor(node.w, node.h, fontScale);
      if (cols === sentCols && rows === sentRows) return;
      sentCols = cols;
      sentRows = rows;
      termResize(termId, cols, rows).catch((e) => (live.error = errorText(e)));
    }, 120);
  }

  async function restart() {
    try {
      await termClose(termId, false);
    } catch {
    }
    termHub.forget(termId);
    live.exited = null;
    await open(false);
    focusInput();
  }

  async function closeTerminal() {
    try {
      await termClose(termId, false);
    } catch (e) {
      live.error = errorText(e);
    }
  }

  function focusInput() {
    onfocusrequest?.();
    if (useGl) input?.focus();
  }

  function updateCaret() {
    if (!body) return;
    const c = cursorBox(termId, body);
    if (c.x !== caret.x || c.y !== caret.y || c.h !== caret.h) caret = c;
  }

  $effect(() => {
    void node.w;
    void node.h;
    void fontScale;
    if (useGl && sentCols) scheduleResize();
  });

  $effect(() => {
    if (focused && useGl && !chatFace) queueMicrotask(() => input?.focus());
  });

  $effect(() => {
    const ws = workspaceId || engine.store.workspaceId;
    if (ws && !chatState.loaded) void chatState.peek(ws);
  });

  $effect(() => {
    if (!useGl || !body || !ta) return;
    const b = body;
    const area = ta;
    input = attachTermInput(area, {
      id: termId,
      onUnfocus: () => onexit?.(),
      optionAsMeta: () => canvasPrefs().terminal.option_as_meta,
      onCommand,
    });
    detachPointer = attachTermPointer(b, {
      id: termId,
      focus: () => {
        onfocusrequest?.();
        input?.focus();
      },
      openInCanvas: (path) => {
        void invoke<boolean[]>("canvas_files_kind", { paths: [path] })
          .catch(() => [false])
          .then((dirs) => nodeForPath(engine, path, !!dirs[0], { x: node.x + node.w + 40, y: node.y }));
      },
    });
    const onFocusIn = () => {
      if (!focused) onfocusrequest?.();
      updateCaret();
      if (caretTimer) clearInterval(caretTimer);
      caretTimer = setInterval(updateCaret, 80);
    };
    const onFocusOut = () => {
      if (caretTimer) clearInterval(caretTimer);
      caretTimer = null;
    };
    area.addEventListener("focus", onFocusIn);
    area.addEventListener("blur", onFocusOut);
    return () => {
      area.removeEventListener("focus", onFocusIn);
      area.removeEventListener("blur", onFocusOut);
      input?.destroy();
      input = null;
      detachPointer?.();
      detachPointer = null;
      if (caretTimer) clearInterval(caretTimer);
      caretTimer = null;
    };
  });

  onMount(() => {
    void open(true);
  });

  // svelte-ignore state_referenced_locally
  let seenRestart = engine.restarts[node.id] ?? 0;
  $effect(() => {
    const r = engine.restarts[node.id] ?? 0;
    if (r !== seenRestart) {
      seenRestart = r;
      termHub.forget(termId);
      live.exited = null;
      void open(false);
    }
  });

  $effect(() => {
    if (!focused) return;
    const path = typeof location !== "undefined" ? location.pathname : "/world";
    void invoke("llm_ui_focus", { path, jobId: null, conversationId: `canvas:${node.id}` }).catch(() => {});
    void invoke("canvas_mark_seen", { nodeId: node.id }).catch(() => {});
    return () => {
      void invoke("llm_ui_focus", { path, jobId: null, conversationId: null }).catch(() => {});
    };
  });

  onDestroy(() => {
    if (findTimer) clearTimeout(findTimer);
    if (resizeTimer) clearTimeout(resizeTimer);
    if (caretTimer) clearInterval(caretTimer);
  });

  function progressStyle(p: number | null): string {
    if (p === null) return "";
    if (p < 0) return "";
    return `width: ${Math.max(0, Math.min(100, p))}%`;
  }
</script>

<div
  class="term-node"
  class:selected
  class:focused
  class:card
  style="--state: {stateColor}; --header-h: {TERM_HEADER_H}px; --pad: {termPadding().x}px;"
  data-terminal-id={termId}
>
  <header class="chrome">
    <span
      class="badge"
      title={$t(`canvas.term.state.${badgeState}`)}
      aria-label={$t(`canvas.term.state.${badgeState}`)}
      class:pulse={agentState === "working" || agentState === "thinking" || agentState === "compacting"}
    >
      <span class="sym" aria-hidden="true">{SYMBOLS[badgeState]}</span>
    </span>
    {#if role}
      <span class="role" title={role.name}><RoleBadge icon={role.icon} color={role.color} size={16} /></span>
    {/if}
    <span class="title" title={live.cwd || title}>{title}</span>
    <MaestroBadge {workspaceId} floorId={engine.store.floorId} nodeId={node.id} on={isManager} compact={zoom < 0.35} />
    {#if live.exited}
      <span class="exit-tag">
        {live.exited.signal
          ? $t("canvas.term.exited_signal", { signal: live.exited.signal })
          : $t("canvas.term.exited_code", { code: live.exited.code ?? "?" })}
      </span>
    {/if}
    <span class="spacer"></span>
    {#if scrollLocked}
      <button type="button" class="lock-tag" title={$t("canvas.term.scroll_locked")} aria-label={$t("canvas.term.scroll_locked")} onclick={() => onCommand("scroll-lock")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 11V8a5 5 0 0 1 10 0v3 M5 11h14v10H5z" /></svg>
      </button>
    {/if}
    {#if zoom >= 0.35}
      <div class="actions">
        {#if !chatFace}
          <button
            type="button"
            class="act chat-btn"
            title={$t("canvas.chat.open", { default: "Chat (⌘J)" })}
            aria-label={chatUnread
              ? $t("canvas.chat.unreadLabel", { default: "Chat, {{count}} unread", count: chatUnread })
              : $t("canvas.chat.open", { default: "Chat (⌘J)" })}
            onclick={() => chats.setFace(node.id, true)}
          >
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 5h16v11H9l-5 4z" /></svg>
            {#if chatUnread}<span class="chat-unread">{chatUnread}</span>{/if}
          </button>
        {/if}
        <button type="button" class="act" title={$t("canvas.term.action.focus")} aria-label={$t("canvas.term.action.focus")} onclick={focusInput}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 9V4h5 M15 4h5v5 M20 15v5h-5 M9 20H4v-5" /></svg>
        </button>
        <button type="button" class="act" title={$t("canvas.term.action.restart")} aria-label={$t("canvas.term.action.restart")} onclick={restart}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 11a8 8 0 1 0-2.3 5.7 M20 4v7h-7" /></svg>
        </button>
        <button type="button" class="act danger" title={$t("canvas.term.action.close")} aria-label={$t("canvas.term.action.close")} onclick={closeTerminal}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12 M18 6L6 18" /></svg>
        </button>
      </div>
    {/if}
    {#if live.progress !== null}
      <div class="progress" class:indeterminate={live.progress < 0} role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={live.progress < 0 ? undefined : live.progress}>
        <div class="bar" style={progressStyle(live.progress)}></div>
      </div>
    {/if}
  </header>

  {#if chatFace}
    <div class="chat-face">
      <ChatFace
        {node}
        workspaceId={workspaceId || engine.store.workspaceId}
        {focused}
        {termId}
        onshell={() => chats.setFace(node.id, false)}
        ondata={(d) => engine.commit([{ op: "update_node", id: node.id, patch: { data: { ...node.data, ...d } } }], { history: false })}
      />
    </div>
  {/if}
  {#if !useGl}
    <div class="fallback">
      {#if fallbackReady}
      <TerminalView
        id={termId}
        {cwd}
        {command}
        env={canvasEnv()}
        fontSize={13}
        {focused}
        onFocus={() => onfocusrequest?.()}
        onExit={(code, signal) => (live.exited = { code, signal })}
      />
      {/if}
    </div>
  {:else}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="body" bind:this={body}>
      <textarea
        bind:this={ta}
        class="ime"
        aria-label={$t("canvas.term.input_label")}
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        tabindex="-1"
        style="left: {caret.x}px; top: {caret.y}px; height: {caret.h}px; font-size: {caret.font}px; line-height: {caret.h}px;"
      ></textarea>
      {#if findOpen && !card}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="find" onpointerdown={(e) => e.stopPropagation()}>
          <input
            bind:this={findInput}
            bind:value={findQuery}
            type="text"
            placeholder={$t("canvas.term.find.placeholder")}
            aria-label={$t("canvas.term.find.placeholder")}
            spellcheck="false"
            autocomplete="off"
            oninput={onFindInput}
            onkeydown={onFindKey}
          />
          <span class="find-count" class:miss={findMiss} aria-live="polite">
            {#if findCount !== null}
              {findCount === 0 ? $t("canvas.term.find.none") : $t("canvas.term.find.count", { count: findCount })}
            {/if}
          </span>
          <button type="button" class="act" title={$t("canvas.term.find.prev")} aria-label={$t("canvas.term.find.prev")} onclick={() => findStep(true)}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 15l6-6 6 6" /></svg>
          </button>
          <button type="button" class="act" title={$t("canvas.term.find.next")} aria-label={$t("canvas.term.find.next")} onclick={() => findStep(false)}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
          </button>
          <button type="button" class="act" title={$t("canvas.term.find.close")} aria-label={$t("canvas.term.find.close")} onclick={closeFind}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12 M18 6L6 18" /></svg>
          </button>
        </div>
      {/if}
      {#if card}
        <div class="card-face">
          <span class="card-sym" aria-hidden="true">{SYMBOLS[agentState]}</span>
          <span class="card-state">{$t(`canvas.term.state.${agentState}`)}</span>
          {#if live.agentMessage}
            <span class="card-msg">{live.agentMessage}</span>
          {/if}
        </div>
      {/if}
      {#if live.error}
        <div class="overlay error" role="alert">
          <span>{$t("canvas.term.error", { error: live.error })}</span>
          <button type="button" onclick={() => open(true)}>{$t("canvas.term.action.retry")}</button>
        </div>
      {:else if live.exited && !card}
        <div class="overlay">
          <button type="button" onclick={restart}>{$t("canvas.term.action.restart")}</button>
        </div>
      {:else if opening && !termHub.grid(termId)}
        <div class="overlay muted">{$t("canvas.term.starting")}</div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .term-node {
    position: absolute;
    inset: 0;
    border-radius: 10px;
    border: 1px solid color-mix(in srgb, var(--text, #fff) 12%, transparent);
    background: transparent;
    box-shadow: 0 6px 24px rgb(0 0 0 / 0.25);
    overflow: hidden;
    color: var(--text, #f5f5f7);
  }
  .term-node.selected {
    border-color: var(--accent, #ff9f0a);
  }
  .term-node.focused {
    border-color: var(--state);
    box-shadow:
      0 0 0 1px var(--state),
      0 8px 28px rgb(0 0 0 / 0.3);
  }
  .chrome {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: var(--header-h);
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px 0 10px;
    background: color-mix(in srgb, var(--pane-bg, #1c1c1e) 92%, var(--state) 8%);
    border-bottom: 1px solid color-mix(in srgb, var(--text, #fff) 8%, transparent);
    font-size: 12px;
    user-select: none;
  }
  .badge {
    flex: none;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--state);
    color: #111;
    font-size: 11px;
    font-weight: 700;
    line-height: 1;
  }
  .badge.pulse {
    animation: pulse 1.6s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      box-shadow: 0 0 0 4px color-mix(in srgb, var(--state) 30%, transparent);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .badge.pulse {
      animation: none;
    }
  }
  .title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }
  .exit-tag {
    flex: none;
    font-size: 11px;
    color: var(--text-dim, #98989d);
  }
  .spacer {
    flex: 1;
  }
  .actions {
    display: flex;
    gap: 2px;
  }
  .act {
    width: 22px;
    height: 22px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--text-dim, #98989d);
    display: grid;
    place-items: center;
    cursor: pointer;
    padding: 0;
  }
  .chat-btn {
    position: relative;
  }
  .chat-unread {
    position: absolute;
    top: -4px;
    right: -5px;
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
  .act:hover {
    background: color-mix(in srgb, var(--text, #fff) 10%, transparent);
    color: var(--text, #f5f5f7);
  }
  .act.danger:hover {
    color: var(--error, #ff453a);
  }
  .act svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .progress {
    position: absolute;
    left: 0;
    right: 0;
    bottom: -1px;
    height: 2px;
    overflow: hidden;
  }
  .progress .bar {
    height: 100%;
    background: var(--state);
    transition: width 0.3s ease;
  }
  .progress.indeterminate .bar {
    width: 30%;
    animation: slide 1.2s ease-in-out infinite;
  }
  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(340%);
    }
  }
  .body {
    position: absolute;
    left: var(--pad);
    right: var(--pad);
    top: var(--header-h);
    bottom: var(--pad);
    cursor: text;
    touch-action: none;
  }
  .term-node.card .body {
    cursor: default;
  }
  .ime {
    position: absolute;
    width: 2px;
    padding: 0;
    margin: 0;
    border: 0;
    outline: none;
    resize: none;
    overflow: hidden;
    white-space: nowrap;
    background: transparent;
    color: transparent;
    caret-color: transparent;
    opacity: 0;
    z-index: 2;
  }
  .ime:global(.composing) {
    width: auto;
    min-width: 2px;
    opacity: 1;
    color: var(--text, #f5f5f7);
    background: var(--pane-bg, #1c1c1e);
    text-decoration: underline;
    font-family: var(--font-mono, ui-monospace, Menlo, monospace);
    field-sizing: content;
  }
  .card-face {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    background: color-mix(in srgb, var(--pane-bg, #1c1c1e) 94%, var(--state) 6%);
    text-align: center;
    padding: 12px;
  }
  .card-sym {
    font-size: 48px;
    line-height: 1;
    color: var(--state);
  }
  .card-state {
    font-size: 22px;
    font-weight: 600;
  }
  .card-msg {
    font-size: 16px;
    color: var(--text-dim, #98989d);
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
  }
  .overlay {
    position: absolute;
    left: 50%;
    bottom: 12px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--pane-bg, #1c1c1e) 90%, transparent);
    border: 1px solid color-mix(in srgb, var(--text, #fff) 12%, transparent);
    font-size: 12px;
    z-index: 3;
  }
  .overlay.error {
    color: var(--error, #ff453a);
  }
  .overlay.muted {
    color: var(--text-dim, #98989d);
  }
  .overlay button {
    border: 0;
    border-radius: 6px;
    padding: 3px 8px;
    background: var(--accent, #ff9f0a);
    color: #111;
    font-size: 12px;
    cursor: pointer;
  }
  .role {
    flex: none;
    display: grid;
    place-items: center;
  }
  .find {
    position: absolute;
    top: 6px;
    right: 8px;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px;
    border-radius: 8px;
    background: var(--pane-bg, #1c1c1e);
    border: 1px solid color-mix(in srgb, var(--text, #fff) 16%, transparent);
    box-shadow: 0 4px 14px rgb(0 0 0 / 0.25);
    font-size: 12px;
  }
  .find input {
    width: 160px;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text, #f5f5f7);
    font: inherit;
  }
  .find-count {
    min-width: 56px;
    color: var(--text-dim, #98989d);
    font-variant-numeric: tabular-nums;
  }
  .find-count.miss {
    color: var(--error, #ff453a);
  }
  .lock-tag {
    flex: none;
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--accent, #ff9f0a);
    cursor: pointer;
  }
  .lock-tag svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .chat-face {
    position: absolute;
    left: 0;
    right: 0;
    top: var(--header-h);
    bottom: 0;
    z-index: 4;
    background: var(--pane-bg, #1c1c1e);
  }
  .fallback {
    position: absolute;
    left: 0;
    right: 0;
    top: var(--header-h);
    bottom: 0;
    background: var(--pane-bg, #1c1c1e);
  }
</style>
