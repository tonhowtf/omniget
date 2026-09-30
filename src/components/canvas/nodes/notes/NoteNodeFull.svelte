<script lang="ts">
  // Note node (study 95 §G): a real .md file with Formatted and Raw modes,
  // live Markdown render, Lock Contents (agent writes fail), colours, name
  // from the first line unless pinned (Rust updates the title on write),
  // conditional saves with `ifRevision` and a conflict bar when someone else
  // wrote meanwhile, live reload on `canvas://note`, and "written by <agent>"
  // when the last change came from an agent. External notes (.md/.markdown/
  // .txt dropped from Finder) stay where they are.
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { renderSafeMarkdownSync } from "$lib/llm/markdown";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import type { CanvasNode } from "$lib/canvas/model/types";
  import {
    NOTE_COLORS,
    NoteConflict,
    NoteLocked,
    hasFile,
    noteColorCss,
    noteData,
    noteRead,
    noteMove,
    noteWrite,
    onNoteEvent,
    stackIdOf,
  } from "$lib/canvas/notes/api";
  import { ensureStackController } from "$lib/canvas/notes/stack";
  import { canvasPrefs } from "$lib/canvas/prefs.svelte";
  import { richMarkdown } from "$lib/canvas/notes/rich";
  import { dirOf, openPathOnCanvas, workspaceDir } from "$lib/canvas/notes/rich/open";
  import { attachmentSave, blobToBase64 } from "$lib/canvas/chat/client";

  let {
    node,
    selected = false,
    focused = false,
    embedded = false,
    onpatch,
    onexit,
    onfocusrequest,
  }: {
    node: CanvasNode;
    selected?: boolean;
    focused?: boolean;
    /** Rendered by a fichário (front page), not on its own. */
    embedded?: boolean;
    onpatch?: (patch: Partial<CanvasNode>) => void;
    onexit?: () => void;
    onfocusrequest?: () => void;
  } = $props();

  const engine = useEngine();
  ensureStackController(engine);

  const SAVE_MS = 600;
  const AGENT_BADGE_MS = 5 * 60_000;

  const cache = new Map<string, string>();
  let bump = $state(0);
  /** Text of the file as last read or written. */
  let text = $state("");
  let revision = $state<string | undefined>(undefined);
  let draft = $state("");
  let dirty = $state(false);
  let saving = $state(false);
  let loaded = $state(false);
  let fileBacked = $state(true);
  let error = $state<string | null>(null);
  let conflict = $state<{ text: string; revision: string } | null>(null);
  let writer = $state<{ name: string; at: number } | null>(null);
  // New Item Defaults (Settings → Canvas) may make a note start in Raw.
  let raw = $state(untrack(() => node.data.mode === "raw"));
  let colorsOpen = $state(false);
  let area = $state<HTMLTextAreaElement | null>(null);
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let now = $state(Date.now());
  let clock: ReturnType<typeof setInterval> | null = null;

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) });

  const data = $derived(noteData(node));
  const locked = $derived(!!data.isContentLocked);
  const tint = $derived(noteColorCss(data.customColor));
  const fileKey = $derived(`${data.fileName ?? ""}|${data.path ?? ""}`);
  const hidden = $derived(!embedded && !!stackIdOf(node));
  const shown = $derived(dirty || raw ? draft : text);
  const html = $derived.by(() => {
    void bump;
    return renderSafeMarkdownSync(shown, cache, () => bump++, { images: true });
  });
  const editing = $derived(focused && raw && !locked);

  function ids() {
    return { ws: engine.store.workspaceId, floor: engine.store.floorId };
  }

  async function load() {
    error = null;
    if (!hasFile(node) || engine.syncMode === "memory") {
      // Memory mode or a placeholder note: the text lives in the node.
      fileBacked = false;
      text = String(node.data.text ?? "");
      if (!dirty) draft = text;
      loaded = true;
      return;
    }
    fileBacked = true;
    const { ws, floor } = ids();
    try {
      const c = await noteRead(ws, floor, node.id);
      // A note created with inline text (older placeholder): move it into the file.
      const legacy = String(node.data.text ?? "");
      if (!c.text && legacy) {
        const w = await noteWrite(ws, floor, node.id, legacy, c.revision).catch(() => null);
        if (w) {
          text = w.text;
          revision = w.revision;
          onpatch?.({ data: { ...node.data, text: null } });
        } else {
          text = legacy;
          revision = c.revision;
        }
      } else {
        text = c.text;
        revision = c.revision;
      }
      if (!dirty) draft = text;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (msg.startsWith("CANVAS_")) {
        error = msg;
      } else {
        // Notes API not available (command missing): edit the inline text.
        fileBacked = false;
        text = String(node.data.text ?? "");
        if (!dirty) draft = text;
      }
    } finally {
      loaded = true;
    }
  }

  // (Re)load when the backing file changes identity.
  $effect(() => {
    void fileKey;
    untrack(() => void load());
  });

  // Memory-mode notes follow node.data.text when not being edited.
  $effect(() => {
    const inline = String(node.data.text ?? "");
    untrack(() => {
      if (!fileBacked && !dirty) {
        text = inline;
        draft = inline;
      }
    });
  });

  // Entering focus = Raw editing (Maestri double-click), leaving = Formatted.
  $effect(() => {
    if (focused) {
      untrack(() => {
        if (!locked) raw = true;
        if (!dirty) draft = text;
      });
      void tick().then(() => area?.focus());
    } else {
      untrack(() => {
        void flush();
        // Back to the note's own face: Raw stays Raw (New Item Defaults).
        raw = node.data.mode === "raw" && !locked;
        colorsOpen = false;
      });
    }
  });

  // The mode changed from outside (MCP, another window): follow it while
  // the note is not being edited.
  $effect(() => {
    const wantRaw = node.data.mode === "raw";
    untrack(() => {
      if (!focused && !locked) raw = wantRaw;
    });
  });

  function agentName(origin: string): string | null {
    if (!origin.startsWith("agent:")) return null;
    const ref = origin.slice(6);
    const n = engine.resolveNode(ref);
    return n?.title ?? tr("canvas.notes.anAgent", "an agent");
  }

  onMount(() => {
    const off = onNoteEvent(async (e) => {
      if (e.nodeId !== node.id || e.workspaceId !== engine.store.workspaceId) return;
      const name = agentName(e.origin);
      if (name) writer = { name, at: Date.now() };
      else if (e.origin === "user") writer = null;
      if (e.revision === revision) return;
      const { ws, floor } = ids();
      try {
        const c = await noteRead(ws, floor, node.id);
        if (dirty && c.text !== draft) {
          conflict = { text: c.text, revision: c.revision };
        } else {
          text = c.text;
          revision = c.revision;
          draft = c.text;
          dirty = false;
        }
      } catch {
        // Gone or unreadable: keep what we have.
      }
    });
    clock = setInterval(() => (now = Date.now()), 30_000);
    return () => {
      off();
      if (clock) clearInterval(clock);
    };
  });

  onDestroy(() => {
    void flush();
    cache.clear();
  });

  // ---- saving ----------------------------------------------------------

  function onInput() {
    dirty = draft !== text;
    if (saveTimer) clearTimeout(saveTimer);
    if (dirty) saveTimer = setTimeout(() => void flush(), SAVE_MS);
  }

  async function flush(force?: string) {
    if (saveTimer) {
      clearTimeout(saveTimer);
      saveTimer = null;
    }
    if (!dirty || saving || locked) return;
    if (conflict && !force) return;
    const next = draft;
    if (!fileBacked) {
      onpatch?.({ data: { ...node.data, text: next } });
      text = next;
      dirty = false;
      return;
    }
    saving = true;
    const { ws, floor } = ids();
    try {
      const c = await noteWrite(ws, floor, node.id, next, force ?? revision);
      revision = c.revision;
      text = c.text;
      conflict = null;
      writer = null;
      dirty = draft !== text;
    } catch (e) {
      if (e instanceof NoteConflict) {
        const cur = await noteRead(ws, floor, node.id).catch(() => null);
        conflict = { text: cur?.text ?? "", revision: cur?.revision ?? e.current };
      } else if (e instanceof NoteLocked) {
        error = tr("canvas.notes.lockedError", "This note is locked");
      } else {
        error = e instanceof Error ? e.message : String(e);
      }
    } finally {
      saving = false;
    }
    if (dirty && !conflict && !saveTimer) saveTimer = setTimeout(() => void flush(), SAVE_MS);
  }

  function keepMine() {
    const c = conflict;
    if (!c) return;
    conflict = null;
    dirty = true;
    void flush(c.revision);
  }

  function takeTheirs() {
    const c = conflict;
    if (!c) return;
    conflict = null;
    text = c.text;
    revision = c.revision;
    draft = c.text;
    dirty = false;
  }

  // ---- toolbar ---------------------------------------------------------

  function setData(changes: Record<string, unknown>) {
    onpatch?.({ data: { ...node.data, ...changes } });
  }

  async function toggleLock() {
    if (!locked) await flush();
    setData({ isContentLocked: locked ? null : true });
    if (!locked) raw = false;
  }

  let moveError = $state<string | null>(null);

  /** Maestri "Move to…": save the note as a file of the project; it becomes external there. */
  async function moveTo() {
    moveError = null;
    await flush();
    let dest: string | null = null;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const base = (node.title ?? "note").replace(/[\\/:*?"<>|]+/g, "-").trim() || "note";
      dest = await save({
        title: tr("canvas.notes.moveTo", "Move to…"),
        defaultPath: `${base}.md`,
        filters: [{ name: "Markdown", extensions: ["md", "markdown", "txt"] }],
      });
    } catch {
      dest = null;
    }
    if (!dest) return;
    const { ws, floor } = ids();
    try {
      const c = await noteMove(ws, floor, node.id, dest);
      text = c.text;
      revision = c.revision;
      if (!dirty) draft = c.text;
    } catch (e) {
      moveError = e instanceof Error ? e.message : String(e);
    }
  }

  function togglePinName() {
    if (data.hasCustomName) setData({ hasCustomName: null });
    else setData({ hasCustomName: true });
  }

  function pickColor(id: string | null) {
    colorsOpen = false;
    setData({ customColor: id });
  }

  // ---- rich Markdown (Mermaid, code, tables, images, paths) --------------

  const richBase = $derived(data.isExternal && data.path ? dirOf(data.path) : workspaceDir(engine));

  function openPath(abs: string, isDir: boolean, line?: number) {
    openPathOnCanvas(engine, node, abs, isDir, line);
  }

  /** Pasting an image stores it with the workspace and writes `![](path)`. */
  async function onPaste(e: ClipboardEvent) {
    const files = Array.from(e.clipboardData?.files ?? []).filter((f) => f.type.startsWith("image/"));
    if (!files.length) return;
    e.preventDefault();
    const el = area;
    let at = el?.selectionStart ?? draft.length;
    const end = el?.selectionEnd ?? at;
    const parts: string[] = [];
    for (const f of files) {
      try {
        const ext = (f.type.split("/")[1] || "png").replace("jpeg", "jpg").replace(/\+.*$/, "");
        const name = f.name && f.name !== "image.png" ? f.name : `pasted-${Date.now()}.${ext}`;
        const path = await attachmentSave(engine.store.workspaceId, name, await blobToBase64(f));
        parts.push(`![${name.replace(/[[\]]/g, "")}](<${path}>)`);
      } catch (err) {
        error = err instanceof Error ? err.message : String(err);
      }
    }
    if (!parts.length) return;
    const ins = parts.join("\n");
    draft = draft.slice(0, at) + ins + draft.slice(end);
    at += ins.length;
    onInput();
    await tick();
    el?.setSelectionRange(at, at);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && !e.shiftKey) {
      e.preventDefault();
      void flush();
      onexit?.();
    } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      e.stopPropagation();
      void flush();
    }
  }

  function ago(at: number): string {
    const s = Math.max(0, Math.round((now - at) / 1000));
    if (s < 60) return tr("canvas.notes.justNow", "just now");
    return tr("canvas.notes.minutesAgo", "{{count}} min ago", { count: Math.round(s / 60) });
  }

  // Notes hold code and paths: system autocorrect/capitalization stay off
  // (Maestri), spell check follows Settings → Canvas.
  const noAutocorrect: Record<string, string> = { autocorrect: "off", autocapitalize: "off" };

  const showWriter = $derived(!!writer && now - writer.at < AGENT_BADGE_MS);
  const showBar = $derived(focused || selected || embedded);
</script>

{#if hidden}
  <div class="stacked">{tr("canvas.notes.inStack", "In a fichário")}</div>
{:else}
  <div
    class="note"
    class:focused
    class:locked
    style:--note-tint={tint ?? "transparent"}
    style:font-family={typeof node.data.font === "string" && node.data.font ? node.data.font : null}
    class:tinted={!!tint}
  >
    {#if showBar}
      <div class="bar" data-canvas-chrome>
        <div class="seg" role="group" aria-label={tr("canvas.notes.mode", "View mode")}>
          <button
            type="button"
            class:on={!raw}
            onclick={() => {
              void flush();
              raw = false;
              if (node.data.mode === "raw") setData({ mode: null });
            }}>{tr("canvas.notes.formatted", "Formatted")}</button
          >
          <button
            type="button"
            class:on={raw}
            disabled={locked}
            onclick={() => {
              raw = true;
              if (node.data.mode !== "raw") setData({ mode: "raw" });
              if (!focused) onfocusrequest?.();
              void tick().then(() => area?.focus());
            }}>{tr("canvas.notes.raw", "Raw")}</button
          >
        </div>
        {#if showWriter && writer}
          <span class="writer" title={tr("canvas.notes.writtenBy", "Written by {{name}}", { name: writer.name })}>
            ✎ {tr("canvas.notes.writtenBy", "Written by {{name}}", { name: writer.name })} · {ago(writer.at)}
          </span>
        {:else if data.isExternal}
          <span class="ext" title={data.path}>{tr("canvas.notes.external", "External file")}</span>
        {:else}
          <span class="spacer"></span>
        {/if}
        {#if saving}<span class="status">{tr("canvas.notes.saving", "Saving…")}</span>{:else if dirty}<span class="status">•</span>{/if}
        <button
          type="button"
          class="tool"
          class:on={!!data.hasCustomName}
          title={data.hasCustomName ? tr("canvas.notes.unpinName", "Name follows the first line") : tr("canvas.notes.pinName", "Keep this name")}
          aria-pressed={!!data.hasCustomName}
          onclick={togglePinName}>{data.hasCustomName ? "📌" : "Aa"}</button
        >
        {#if fileBacked}
          <button
            type="button"
            class="tool"
            title={data.path ? tr("canvas.notes.moveToExternal", "Move to… (now at {{path}})", { path: data.path }) : tr("canvas.notes.moveTo", "Move to…")}
            aria-label={tr("canvas.notes.moveTo", "Move to…")}
            onclick={() => void moveTo()}>⇥</button
          >
        {/if}
        <div class="colors-wrap">
          <button
            type="button"
            class="tool dot-btn"
            title={tr("canvas.notes.color", "Color")}
            aria-expanded={colorsOpen}
            onclick={() => (colorsOpen = !colorsOpen)}
          >
            <span class="dot" style:background={tint ?? "var(--border)"}></span>
          </button>
          {#if colorsOpen}
            <div class="colors" role="listbox" aria-label={tr("canvas.notes.color", "Color")}>
              <button type="button" class="swatch none" aria-label={tr("canvas.notes.noColor", "No color")} onclick={() => pickColor(null)}></button>
              {#each NOTE_COLORS as c}
                <button
                  type="button"
                  class="swatch"
                  class:on={data.customColor === c.id}
                  style:background={c.css}
                  aria-label={c.id}
                  onclick={() => pickColor(c.id)}
                ></button>
              {/each}
            </div>
          {/if}
        </div>
        <button
          type="button"
          class="tool"
          class:on={locked}
          title={locked ? tr("canvas.notes.unlock", "Unlock contents") : tr("canvas.notes.lock", "Lock contents (agents cannot write)")}
          aria-pressed={locked}
          onclick={() => void toggleLock()}>{locked ? "🔒" : "🔓"}</button
        >
      </div>
    {/if}

    {#if conflict}
      <div class="conflict" data-canvas-chrome role="alert">
        <span>{tr("canvas.notes.conflict", "This note changed while you were editing.")}</span>
        <button type="button" onclick={takeTheirs}>{tr("canvas.notes.takeTheirs", "Use theirs")}</button>
        <button type="button" class="primary" onclick={keepMine}>{tr("canvas.notes.keepMine", "Keep mine")}</button>
      </div>
    {/if}
    {#if error}
      <div class="error" role="status">{error}</div>
    {/if}
    {#if moveError}
      <div class="error" role="status">{moveError}</div>
    {/if}

    {#if editing}
      <textarea
        bind:this={area}
        bind:value={draft}
        oninput={onInput}
        onblur={() => void flush()}
        onkeydown={onKey}
        onpaste={(e) => void onPaste(e)}
        spellcheck={canvasPrefs().spell_check}
        {...noAutocorrect}
        placeholder={tr("canvas.notes.placeholder", "Write in Markdown…")}
      ></textarea>
    {:else if !loaded}
      <p class="empty">…</p>
    {:else if raw && shown.trim()}
      <!-- Raw face without focus (New Item Defaults render = raw). -->
      <pre class="raw-view" style:font-family={typeof node.data.font === "string" && node.data.font ? node.data.font : null}>{shown}</pre>
    {:else if shown.trim()}
      <div class="md" use:richMarkdown={{ baseDir: richBase, onOpenPath: openPath }}>{@html html}</div>
    {:else}
      <p class="empty">{locked ? tr("canvas.notes.emptyLocked", "Empty and locked") : tr("canvas.notes.empty", "Double-click to write")}</p>
    {/if}
  </div>
{/if}

<style>
  .note {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    font-size: 14px;
    line-height: 1.5;
    color: var(--text);
    background: transparent;
  }
  .note.tinted {
    background: color-mix(in srgb, var(--note-tint) 22%, transparent);
  }
  .stacked {
    height: 100%;
    display: grid;
    place-items: center;
    color: var(--text-muted);
    font-size: 12px;
  }
  .bar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    font-size: 11px;
    user-select: none;
  }
  .seg {
    display: flex;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .seg button {
    border: none;
    background: transparent;
    color: var(--text-muted);
    font: inherit;
    padding: 2px 7px;
    cursor: pointer;
  }
  .seg button.on {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--text);
  }
  .seg button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .spacer,
  .writer,
  .ext {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .writer {
    color: var(--accent);
  }
  .ext {
    color: var(--text-muted);
  }
  .status {
    color: var(--text-muted);
  }
  .tool {
    min-width: 22px;
    height: 20px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--text-muted);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    padding: 0 4px;
  }
  .tool:hover,
  .tool.on {
    border-color: var(--border);
    color: var(--text);
  }
  .colors-wrap {
    position: relative;
  }
  .dot-btn {
    display: grid;
    place-items: center;
  }
  .dot {
    width: 11px;
    height: 11px;
    border-radius: 50%;
    border: 1px solid var(--border);
  }
  .colors {
    position: absolute;
    right: 0;
    top: 24px;
    z-index: 5;
    display: grid;
    grid-template-columns: repeat(5, 18px);
    gap: 5px;
    padding: 7px;
    border-radius: 9px;
    border: 1px solid var(--border);
    background: var(--popup-bg, var(--surface));
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
  }
  .swatch {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    padding: 0;
  }
  .swatch.on {
    border-color: var(--text);
  }
  .swatch.none {
    background: linear-gradient(135deg, transparent 45%, var(--danger, #d96a62) 45% 55%, transparent 55%);
    border-color: var(--border);
  }
  .conflict {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    font-size: 12px;
    background: color-mix(in srgb, var(--warning, #dcab3c) 20%, transparent);
  }
  .conflict span {
    flex: 1;
  }
  .conflict button {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 5px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .conflict button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent, #fff);
  }
  .error {
    flex: none;
    padding: 4px 8px;
    font-size: 11px;
    color: var(--danger, #d96a62);
  }
  .md {
    flex: 1;
    overflow: auto;
    padding: 10px 12px;
    user-select: text;
  }
  .md :global(h1),
  .md :global(h2),
  .md :global(h3) {
    margin: 0.4em 0 0.3em;
    line-height: 1.25;
  }
  .md :global(p) {
    margin: 0 0 0.6em;
  }
  .md :global(ul),
  .md :global(ol) {
    margin: 0 0 0.6em;
    padding-left: 1.3em;
  }
  .md :global(img) {
    max-width: 100%;
  }
  .md :global(table) {
    border-collapse: collapse;
    margin: 0 0 0.6em;
  }
  .md :global(th),
  .md :global(td) {
    border: 1px solid var(--border);
    padding: 3px 6px;
  }
  .md :global(pre) {
    background: var(--fill-1, rgba(127, 127, 127, 0.12));
    padding: 8px;
    border-radius: var(--radius-sm, 6px);
    overflow: auto;
  }
  .md :global(code) {
    font-family: var(--font-mono, monospace);
    font-size: 0.92em;
  }
  .raw-view {
    flex: 1;
    margin: 0;
    overflow: auto;
    padding: 10px 12px;
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
    font: 13px/1.5 var(--font-mono, monospace);
  }
  textarea {
    flex: 1;
    width: 100%;
    resize: none;
    border: none;
    outline: none;
    padding: 10px 12px;
    box-sizing: border-box;
    background: transparent;
    color: inherit;
    font: 13px/1.5 var(--font-mono, monospace);
  }
  .empty {
    color: var(--text-muted);
    margin: 0;
    padding: 10px 12px;
  }
</style>
