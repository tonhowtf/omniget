<script lang="ts">
  // Prompt Composer (Maestri H): ⌘⇧P floats a rich input over the focused
  // terminal node and follows the focus. `@` mentions what the terminal is
  // connected to (expanded into tags naming the skill to load) plus File…,
  // Maestro, actions, floors and workspaces; `#` where a mention could start
  // opens an inline picker of the terminal folder's files and inserts the
  // relative path ("issue #42" stays text); pasted screenshots are saved to <ws>/inbox/attachments and go in as
  // paths, large pastes fold into pills. One draft per terminal. With the
  // composer empty, arrows, Return and Tab go to the terminal. Send = bracketed
  // paste + Enter into the terminal.
  // Mounted once by Canvas.svelte inside the screen-space layer.
  import { canvasPrefs } from "$lib/canvas/prefs.svelte";
  import { onDestroy, onMount, tick } from "svelte";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { terminalIdOf, type CanvasNode } from "$lib/canvas/model/types";
  import { termKey, termPaste } from "$lib/canvas/term/client";
  import { attachmentSave, blobToBase64 } from "$lib/canvas/chat/client";
  import { termLive } from "$lib/canvas/term/state.svelte";
  import { filesList, filesSearch } from "$lib/canvas/files/api";
  import { envDrop, envFiles } from "$lib/canvas/floors/api";
  import { workspaces } from "$lib/canvas/palette/workspaces.svelte";
  import {
    expand,
    fileQuery,
    filterTargets,
    pastePill,
    loadDraft,
    mentionQuery,
    mentionTargets,
    mentionToken,
    pathToken,
    pillLabel,
    prunePastes,
    saveDraft,
    type Draft,
    type MentionTarget,
  } from "$lib/canvas/chat/composer";

  const engine = useEngine();

  let open = $state(false);
  let nodeId = $state<string | null>(null);
  let draft = $state<Draft>({ text: "", pastes: [] });
  let ta = $state<HTMLTextAreaElement | null>(null);
  let rect = $state({ x: 0, y: 0, w: 0, h: 0 });
  let dock = $state<"float" | "below">(loadDock());
  let menu = $state<{ start: number; query: string } | null>(null);
  let active = $state(0);
  let fileMenu = $state<{ start: number; query: string } | null>(null);
  let fileHits = $state<string[]>([]);
  let fileRoot = $state("");
  let fileTimer: ReturnType<typeof setTimeout> | null = null;
  let fileSeq = 0;
  let busy = $state(false);
  let error = $state("");

  const workspaceId = $derived(engine.store.workspaceId);
  const node = $derived.by((): CanvasNode | undefined => {
    void engine.store.version;
    return nodeId ? engine.store.node(nodeId) : undefined;
  });
  const targets = $derived.by((): MentionTarget[] => {
    void engine.store.structure;
    return nodeId
      ? mentionTargets(nodeId, engine.store.nodes, engine.store.connections, {
          workspaceId: engine.store.workspaceId,
          floorId: engine.store.floorId,
          workspaces: workspaces.list,
        })
      : [];
  });
  const options = $derived(menu ? filterTargets(targets, menu.query).slice(0, 12) : []);
  const connectedCount = $derived(targets.filter((x) => x.id.startsWith("remote:") || !x.id.includes(":")).length);

  // The MCP driver pastes through this event (a synthetic ClipboardEvent
  // carries no data in WebKit); it runs the same pill logic as a real paste.
  $effect(() => {
    const el = ta;
    if (!el) return;
    el.addEventListener("omniget:paste", onSyntheticPaste);
    return () => el.removeEventListener("omniget:paste", onSyntheticPaste);
  });
  const name = $derived(node?.title || $t("canvas.composer.terminal", { default: "terminal" }));

  function loadDock(): "float" | "below" {
    try {
      return localStorage.getItem("omniget.canvas.composer.dock") === "below" ? "below" : "float";
    } catch {
      return "float";
    }
  }

  function setDock(d: "float" | "below") {
    dock = d;
    try {
      localStorage.setItem("omniget.canvas.composer.dock", d);
    } catch {
      // storage unavailable
    }
    place();
  }

  function isTerminal(id: string | null): id is string {
    if (!id) return false;
    const n = engine.store.node(id);
    return !!n && (n.kind === "terminal" || n.kind === "agent");
  }

  function persist() {
    if (nodeId && workspaceId) saveDraft(workspaceId, nodeId, prunePastes(draft));
  }

  function attach(id: string) {
    if (nodeId === id) return;
    persist();
    nodeId = id;
    draft = workspaceId ? loadDraft(workspaceId, id) : { text: "", pastes: [] };
    menu = null;
    error = "";
    place();
  }

  async function show() {
    const f = engine.selection.focused;
    const target = isTerminal(f) ? f : isTerminal(nodeId) ? nodeId : null;
    if (!target) return;
    attach(target);
    open = true;
    await tick();
    ta?.focus();
    const end = draft.text.length;
    ta?.setSelectionRange(end, end);
  }

  function hide(refocusTerminal = true) {
    persist();
    open = false;
    menu = null;
    if (refocusTerminal && nodeId) engine.selection.focus(nodeId);
  }

  // Follow the focus to another terminal while open (Maestri).
  $effect(() => {
    const f = engine.selection.focused;
    if (open && isTerminal(f) && f !== nodeId) attach(f);
  });

  function place() {
    const n = nodeId ? engine.store.node(nodeId) : undefined;
    if (!n) return;
    const r = engine.camera.rectToScreen({ x: n.x, y: n.y, w: n.w, h: n.h });
    const w = Math.max(320, Math.min(720, r.w - 24));
    const x = Math.min(Math.max(8, r.x + (r.w - w) / 2), Math.max(8, engine.camera.vw - w - 8));
    const h = 120;
    const y =
      dock === "below"
        ? Math.min(r.y + r.h + 8, engine.camera.vh - h - 8)
        : Math.min(Math.max(8, r.y + r.h - h - 16), engine.camera.vh - h - 8);
    if (x !== rect.x || y !== rect.y || w !== rect.w) rect = { x, y, w, h };
  }

  let offFrame: (() => void) | null = null;
  const onWindowKey = (e: KeyboardEvent) => {
    const primary = e.metaKey || e.ctrlKey;
    if (primary && e.shiftKey && !e.altKey && e.key.toLowerCase() === "p") {
      if (!open && !isTerminal(engine.selection.focused)) return;
      e.preventDefault();
      e.stopPropagation();
      if (open) hide();
      else void show();
    }
  };

  let offWs: (() => void) | null = null;
  onMount(() => {
    offFrame = engine.onFrame(() => {
      if (open) place();
    });
    window.addEventListener("keydown", onWindowKey, true);
    offWs = workspaces.use();
  });

  onDestroy(() => {
    persist();
    offWs?.();
    if (fileTimer) clearTimeout(fileTimer);
    offFrame?.();
    window.removeEventListener("keydown", onWindowKey, true);
  });

  function updateMenu() {
    if (!ta) return;
    const caret = ta.selectionStart ?? draft.text.length;
    const q = mentionQuery(draft.text, caret);
    menu = q;
    const f = q ? null : fileQuery(draft.text, caret);
    const changed = f?.query !== fileMenu?.query || !!f !== !!fileMenu;
    fileMenu = f;
    active = 0;
    if (changed) loadFiles();
  }

  /** Folder the `#` picker lists: the terminal's live cwd, its launch cwd, or the workspace folder. */
  function terminalDir(): string[] {
    const n = node;
    const out: string[] = [];
    if (n) {
      const live = termLive(terminalIdOf(n)).cwd;
      if (live) out.push(live.replace(/^file:\/\/[^/]*/, ""));
      if (typeof n.data?.cwd === "string" && n.data.cwd) out.push(n.data.cwd);
    }
    const wd = workspaces.byId(engine.store.workspaceId)?.workingDirectory;
    if (wd) out.push(wd);
    return [...new Set(out)];
  }

  function loadFiles() {
    if (fileTimer) clearTimeout(fileTimer);
    const f = fileMenu;
    if (!f || !workspaceId) {
      fileHits = [];
      return;
    }
    const seq = ++fileSeq;
    fileTimer = setTimeout(async () => {
      // A terminal in SSH/Docker/runtime: list the files over there (Maestri).
      if (nodeId) {
        try {
          const live = node ? termLive(terminalIdOf(node)).cwd?.replace(/^file:\/\/[^/]*/, "") : undefined;
          const r = await envFiles(nodeId, live || null, f.query);
          if (seq !== fileSeq) return;
          if (r.remote) {
            const q = f.query.toLowerCase();
            const hits = (r.hits ?? []).map((h) => (h.isDir ? `${h.path}/` : h.path));
            hits.sort((a, b) => {
              const an = (a.split("/").filter(Boolean).pop() ?? a).toLowerCase().startsWith(q) ? 0 : 1;
              const bn = (b.split("/").filter(Boolean).pop() ?? b).toLowerCase().startsWith(q) ? 0 : 1;
              return an - bn || a.length - b.length || a.localeCompare(b);
            });
            fileHits = hits.slice(0, 12);
            fileRoot = `${r.env ?? ""} ${r.root ?? ""}`.trim();
            return;
          }
        } catch {
          // The environment did not answer: fall back to this machine.
        }
      }
      for (const dir of terminalDir()) {
        try {
          const hits = f.query.trim()
            ? (await filesSearch(workspaceId, dir, f.query)).map((h) => (h.isDir ? `${h.path}/` : h.path))
            : (await filesList(workspaceId, dir, "")).map((e) => (e.isDir ? `${e.path}/` : e.path));
          if (seq !== fileSeq) return;
          const q = f.query.toLowerCase();
          // Name matches first, then shorter paths.
          hits.sort((a, b) => {
            const an = (a.split("/").filter(Boolean).pop() ?? a).toLowerCase().startsWith(q) ? 0 : 1;
            const bn = (b.split("/").filter(Boolean).pop() ?? b).toLowerCase().startsWith(q) ? 0 : 1;
            return an - bn || a.length - b.length || a.localeCompare(b);
          });
          fileHits = hits.slice(0, 12);
          fileRoot = dir;
          return;
        } catch {
          // Not a root of this workspace: try the next folder.
        }
      }
      if (seq === fileSeq) fileHits = [];
    }, 90);
  }

  function pickPath(rel: string) {
    if (!fileMenu || !ta) return;
    const caret = ta.selectionStart ?? draft.text.length;
    const isDir = rel.endsWith("/");
    if (isDir) {
      // Folders drill in: keep the picker open under the folder.
      insertAt(fileMenu.start, caret, `#${rel}`);
      void tick().then(updateMenu);
      return;
    }
    insertAt(fileMenu.start, caret, pathToken(rel) + " ");
    fileMenu = null;
    fileHits = [];
  }

  function insertAt(start: number, end: number, s: string) {
    const text = draft.text.slice(0, start) + s + draft.text.slice(end);
    draft = { ...draft, text };
    persist();
    const caret = start + s.length;
    void tick().then(() => {
      ta?.focus();
      ta?.setSelectionRange(caret, caret);
    });
  }

  function insert(s: string) {
    const start = ta?.selectionStart ?? draft.text.length;
    const end = ta?.selectionEnd ?? start;
    const pad = start > 0 && !/\s$/.test(draft.text.slice(0, start)) ? " " : "";
    insertAt(start, end, pad + s + " ");
  }

  function pick(target: MentionTarget) {
    if (!menu || !ta) return;
    const caret = ta.selectionStart ?? draft.text.length;
    if (target.special === "file") {
      // File… = the `#` picker in place of the `@query`.
      insertAt(menu.start, caret, "#");
      menu = null;
      void tick().then(updateMenu);
      return;
    }
    insertAt(menu.start, caret, mentionToken(target) + " ");
    menu = null;
  }

  async function pickFile() {
    try {
      const { open: openDialog } = await import("@tauri-apps/plugin-dialog");
      const res = await openDialog({ multiple: true, directory: false });
      const paths = Array.isArray(res) ? res : res ? [res] : [];
      for (const p of paths) insert(pathToken(String(p)));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function send() {
    if (!nodeId || busy) return;
    const text = expand(draft, targets).trim();
    if (!text) return;
    const termId = node ? terminalIdOf(node) : nodeId;
    busy = true;
    error = "";
    try {
      await termPaste(termId, text);
      await new Promise((r) => setTimeout(r, 120));
      await termKey(termId, { key: "Enter", code: "Enter" });
      draft = { text: "", pastes: [] };
      persist();
      hide();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  function passToTerminal(e: KeyboardEvent): boolean {
    if (draft.text || !nodeId || !node) return false;
    if (!["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Enter", "Tab"].includes(e.key)) return false;
    e.preventDefault();
    void termKey(terminalIdOf(node), { key: e.key, code: e.code, mods: (e.shiftKey ? 1 : 0) | (e.altKey ? 2 : 0) });
    return true;
  }

  function onKey(e: KeyboardEvent) {
    if (e.isComposing) return;
    if (fileMenu && fileHits.length) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        active = (active + 1) % fileHits.length;
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        active = (active - 1 + fileHits.length) % fileHits.length;
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        pickPath(fileHits[active]);
        return;
      }
    }
    if (e.key === "Escape" && fileMenu) {
      e.preventDefault();
      e.stopPropagation();
      fileMenu = null;
      fileHits = [];
      return;
    }
    if (menu && options.length) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        active = (active + 1) % options.length;
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        active = (active - 1 + options.length) % options.length;
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        pick(options[active]);
        return;
      }
    }
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      if (menu) menu = null;
      else hide();
      return;
    }
    if (passToTerminal(e)) return;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
      return;
    }
  }

  /** Text paste (the real `paste` event and the MCP `composerPaste` both land here). */
  function pasteText(text: string): boolean {
    const r = pastePill(draft, text);
    if (!r) return false;
    draft = r.draft;
    insert(r.label);
    return true;
  }

  function onSyntheticPaste(e: Event) {
    const text = (e as CustomEvent<{ text?: string }>).detail?.text ?? "";
    if (!pasteText(text)) insert(text);
  }

  async function onPaste(e: ClipboardEvent) {
    const cd = e.clipboardData;
    if (!cd || !workspaceId) return;
    const files = Array.from(cd.items).filter((i) => i.kind === "file");
    if (files.length) {
      e.preventDefault();
      for (const it of files) {
        const f = it.getAsFile();
        if (!f) continue;
        try {
          const ext = f.type.split("/")[1] || "png";
          let path = await attachmentSave(workspaceId, f.name || `screenshot.${ext}`, await blobToBase64(f));
          // On a remote terminal the image goes to /tmp/omniget-drops there.
          if (nodeId) path = (await envDrop(nodeId, [path]))[0] ?? path;
          insert(pathToken(path));
        } catch (err) {
          error = err instanceof Error ? err.message : String(err);
        }
      }
      return;
    }
    const text = cd.getData("text/plain");
    if (text && pasteText(text)) e.preventDefault();
  }

  function onInput() {
    draft = prunePastes(draft);
    persist();
    updateMenu();
  }

  const KIND_LABEL: Record<string, string> = {
    agent: "agent",
    note: "note",
    portal: "portal",
    file: "file",
    action: "action",
    floor: "floor",
    workspace: "workspace",
  };
</script>

{#if open && node}
  <div
    class="composer"
    class:below={dock === "below"}
    data-canvas-chrome
    style:left="{rect.x}px"
    style:top="{rect.y}px"
    style:width="{rect.w}px"
  >
    <div class="bar">
      <span class="to">{$t("canvas.composer.to", { default: "To {{name}}", name })}</span>
      <span class="spacer"></span>
      <button
        type="button"
        class="tool"
        title={dock === "below" ? $t("canvas.composer.float", { default: "Float over the terminal" }) : $t("canvas.composer.below", { default: "Dock below the terminal" })}
        onclick={() => setDock(dock === "below" ? "float" : "below")}
      >
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          {#if dock === "below"}<path d="M4 4h16v10H4z M8 18h8" />{:else}<path d="M4 4h16v16H4z M4 14h16" />{/if}
        </svg>
      </button>
      <button type="button" class="tool" title={$t("canvas.composer.file", { default: "Attach a file (#)" })} onclick={pickFile}>
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 11l-8 8a5 5 0 0 1-7-7l8-8a3.5 3.5 0 0 1 5 5l-8 8a2 2 0 0 1-3-3l7-7" /></svg>
      </button>
      <button type="button" class="tool" title={$t("canvas.composer.close", { default: "Close (Esc)" })} onclick={() => hide()}>
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
      </button>
    </div>

    {#if fileMenu && fileHits.length}
      <div class="mentions files" role="listbox" aria-label={$t("canvas.composer.files", { default: "Files" })} title={fileRoot}>
        {#each fileHits as p, i (p)}
          <button type="button" class="opt" class:on={i === active} role="option" aria-selected={i === active} onmousedown={(e) => { e.preventDefault(); pickPath(p); }}>
            <span class="kind k-file">{p.endsWith("/") ? $t("canvas.composer.kind.folder", { default: "folder" }) : $t("canvas.composer.kind.file", { default: "file" })}</span>
            <span class="oname">{p}</span>
          </button>
        {/each}
      </div>
    {:else if menu && options.length}
      <div class="mentions" role="listbox" aria-label={$t("canvas.composer.mentions", { default: "Connected" })}>
        {#each options as o, i (o.id)}
          <button type="button" class="opt" class:on={i === active} role="option" aria-selected={i === active} onmousedown={(e) => { e.preventDefault(); pick(o); }}>
            <span class="kind k-{o.kind}">{$t(`canvas.composer.kind.${o.kind}`, { default: KIND_LABEL[o.kind] ?? o.kind })}</span>
            <span class="oname" style:color={o.color ?? null}>{o.name}</span>
            {#if o.detail}<span class="detail">{o.detail}</span>{/if}
          </button>
        {/each}
      </div>
    {:else if menu && !connectedCount && !menu.query.trim() && !options.length}
      <div class="mentions none">{$t("canvas.composer.no_connections", { default: "Nothing is connected to this terminal yet." })}</div>
    {/if}

    <textarea
      bind:this={ta}
      bind:value={draft.text}
      rows="3"
      spellcheck={canvasPrefs().spell_check}
      placeholder={$t("canvas.composer.placeholder", { default: "Write a prompt. @ mentions what is connected, # picks a file." })}
      onkeydown={onKey}
      oninput={onInput}
      onclick={updateMenu}
      onpaste={onPaste}
    ></textarea>

    {#if draft.pastes.length}
      <div class="pills">
        {#each draft.pastes as p (p.n)}
          <span class="pill" title={p.text.slice(0, 400)}>{pillLabel(p).slice(1, -1)}</span>
        {/each}
      </div>
    {/if}

    <div class="foot">
      {#if error}<span class="error" role="alert">{error}</span>{/if}
      <span class="hint">{$t("canvas.composer.hint", { default: "Enter sends · ⇧Enter new line · Esc closes" })}</span>
      <button type="button" class="send" disabled={busy || !draft.text.trim()} onclick={send}>
        {$t("canvas.composer.send", { default: "Send" })}
      </button>
    </div>
  </div>
{/if}

<style>
  .composer {
    position: absolute;
    z-index: 25;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px 8px;
    border-radius: 14px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 18px 44px rgba(0, 0, 0, 0.38);
    color: var(--text);
    font-size: 13px;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .to {
    font-size: 11px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .spacer {
    flex: 1;
  }
  .tool {
    all: unset;
    width: 22px;
    height: 22px;
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
  textarea {
    resize: none;
    min-height: 54px;
    max-height: 40vh;
    field-sizing: content;
    border: none;
    background: transparent;
    color: var(--text);
    font: 13px/1.5 var(--font-sans, inherit);
    outline: none;
  }
  .mentions {
    display: flex;
    flex-direction: column;
    max-height: 220px;
    overflow-y: auto;
    border-radius: 10px;
    border: 1px solid var(--border);
    padding: 4px;
  }
  .mentions.none {
    padding: 8px 10px;
    color: var(--text-muted);
    font-size: 12px;
  }
  .opt {
    all: unset;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border-radius: 7px;
    cursor: pointer;
  }
  .opt.on,
  .opt:hover {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .kind {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 6px;
    border-radius: 5px;
    background: var(--hover, rgba(127, 127, 127, 0.16));
    color: var(--text-muted);
  }
  .k-agent {
    color: #d97757;
  }
  .k-note {
    color: #eab308;
  }
  .k-portal {
    color: #3b82f6;
  }
  .k-action {
    color: #a855f7;
  }
  .k-floor,
  .k-workspace {
    color: #10b981;
  }
  .oname {
    font-weight: 600;
  }
  .detail {
    font-size: 11px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .pills {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .pill {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 99px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--text);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .hint {
    flex: 1;
    font-size: 11px;
    color: var(--text-muted);
  }
  .error {
    font-size: 12px;
    color: var(--danger, #e5484d);
  }
  .send {
    all: unset;
    padding: 5px 14px;
    border-radius: 8px;
    background: var(--accent);
    color: var(--on-accent, #fff);
    font-weight: 600;
    font-size: 12px;
    cursor: pointer;
  }
  .send:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
