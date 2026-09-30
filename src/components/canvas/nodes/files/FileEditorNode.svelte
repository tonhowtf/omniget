<script lang="ts">
  // File node (kind "file"). With `data.view = "editor"` (opened from a File
  // Tree) it is the editor of study 95 §K 118–121: syntax colours, line
  // numbers, ⌘S saves with `ifRevision` (conflict bar when the file changed
  // on disk, e.g. an agent wrote it), ⌘F find and replace, ⌃G go to line,
  // rendered Markdown, image/video/audio/PDF viewer, the file's working-tree
  // diff, ⌘D / ⌘⇧D extra cursors on the next / every occurrence, auto-closed
  // brackets and quotes, detected indentation, and "Ask…" that sends the
  // selected lines (path and line range) to a terminal.
  // Without it, it is the plain file card of the engine (FileNode).
  import { onMount, tick, untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { renderSafeMarkdownSync } from "$lib/llm/markdown";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { terminalIdOf, type CanvasNode } from "$lib/canvas/model/types";
  import { termKey, termPaste } from "$lib/canvas/term/client";
  import { FileConflict, baseName, fileSource, filesDefinition, filesGitDiff, joinPath, relativeTo, type FileText } from "$lib/canvas/files/api";
  import { showInEditor } from "$lib/canvas/files/open";
  import { workspaces } from "$lib/canvas/palette/workspaces.svelte";
  import { highlightLine, type HlState } from "$components/central/vcs/highlight";
  import { languageOf } from "$components/central/vcs/diff-model";
  import DiffViewer from "$components/central/vcs/DiffViewer.svelte";
  import type { FileDiff } from "$lib/central/vcs";
  import FileNode from "../FileNode.svelte";

  let {
    node,
    selected = false,
    focused = false,
    onexit,
    onfocusrequest,
  }: {
    node: CanvasNode;
    selected?: boolean;
    focused?: boolean;
    onexit?: () => void;
    onfocusrequest?: () => void;
    onpatch?: (patch: Partial<CanvasNode>) => void;
  } = $props();

  const engine = useEngine();

  const LINE_H = 18;
  const POLL_MS = 4000;

  let file = $state<FileText | null>(null);
  let draft = $state("");
  let dirty = $state(false);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let conflict = $state<FileText | null>(null);
  let mode = $state<"code" | "preview" | "diff">("code");
  let diff = $state<FileDiff[]>([]);
  let findOpen = $state(false);
  let findText = $state("");
  let replaceText = $state("");
  let findIndex = $state(0);
  let gotoOpen = $state(false);
  let gotoText = $state("");
  let scrollTop = $state(0);
  let scrollLeft = $state(0);
  let area = $state<HTMLTextAreaElement | null>(null);
  let findInput = $state<HTMLInputElement | null>(null);
  let gotoInput = $state<HTMLInputElement | null>(null);
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  const mdCache = new Map<string, string>();
  let mdBump = $state(0);

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) });

  const isEditor = $derived(node.data.view === "editor");
  const abs = $derived(String(node.data.path ?? ""));
  /** Remote editor (opened from an SSH/Docker tree, Maestri §K9). */
  const envData = $derived(node.data.environment);
  const root = $derived.by(() => {
    const r = String(node.data.rootPath ?? "");
    if (r) return r;
    const w = workspaces.byId(engine.store.workspaceId)?.workingDirectory ?? "";
    return w && relativeTo(w, abs) !== null ? w : "";
  });
  const rel = $derived(root ? relativeTo(root, abs) : null);
  const lang = $derived(languageOf(abs));
  const isMarkdown = $derived(lang === "markdown");
  const ws = $derived(engine.store.workspaceId);
  const src = $derived(fileSource(ws, root, envData));

  // ---- tabs (Maestri §K4) ---------------------------------------------------

  const tabs = $derived.by(() => {
    const t = Array.isArray(node.data.tabs) ? (node.data.tabs as unknown[]).filter((x): x is string => typeof x === "string") : [];
    return abs && !t.includes(abs) ? [...t, abs] : t;
  });
  /** Unsaved text of tabs that are not on screen. */
  const drafts = new Map<string, string>();

  function switchTab(path: string, line?: number) {
    if (path === abs && !line) return;
    if (dirty && abs) drafts.set(abs, draft);
    const n = engine.store.node(node.id) ?? node;
    showInEditor(engine, n, path, line);
  }

  function closeTab(path: string) {
    const list = tabs.filter((p) => p !== path);
    if (!list.length) return;
    drafts.delete(path);
    const n = engine.store.node(node.id) ?? node;
    if (path === abs) {
      const i = tabs.indexOf(path);
      const next = list[Math.min(i, list.length - 1)];
      showInEditor(engine, { ...n, data: { ...n.data, tabs: list } }, next);
    } else {
      engine.updateNode(node.id, { data: { ...n.data, tabs: list } });
    }
  }

  // ---- go to definition (F12, ⌘-click) ------------------------------------

  let defNote = $state<string | null>(null);

  async function goToDefinition(at?: number) {
    if (!area || !root || rel === null) return;
    const pos = at ?? area.selectionStart;
    const w = wordAt(draft, pos);
    const symbol = draft.slice(w.start, w.end);
    if (!symbol || !/^[A-Za-z_$][\w$]*$/.test(symbol)) return;
    if (src.remote) {
      defNote = tr("canvas.files.defRemote", "Go to definition works on local folders only.");
      return;
    }
    const here = draft.slice(0, pos).split("\n").length;
    let hits = await filesDefinition(ws, root, symbol, rel).catch(() => []);
    // Standing on the definition itself: go to the next one.
    hits = hits.filter((h) => !(h.path === rel && h.line === here)).concat(hits.filter((h) => h.path === rel && h.line === here));
    const hit = hits[0];
    if (!hit || (hit.path === rel && hit.line === here)) {
      defNote = tr("canvas.files.noDefinition", "No definition of {{name}} found", { name: symbol });
      setTimeout(() => (defNote = null), 2500);
      return;
    }
    defNote = null;
    if (hit.path === rel) goToLine(hit.line ?? 1);
    else switchTab(joinPath(root, hit.path), hit.line ?? 1);
  }

  // ---- Ask… (Maestri §K7): selected lines to a terminal -------------------

  let selLines = $state<{ from: number; to: number; text: string } | null>(null);
  let askOpen = $state(false);

  function trackSelection() {
    if (!area) return;
    const a = area.selectionStart;
    const b = area.selectionEnd;
    if (a === b) {
      selLines = null;
      return;
    }
    const from = draft.slice(0, a).split("\n").length;
    const to = draft.slice(0, Math.max(a, b - 1)).split("\n").length;
    selLines = { from, to, text: draft.slice(a, b) };
  }

  /** Terminals: the ones corded to this editor first, then the rest of the floor. */
  const askTargets = $derived.by(() => {
    void engine.store.structure;
    const tied = new Set<string>();
    for (const c of engine.store.connections) {
      if (c.from === node.id) tied.add(c.to);
      else if (c.to === node.id) tied.add(c.from);
    }
    const terms = engine.store.nodes.filter((n) => n.kind === "terminal" || n.kind === "agent");
    return [...terms.filter((n) => tied.has(n.id)), ...terms.filter((n) => !tied.has(n.id))].map((n) => ({
      id: n.id,
      name: n.title || n.id,
      tied: tied.has(n.id),
      term: terminalIdOf(n),
    }));
  });

  async function askSend(t: { term: string; name: string }) {
    askOpen = false;
    const sel = selLines;
    if (!sel) return;
    const where = `${rel ?? abs}:${sel.from === sel.to ? sel.from : `${sel.from}-${sel.to}`}`;
    const fence = sel.text.includes("```") ? "````" : "```";
    const text = `${where}\n${fence}${lang ?? ""}\n${sel.text.replace(/\n$/, "")}\n${fence}\n`;
    try {
      await termPaste(t.term, text);
      await new Promise((r) => setTimeout(r, 120));
      await termKey(t.term, { key: "Enter", code: "Enter" });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  // ---- media viewer (video, audio, PDF) ---------------------------------

  let mediaEl = $state<HTMLDivElement | null>(null);
  let mediaUrl = $state<string | null>(null);
  let playing = $state(false);

  function mediaKind(mime?: string): "image" | "video" | "audio" | "pdf" | null {
    if (!mime) return null;
    if (mime.startsWith("image/")) return "image";
    if (mime.startsWith("video/")) return "video";
    if (mime.startsWith("audio/")) return "audio";
    if (mime === "application/pdf") return "pdf";
    return null;
  }

  // The bytes come as a data URL once; players read a blob URL (cheaper to seek).
  $effect(() => {
    const f = file;
    const kind = mediaKind(f?.mime);
    if (!f?.dataUrl || !kind || kind === "image") return;
    let url: string | null = null;
    let live = true;
    void fetch(f.dataUrl)
      .then((r) => r.blob())
      .then((b) => {
        if (!live) return;
        url = URL.createObjectURL(b);
        mediaUrl = url;
      })
      .catch(() => {});
    return () => {
      live = false;
      if (url) URL.revokeObjectURL(url);
      mediaUrl = null;
    };
  });

  // Only a visible player plays: leaving the screen (or the floor) pauses it.
  $effect(() => {
    const el = mediaEl;
    if (!el) return;
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => !e.isIntersecting)) {
        el.querySelectorAll<HTMLMediaElement>("video, audio").forEach((m) => m.pause());
      }
    });
    io.observe(el);
    return () => {
      io.disconnect();
      el.querySelectorAll<HTMLMediaElement>("video, audio").forEach((m) => m.pause());
    };
  });

  // ---- load / poll -----------------------------------------------------

  async function load(keepDraft = false) {
    if (!root || rel === null) return;
    try {
      const f = await src.read(rel);
      file = f;
      error = null;
      const kept = drafts.get(abs);
      if (kept !== undefined) {
        drafts.delete(abs);
        draft = kept;
        dirty = kept !== (f.text ?? "");
      } else if (!keepDraft || !dirty) {
        draft = f.text ?? "";
        dirty = false;
      }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  $effect(() => {
    if (!isEditor) return;
    void abs;
    void root;
    untrack(() => void load());
  });

  async function poll() {
    if (!file || file.binary || !root || rel === null || saving) return;
    try {
      const f = await src.read(rel);
      if (f.revision === file.revision) return;
      if (dirty && f.text !== draft) conflict = f;
      else {
        file = f;
        draft = f.text ?? "";
        dirty = false;
      }
    } catch {
      // Deleted or moved: keep the buffer.
    }
  }

  onMount(() => {
    const off = workspaces.use();
    pollTimer = setInterval(() => void poll(), POLL_MS);
    return () => {
      off();
      if (pollTimer) clearInterval(pollTimer);
      mdCache.clear();
    };
  });

  // Jump to data.line when it changes (palette content search).
  $effect(() => {
    const line = Number(node.data.line ?? 0);
    if (!line || !file) return;
    untrack(() => void tick().then(() => goToLine(line)));
  });

  // `data.selectLines {from, to}` selects those lines once (MCP / tests:
  // the Ask… button then offers them), then clears itself.
  $effect(() => {
    const want = node.data.selectLines as { from?: number; to?: number } | undefined;
    if (!want?.from || !file || !area) return;
    untrack(() => {
      void tick().then(() => {
        if (!area) return;
        const from = Math.max(1, Math.min(lines.length, Math.floor(want.from!)));
        const to = Math.max(from, Math.min(lines.length, Math.floor(want.to ?? from)));
        let a = 0;
        for (let i = 0; i < from - 1; i++) a += lines[i].length + 1;
        let b = a;
        for (let i = from - 1; i < to; i++) b += lines[i].length + 1;
        area.focus({ preventScroll: true });
        area.setSelectionRange(a, Math.min(b, draft.length));
        area.scrollTop = Math.max(0, (from - 5) * LINE_H);
        trackSelection();
        // null, not a missing key: Rust merges `data` patches.
        const n = engine.store.node(node.id);
        if (n?.data.selectLines) engine.commit([{ op: "update_node", id: n.id, patch: { data: { ...n.data, selectLines: null } } }], { history: false });
      });
    });
  });

  $effect(() => {
    if (focused && mode === "code") void tick().then(() => area?.focus({ preventScroll: true }));
  });

  // ---- save ------------------------------------------------------------

  async function save(force?: string) {
    if (!file || !root || rel === null || saving) return;
    if (!dirty && !force) return;
    saving = true;
    try {
      const f = await src.write(rel, draft, force ?? file.revision);
      file = f;
      dirty = draft !== (f.text ?? "");
      conflict = null;
      error = null;
    } catch (e) {
      if (e instanceof FileConflict) {
        conflict = await src.read(rel).catch(() => null);
      } else error = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  function keepMine() {
    const c = conflict;
    conflict = null;
    if (c) void save(c.revision);
  }

  function takeTheirs() {
    const c = conflict;
    conflict = null;
    if (!c) return;
    file = c;
    draft = c.text ?? "";
    dirty = false;
  }

  // ---- highlight -------------------------------------------------------

  const lines = $derived(draft.split("\n"));
  const highlighted = $derived.by(() => {
    if (lines.length > 20000 || draft.length > 1_500_000) return null;
    const st: HlState = { block: false };
    return lines.map((l) => highlightLine(l, lang, st));
  });

  function onScroll(e: Event) {
    const el = e.currentTarget as HTMLTextAreaElement;
    scrollTop = el.scrollTop;
    scrollLeft = el.scrollLeft;
  }

  // ---- find / replace / go to line -------------------------------------

  const matches = $derived.by(() => {
    if (!findOpen || !findText) return [] as number[];
    const out: number[] = [];
    const hay = draft.toLowerCase();
    const needle = findText.toLowerCase();
    let i = hay.indexOf(needle);
    while (i >= 0 && out.length < 5000) {
      out.push(i);
      i = hay.indexOf(needle, i + Math.max(1, needle.length));
    }
    return out;
  });

  function selectMatch(k: number) {
    if (!matches.length || !area) return;
    findIndex = ((k % matches.length) + matches.length) % matches.length;
    const at = matches[findIndex];
    area.focus({ preventScroll: true });
    area.setSelectionRange(at, at + findText.length);
    const line = draft.slice(0, at).split("\n").length;
    area.scrollTop = Math.max(0, (line - 5) * LINE_H);
  }

  function replaceOne() {
    if (!matches.length) return;
    const at = matches[findIndex % matches.length];
    draft = draft.slice(0, at) + replaceText + draft.slice(at + findText.length);
    dirty = true;
    void tick().then(() => selectMatch(findIndex));
  }

  function replaceAll() {
    if (!findText) return;
    const re = new RegExp(findText.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi");
    draft = draft.replace(re, () => replaceText);
    dirty = true;
  }

  function goToLine(n: number) {
    if (!area) return;
    const target = Math.max(1, Math.min(lines.length, Math.floor(n)));
    let offset = 0;
    for (let i = 0; i < target - 1; i++) offset += lines[i].length + 1;
    area.focus({ preventScroll: true });
    area.setSelectionRange(offset, offset);
    area.scrollTop = Math.max(0, (target - 5) * LINE_H);
  }

  async function openFind() {
    findOpen = true;
    gotoOpen = false;
    const sel = area ? draft.slice(area.selectionStart, area.selectionEnd) : "";
    if (sel && !sel.includes("\n")) findText = sel;
    await tick();
    findInput?.select();
  }

  async function openGoto() {
    gotoOpen = true;
    findOpen = false;
    gotoText = "";
    await tick();
    gotoInput?.focus();
  }

  // ---- keys ------------------------------------------------------------

  // ---- extra cursors (⌘D), brackets, indentation -----------------------

  /** Extra selections; the textarea's own selection is always the last one. */
  let multi = $state<{ start: number; end: number }[]>([]);
  const PAIRS: Record<string, string> = { "(": ")", "[": "]", "{": "}", '"': '"', "'": "'", "`": "`" };

  /** Indent unit of the file: a tab, or the smallest run of leading spaces (2 by default). */
  const indentUnit = $derived.by(() => {
    let tabs = 0;
    let min = 0;
    for (const l of draft.split("\n", 2000)) {
      if (l.startsWith("\t")) tabs++;
      else {
        const m = /^( +)\S/.exec(l);
        if (m && (min === 0 || m[1].length < min)) min = m[1].length;
      }
    }
    if (tabs > 0 && tabs >= 3) return "\t";
    return " ".repeat(min >= 2 && min <= 8 ? min : 2);
  });

  function wordAt(text: string, at: number): { start: number; end: number } {
    let a = at;
    let b = at;
    while (a > 0 && /\w/.test(text[a - 1])) a--;
    while (b < text.length && /\w/.test(text[b])) b++;
    return { start: a, end: b };
  }

  function setText(next: string, caret: number, caretEnd = caret) {
    draft = next;
    dirty = draft !== (file?.text ?? "");
    void tick().then(() => area?.setSelectionRange(caret, caretEnd));
  }

  /** ⌘D: add the next occurrence of the selection; ⌘⇧D: every occurrence. */
  function addOccurrence(all: boolean) {
    if (!area) return;
    let cur = { start: area.selectionStart, end: area.selectionEnd };
    if (cur.start === cur.end) {
      cur = wordAt(draft, cur.start);
      if (cur.start === cur.end) return;
      if (!multi.length && !all) {
        area.setSelectionRange(cur.start, cur.end);
        return;
      }
    }
    const needle = draft.slice(cur.start, cur.end);
    if (all) {
      const out: { start: number; end: number }[] = [];
      for (let i = draft.indexOf(needle); i >= 0; i = draft.indexOf(needle, i + needle.length)) out.push({ start: i, end: i + needle.length });
      multi = out;
      const last = out[out.length - 1];
      if (last) area.setSelectionRange(last.start, last.end);
      return;
    }
    const have = multi.length ? multi : [cur];
    const from = Math.max(...have.map((r) => r.end));
    let i = draft.indexOf(needle, from);
    if (i < 0) i = draft.indexOf(needle);
    if (i < 0 || have.some((r) => r.start === i)) return;
    multi = [...have, { start: i, end: i + needle.length }];
    area.setSelectionRange(i, i + needle.length);
  }

  /** Types `ins` (or deletes one char back when null) at every extra cursor. */
  function editAll(ins: string | null) {
    const ranges = [...multi].sort((a, b) => a.start - b.start);
    let text = draft;
    let shift = 0;
    const next: { start: number; end: number }[] = [];
    for (const r of ranges) {
      let s0 = r.start + shift;
      const e0 = r.end + shift;
      if (ins === null && s0 === e0) s0 = Math.max(0, s0 - 1);
      const put = ins ?? "";
      text = text.slice(0, s0) + put + text.slice(e0);
      shift += put.length - (e0 - s0);
      next.push({ start: s0 + put.length, end: s0 + put.length });
    }
    multi = next;
    const last = next[next.length - 1];
    setText(text, last?.start ?? 0);
  }

  function onKey(e: KeyboardEvent) {
    const mod = e.metaKey || e.ctrlKey;
    const k = e.key.toLowerCase();
    if (mod && k === "d" && !e.altKey) {
      e.preventDefault();
      e.stopPropagation();
      addOccurrence(e.shiftKey);
      return;
    }
    if (multi.length > 1) {
      if (e.key.length === 1 && !mod && !e.altKey) {
        e.preventDefault();
        editAll(e.key);
        return;
      }
      if (e.key === "Backspace" && !mod) {
        e.preventDefault();
        editAll(null);
        return;
      }
      if (e.key === "Escape" || e.key.startsWith("Arrow") || e.key === "Enter" || e.key === "Tab") {
        multi = [];
        if (e.key === "Escape") {
          e.preventDefault();
          return;
        }
      }
    }
    if (area && !mod && !e.altKey && e.key.length === 1 && mode === "code") {
      const s0 = area.selectionStart;
      const e0 = area.selectionEnd;
      const close = PAIRS[e.key];
      const nextCh = draft[e0];
      // Typing the closer that is already there steps over it.
      if (s0 === e0 && (e.key === ")" || e.key === "]" || e.key === "}" || (close === e.key && nextCh === e.key)) && nextCh === e.key) {
        e.preventDefault();
        area.setSelectionRange(s0 + 1, s0 + 1);
        return;
      }
      if (close) {
        const quote = close === e.key;
        const prev = draft[s0 - 1] ?? "";
        // Quotes only pair at a word boundary (not in "don't").
        if (s0 !== e0 || !quote || (!/\w/.test(prev) && (!nextCh || /[\s)\]},;]/.test(nextCh)))) {
          if (s0 !== e0 || !nextCh || /[\s)\]},;]/.test(nextCh) || quote) {
            e.preventDefault();
            const inner = draft.slice(s0, e0);
            setText(draft.slice(0, s0) + e.key + inner + close + draft.slice(e0), s0 + 1, s0 + 1 + inner.length);
            return;
          }
        }
      }
    }
    if (area && e.key === "Backspace" && !mod && mode === "code") {
      const s0 = area.selectionStart;
      if (s0 === area.selectionEnd && s0 > 0 && PAIRS[draft[s0 - 1]] === draft[s0] && draft[s0] !== undefined) {
        e.preventDefault();
        setText(draft.slice(0, s0 - 1) + draft.slice(s0 + 1), s0 - 1);
        return;
      }
    }
    if (area && e.key === "Enter" && !mod && !e.shiftKey && mode === "code") {
      // Keep the line's indentation; one more level after an opener.
      e.preventDefault();
      const s0 = area.selectionStart;
      const e0 = area.selectionEnd;
      const lineStart = draft.lastIndexOf("\n", s0 - 1) + 1;
      const indent = /^[\t ]*/.exec(draft.slice(lineStart, s0))?.[0] ?? "";
      const before = draft[s0 - 1];
      const after = draft[e0];
      const opens = before === "(" || before === "[" || before === "{" || before === ":";
      if (opens && after && PAIRS[before] === after) {
        const ins = "\n" + indent + indentUnit + "\n" + indent;
        setText(draft.slice(0, s0) + ins + draft.slice(e0), s0 + 1 + indent.length + indentUnit.length);
      } else {
        const ins = "\n" + indent + (opens ? indentUnit : "");
        setText(draft.slice(0, s0) + ins + draft.slice(e0), s0 + ins.length);
      }
      return;
    }
    if (e.key === "F12" && !mod) {
      e.preventDefault();
      e.stopPropagation();
      void goToDefinition();
      return;
    }
    if (mod && k === "s") {
      e.preventDefault();
      e.stopPropagation();
      void save();
    } else if (mod && k === "f") {
      e.preventDefault();
      e.stopPropagation();
      void openFind();
    } else if (e.ctrlKey && k === "g") {
      e.preventDefault();
      e.stopPropagation();
      void openGoto();
    } else if (e.key === "Tab" && !mod && area) {
      e.preventDefault();
      const s = area.selectionStart;
      const en = area.selectionEnd;
      const unit = indentUnit;
      draft = draft.slice(0, s) + unit + draft.slice(en);
      dirty = true;
      void tick().then(() => area?.setSelectionRange(s + unit.length, s + unit.length));
    } else if (e.key === "Escape" && !e.shiftKey) {
      e.preventDefault();
      if (findOpen || gotoOpen) {
        findOpen = false;
        gotoOpen = false;
      } else onexit?.();
    }
  }

  function findKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      selectMatch(findIndex + (e.shiftKey ? -1 : 1));
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      findOpen = false;
      area?.focus();
    }
  }

  function gotoKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      gotoOpen = false;
      goToLine(Number(gotoText));
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      gotoOpen = false;
      area?.focus();
    }
  }

  async function showDiff() {
    mode = "diff";
    if (!root || rel === null) return;
    diff = await filesGitDiff(ws, root, rel)
      .then((d) => d.files)
      .catch(() => []);
  }

  const mdHtml = $derived.by(() => {
    void mdBump;
    return mode === "preview" ? renderSafeMarkdownSync(draft, mdCache, () => mdBump++) : "";
  });
</script>

{#if !isEditor}
  <FileNode {node} />
{:else}
  <div class="editor" class:focused class:selected>
    <div class="bar" data-canvas-chrome>
      <span class="path" title={abs}>{rel ?? abs}{dirty ? " •" : ""}</span>
      <div class="seg" role="tablist">
        <button type="button" role="tab" aria-selected={mode === "code"} class:on={mode === "code"} onclick={() => (mode = "code")}>{tr("canvas.files.code", "Code")}</button>
        {#if isMarkdown}
          <button type="button" role="tab" aria-selected={mode === "preview"} class:on={mode === "preview"} onclick={() => (mode = "preview")}>{tr("canvas.files.preview", "Preview")}</button>
        {/if}
        <button type="button" role="tab" aria-selected={mode === "diff"} class:on={mode === "diff"} onclick={() => void showDiff()}>{tr("canvas.files.diff", "Diff")}</button>
      </div>
      {#if saving}<span class="muted">{tr("canvas.files.saving", "Saving…")}</span>{/if}
      {#if multi.length > 1}<span class="muted">{tr("canvas.files.cursors", "{{count}} cursors", { count: multi.length })}</span>{/if}
      {#if selLines && mode === "code"}
        <div class="askwrap">
          <button type="button" class="tool ask" title={tr("canvas.files.askHint", "Send the selected lines to a terminal")} onclick={() => (askOpen = !askOpen)}>
            {tr("canvas.files.ask", "Ask…")} <span class="muted">{selLines.from === selLines.to ? `L${selLines.from}` : `L${selLines.from}–${selLines.to}`}</span>
          </button>
          {#if askOpen}
            <div class="askmenu" role="menu">
              {#each askTargets as t (t.id)}
                <button type="button" role="menuitem" onmousedown={(e) => e.preventDefault()} onclick={() => void askSend(t)}>
                  {t.name}{#if t.tied}<span class="muted"> · {tr("canvas.files.connected", "connected")}</span>{/if}
                </button>
              {:else}
                <p class="muted pad">{tr("canvas.files.noTerminals", "No terminal on this floor")}</p>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
      <button type="button" class="tool" disabled={!dirty} title={tr("canvas.files.save", "Save (⌘S)")} onclick={() => void save()}>{tr("canvas.files.saveShort", "Save")}</button>
    </div>

    {#if tabs.length > 1}
      <div class="tabs" data-canvas-chrome role="tablist">
        {#each tabs as p (p)}
          <div class="tab" class:on={p === abs} role="tab" aria-selected={p === abs} title={p}>
            <button type="button" class="tname" onclick={() => switchTab(p)}>{baseName(p)}{p === abs && dirty ? " •" : drafts.has(p) ? " •" : ""}</button>
            <button type="button" class="tclose" aria-label={tr("canvas.files.closeTab", "Close tab")} onclick={() => closeTab(p)}>×</button>
          </div>
        {/each}
      </div>
    {/if}
    {#if defNote}
      <div class="error" data-canvas-chrome role="status">{defNote}</div>
    {/if}

    {#if conflict}
      <div class="conflict" data-canvas-chrome role="alert">
        <span>{tr("canvas.files.conflict", "The file changed on disk.")}</span>
        <button type="button" onclick={takeTheirs}>{tr("canvas.files.reload", "Reload")}</button>
        <button type="button" class="primary" onclick={keepMine}>{tr("canvas.files.overwrite", "Overwrite")}</button>
      </div>
    {/if}
    {#if error}
      <div class="error" data-canvas-chrome role="status">{error}</div>
    {/if}

    {#if findOpen}
      <div class="find" data-canvas-chrome>
        <input bind:this={findInput} bind:value={findText} onkeydown={findKey} placeholder={tr("canvas.files.find", "Find")} spellcheck="false" />
        <span class="muted">{matches.length ? `${(findIndex % matches.length) + 1}/${matches.length}` : "0"}</span>
        <button type="button" onclick={() => selectMatch(findIndex - 1)} aria-label={tr("canvas.files.prev", "Previous")}>↑</button>
        <button type="button" onclick={() => selectMatch(findIndex + 1)} aria-label={tr("canvas.files.next", "Next")}>↓</button>
        <input bind:value={replaceText} placeholder={tr("canvas.files.replace", "Replace")} spellcheck="false" />
        <button type="button" onclick={replaceOne}>{tr("canvas.files.replaceOne", "Replace")}</button>
        <button type="button" onclick={replaceAll}>{tr("canvas.files.replaceAll", "All")}</button>
      </div>
    {/if}
    {#if gotoOpen}
      <div class="find" data-canvas-chrome>
        <input bind:this={gotoInput} bind:value={gotoText} onkeydown={gotoKey} inputmode="numeric" placeholder={tr("canvas.files.gotoLine", "Go to line (1–{{count}})", { count: lines.length })} />
      </div>
    {/if}

    <div class="body">
      {#if !root || rel === null}
        <p class="muted pad">{tr("canvas.files.outsideRoot", "This file is outside the workspace folder.")}</p>
      {:else if !file}
        <p class="muted pad">{tr("canvas.files.loading", "Loading…")}</p>
      {:else if file.dataUrl && mediaKind(file.mime) !== "image"}
        <div class="media" data-canvas-chrome data-playing={playing} bind:this={mediaEl}>
          {#if !mediaUrl}
            <p class="muted pad">{tr("canvas.files.loading", "Loading…")}</p>
          {:else if mediaKind(file.mime) === "video"}
            <!-- svelte-ignore a11y_media_has_caption -->
            <video src={mediaUrl} controls preload="metadata" onplay={() => (playing = true)} onpause={() => (playing = false)}></video>
          {:else if mediaKind(file.mime) === "audio"}
            <audio src={mediaUrl} controls preload="metadata" onplay={() => (playing = true)} onpause={() => (playing = false)}></audio>
          {:else}
            <iframe class="pdf" src={mediaUrl} title={rel}></iframe>
          {/if}
        </div>
      {:else if file.dataUrl}
        <div class="media"><img src={file.dataUrl} alt={rel} /></div>
      {:else if file.binary}
        <p class="muted pad">
          {file.truncated ? tr("canvas.files.tooLarge", "Too large to open here") : tr("canvas.files.binary", "Binary file")}
        </p>
      {:else if mode === "diff"}
        <div class="diffwrap" data-canvas-chrome>
          <DiffViewer files={diff} height="100%" emptyText={tr("canvas.files.noChanges", "No changes")} />
        </div>
      {:else if mode === "preview"}
        <div class="md">{@html mdHtml}</div>
      {:else}
        <div class="code">
          <div class="gutter" aria-hidden="true" style:transform="translateY({-scrollTop}px)">
            {#each lines as _, i}
              <div>{i + 1}</div>
            {/each}
          </div>
          <div class="stage">
            {#if highlighted}
              <pre class="hl" aria-hidden="true" style:transform="translate({-scrollLeft}px, {-scrollTop}px)">{#each highlighted as h}<div>{@html h || " "}</div>{/each}</pre>
            {/if}
            <textarea
              bind:this={area}
              bind:value={draft}
              class:plain={!highlighted}
              oninput={() => (dirty = draft !== (file?.text ?? ""))}
              onscroll={onScroll}
              onkeydown={onKey}
              onkeyup={trackSelection}
              onmouseup={trackSelection}
              onselect={trackSelection}
              onmousedown={() => (multi = [])}
              onclick={(e) => {
                if ((e.metaKey || e.ctrlKey) && area && area.selectionStart === area.selectionEnd) void goToDefinition(area.selectionStart);
              }}
              onfocus={() => {
                if (!focused) onfocusrequest?.();
              }}
              spellcheck="false"
              wrap="off"
              aria-label={rel ?? abs}
            ></textarea>
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .editor {
    height: 100%;
    display: flex;
    flex-direction: column;
    color: var(--text);
    font-size: 12px;
  }
  .bar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border-bottom: 1px solid var(--border);
    user-select: none;
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 11px var(--font-mono, monospace);
    color: var(--text-muted);
    direction: rtl;
    text-align: left;
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
    font-size: 11px;
    padding: 2px 7px;
    cursor: pointer;
  }
  .seg button.on {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--text);
  }
  .tool {
    font: inherit;
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 5px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .tool:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .muted {
    color: var(--text-muted);
  }
  .pad {
    padding: 12px;
    margin: 0;
  }
  .conflict,
  .find {
    flex: none;
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 4px 6px;
    border-bottom: 1px solid var(--border);
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 2px 6px 0;
    overflow-x: auto;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .tab {
    display: flex;
    align-items: center;
    border-radius: 6px 6px 0 0;
    font-size: 11px;
    opacity: 0.7;
  }
  .tab.on {
    opacity: 1;
    background: color-mix(in srgb, var(--accent, #4a8) 14%, transparent);
  }
  .tab button {
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font: inherit;
    padding: 3px 4px;
    white-space: nowrap;
  }
  .tab .tclose {
    opacity: 0.6;
    padding: 3px 5px 3px 0;
  }
  .tab .tclose:hover {
    opacity: 1;
  }
  .conflict {
    background: color-mix(in srgb, var(--warning, #dcab3c) 20%, transparent);
  }
  .conflict span {
    flex: 1;
  }
  .conflict button,
  .find button {
    font: inherit;
    font-size: 11px;
    padding: 2px 7px;
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
  .find input {
    flex: 1;
    min-width: 60px;
    font: 11px var(--font-mono, monospace);
    padding: 3px 6px;
    border-radius: 5px;
    border: 1px solid var(--border);
    background: var(--input-bg, transparent);
    color: var(--text);
    outline: none;
  }
  .error {
    flex: none;
    padding: 4px 8px;
    color: var(--danger, #d96a62);
    font-size: 11px;
  }
  .body {
    flex: 1;
    min-height: 0;
    position: relative;
    overflow: hidden;
  }
  .code {
    position: absolute;
    inset: 0;
    display: flex;
    font: 12px/18px var(--font-mono, monospace);
  }
  .gutter {
    flex: none;
    min-width: 36px;
    padding: 6px 8px 6px 4px;
    text-align: right;
    color: var(--text-muted);
    opacity: 0.6;
    user-select: none;
    border-right: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    will-change: transform;
  }
  .gutter div {
    height: 18px;
  }
  .stage {
    position: relative;
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }
  .hl {
    position: absolute;
    left: 0;
    top: 0;
    margin: 0;
    padding: 6px 10px;
    font: inherit;
    white-space: pre;
    pointer-events: none;
    color: var(--text);
    will-change: transform;
  }
  .hl div {
    height: 18px;
  }
  textarea {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    box-sizing: border-box;
    margin: 0;
    padding: 6px 10px;
    border: none;
    outline: none;
    resize: none;
    background: transparent;
    color: transparent;
    caret-color: var(--text);
    font: inherit;
    white-space: pre;
    overflow: auto;
    tab-size: 2;
  }
  textarea.plain {
    color: var(--text);
  }
  textarea::selection {
    background: color-mix(in srgb, var(--accent) 35%, transparent);
    color: transparent;
  }
  /* Same advance as the transparent textarea on top: no bold/italic. */
  .hl :global(.hl-keyword),
  .hl :global(.hl-comment) {
    font-weight: inherit;
    font-style: normal;
  }
  .md {
    height: 100%;
    overflow: auto;
    padding: 10px 14px;
    font-size: 14px;
    line-height: 1.5;
    user-select: text;
  }
  .askwrap {
    position: relative;
  }
  .askmenu {
    position: absolute;
    right: 0;
    top: 100%;
    z-index: 20;
    min-width: 160px;
    display: flex;
    flex-direction: column;
    padding: 4px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 10px 24px rgba(0, 0, 0, 0.3);
  }
  .askmenu button {
    text-align: left;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    padding: 4px 8px;
    border-radius: 5px;
    cursor: pointer;
  }
  .askmenu button:hover {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .media {
    height: 100%;
    display: grid;
    place-items: center;
    overflow: auto;
  }
  .media video,
  .media img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  .media video {
    max-height: 100%;
  }
  .media audio {
    width: min(100%, 420px);
  }
  .media .pdf {
    width: 100%;
    height: 100%;
    border: none;
    background: #fff;
  }
  .diffwrap {
    height: 100%;
  }
</style>
