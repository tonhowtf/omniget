<script lang="ts">
  // Visualizador de diff próprio (sem lib): unificado e dividido, números de
  // linha, dobra de contexto, destaque leve por regex, virtualização por
  // altura fixa de linha e seleção de linhas que vira comentário/contexto
  // para o chat.
  import { tick, untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { compactCount, type FileDiff, type ReviewComment } from "$lib/central/vcs";
  import { buildReviewComment, fileRows, languageOf, parsePatch, type DiffLine, type ParsedPatch, type Row } from "./diff-model";
  import { highlightLine, type HlState } from "./highlight";

  type Mode = "unified" | "split";

  let {
    files = [],
    mode = $bindable<Mode>("unified"),
    height = "100%",
    toolbar = true,
    collapsedByDefault = false,
    revealPath = null,
    oncomment,
    onrequestcontext,
    emptyText = null,
  }: {
    files?: FileDiff[];
    mode?: Mode;
    height?: string;
    toolbar?: boolean;
    collapsedByDefault?: boolean;
    /** Rola até este arquivo (e o abre) quando muda. */
    revealPath?: string | null;
    /** Seleção de linhas → comentário ou "adicionar ao chat" (texto vazio). */
    oncomment?: (c: ReviewComment) => void;
    /** Clique num trecho oculto entre hunks: o pai pode pedir o diff com mais contexto. */
    onrequestcontext?: () => void;
    emptyText?: string | null;
  } = $props();

  const H: Record<Row["t"], number> = { file: 36, hunk: 24, gap: 24, fold: 22, note: 30, line: 20, pair: 20 };
  const OVERSCAN = 30;

  let collapsed = $state(new Set<number>());
  let folds = $state(new Map<number, Set<string>>());
  let scrollTop = $state(0);
  let viewH = $state(600);
  let scroller: HTMLDivElement | undefined = $state();


  const parsed: ParsedPatch[] = $derived(files.map((f) => parsePatch(f.patch)));

  // HTML destacado por linha (estado de comentário de bloco por lado).
  const hlCache = new WeakMap<DiffLine, string>();
  function highlightFile(i: number) {
    const p = parsed[i];
    if (!p || !p.hunks.length) return;
    const first = p.hunks[0].lines[0];
    if (!first || hlCache.has(first)) return;
    const lang = languageOf(files[i].path);
    let total = 0;
    for (const h of p.hunks) total += h.lines.length;
    const plain = total > 20000;
    const oldS: HlState = { block: false };
    const newS: HlState = { block: false };
    for (const h of p.hunks) {
      oldS.block = false;
      newS.block = false;
      for (const l of h.lines) {
        if (plain) {
          hlCache.set(l, highlightLine(l.text, "text", newS));
        } else if (l.kind === "del") {
          hlCache.set(l, highlightLine(l.text, lang, oldS));
        } else if (l.kind === "add") {
          hlCache.set(l, highlightLine(l.text, lang, newS));
        } else {
          hlCache.set(l, highlightLine(l.text, lang, newS));
          highlightLine(l.text, lang, oldS);
        }
      }
    }
  }
  const html = (l: DiffLine | null) => (l ? (hlCache.get(l) ?? "") : "");

  type Placed = { row: Row; top: number; h: number };

  const perFile: Row[][] = $derived(
    files.map((f, i) => {
      if (collapsed.has(i)) return [];
      if (f.binary) return [{ t: "note", fileIndex: i, text: $t("llm.central.vcs.diff.binary") as string } as Row];
      if (f.truncated && !f.patch)
        return [{ t: "note", fileIndex: i, text: $t("llm.central.vcs.diff.too_large") as string } as Row];
      const p = parsed[i];
      if (!p.hunks.length) {
        const msg = f.status === "renamed" ? "llm.central.vcs.diff.renamed_only" : "llm.central.vcs.diff.no_content";
        return [{ t: "note", fileIndex: i, text: $t(msg) as string } as Row];
      }
      highlightFile(i);
      return fileRows(i, p, mode, folds.get(i) ?? new Set());
    }),
  );

  const layout = $derived.by(() => {
    const placed: Placed[] = [];
    const fileTop: number[] = [];
    let y = 0;
    files.forEach((_, i) => {
      fileTop[i] = y;
      placed.push({ row: { t: "file", fileIndex: i }, top: y, h: H.file });
      y += H.file;
      for (const r of perFile[i] ?? []) {
        const h = H[r.t];
        placed.push({ row: r, top: y, h });
        y += h;
      }
    });
    return { placed, fileTop, total: y };
  });

  const maxChars = $derived.by(() => {
    let m = 40;
    for (const rows of perFile)
      for (const r of rows) {
        if (r.t === "line") m = Math.max(m, r.line.text.length);
        else if (r.t === "pair") m = Math.max(m, r.left?.text.length ?? 0, r.right?.text.length ?? 0);
      }
    return Math.min(m, 400);
  });

  function firstIndexAt(y: number): number {
    const p = layout.placed;
    let lo = 0;
    let hi = p.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (p[mid].top <= y) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  const visible = $derived.by(() => {
    const p = layout.placed;
    if (!p.length) return { items: [] as Placed[], padTop: 0 };
    const a = Math.max(0, firstIndexAt(scrollTop) - OVERSCAN);
    const b = Math.min(p.length, firstIndexAt(scrollTop + viewH) + OVERSCAN + 1);
    return { items: p.slice(a, b), padTop: p[a].top };
  });

  // Cabeçalho fixo do arquivo em que o topo da vista está.
  const stickyFile = $derived.by(() => {
    const tops = layout.fileTop;
    let idx = -1;
    for (let i = 0; i < tops.length; i++) {
      if (tops[i] <= scrollTop) idx = i;
      else break;
    }
    return idx >= 0 && scrollTop > tops[idx] ? idx : -1;
  });

  const totals = $derived(
    files.reduce((acc, f) => ({ add: acc.add + f.additions, del: acc.del + f.deletions }), { add: 0, del: 0 }),
  );

  function toggleFile(i: number) {
    const next = new Set(collapsed);
    if (next.has(i)) next.delete(i);
    else next.add(i);
    collapsed = next;
  }

  function setAll(open: boolean) {
    collapsed = new Set(open ? [] : files.map((_, i) => i));
  }

  function openFold(fileIndex: number, key: string) {
    const next = new Map(folds);
    const set = new Set(next.get(fileIndex) ?? []);
    set.add(key);
    next.set(fileIndex, set);
    folds = next;
  }

  $effect(() => {
    const p = revealPath;
    const list = files;
    if (!p) return;
    untrack(() => {
      const i = list.findIndex((f) => f.path === p);
      if (i < 0) return;
      if (collapsed.has(i)) {
        const next = new Set(collapsed);
        next.delete(i);
        collapsed = next;
      }
      void tick().then(() => scroller?.scrollTo({ top: layout.fileTop[i] ?? 0 }));
    });
  });

  // ---- seleção de linhas ----

  type Side = "unified" | "left" | "right";
  let sel = $state<{ file: number; side: Side; a: number; b: number } | null>(null);
  let dragging = $state(false);
  let composing = $state(false);
  let commentText = $state("");

  function clearSelection() {
    sel = null;
    dragging = false;
    composing = false;
    commentText = "";
  }

  // Reinicia o estado quando a lista de arquivos muda de identidade (depois
  // de `sel`, que o efeito limpa: declarar antes quebra a montagem).
  let lastFiles: FileDiff[] | null = null;
  $effect.pre(() => {
    if (files !== lastFiles) {
      lastFiles = files;
      collapsed = new Set(collapsedByDefault ? files.map((_, i) => i) : []);
      folds = new Map();
      clearSelection();
    }
  });

  function lineOfRow(r: Row, side: Side): DiffLine | null {
    if (r.t === "line") return r.line;
    if (r.t === "pair") return side === "left" ? r.left : r.right;
    return null;
  }

  function startSel(e: MouseEvent, r: Row, side: Side) {
    if (e.button !== 0 || (r.t !== "line" && r.t !== "pair")) return;
    if (!lineOfRow(r, side)) return;
    e.preventDefault();
    if (e.shiftKey && sel && sel.file === r.fileIndex && sel.side === side) {
      sel = { ...sel, b: r.id };
    } else {
      sel = { file: r.fileIndex, side, a: r.id, b: r.id };
      composing = false;
    }
    dragging = true;
  }

  function moveSel(r: Row, side: Side) {
    if (!dragging || !sel || r.fileIndex !== sel.file || side !== sel.side) return;
    if (r.t !== "line" && r.t !== "pair") return;
    if (!lineOfRow(r, side)) return;
    sel = { ...sel, b: r.id };
  }

  function isSelected(r: Row, side: Side): boolean {
    if (!sel || r.fileIndex !== sel.file || (r.t !== "line" && r.t !== "pair")) return false;
    if (sel.side !== side) return false;
    const lo = Math.min(sel.a, sel.b);
    const hi = Math.max(sel.a, sel.b);
    return r.id >= lo && r.id <= hi && !!lineOfRow(r, side);
  }

  function selectedLines(): DiffLine[] {
    if (!sel) return [];
    const lo = Math.min(sel.a, sel.b);
    const hi = Math.max(sel.a, sel.b);
    const out: DiffLine[] = [];
    for (const r of perFile[sel.file] ?? []) {
      if ((r.t === "line" || r.t === "pair") && r.id >= lo && r.id <= hi) {
        const l = lineOfRow(r, sel.side);
        if (l && out[out.length - 1] !== l) out.push(l);
      }
    }
    return out;
  }

  const selBox = $derived.by(() => {
    if (!sel || dragging) return null;
    const hi = Math.max(sel.a, sel.b);
    const p = layout.placed.find(
      (x) => x.row.fileIndex === sel!.file && (x.row.t === "line" || x.row.t === "pair") && x.row.id === hi,
    );
    return p ? { top: p.top + p.h } : null;
  });

  function emit(text: string) {
    if (!sel || !oncomment) return;
    const lines = selectedLines();
    if (!lines.length) return;
    oncomment(buildReviewComment(files[sel.file].path, lines, text));
    clearSelection();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && sel) {
      e.stopPropagation();
      clearSelection();
    }
  }

  function onCommentKey(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      if (commentText.trim()) emit(commentText);
    } else if (e.key === "Escape") {
      e.preventDefault();
      clearSelection();
    }
  }

  const halves = (r: Extract<Row, { t: "pair" }>): [Side, DiffLine | null][] => [
    ["left", r.left],
    ["right", r.right],
  ];

  const statusLetter: Record<string, string> = { added: "A", deleted: "D", modified: "M", renamed: "R", binary: "B" };
  const num = (n: number | null | undefined) => (n == null ? "" : String(n));
</script>

<svelte:window onmouseup={() => (dragging = false)} />

<div class="dv" style:height>
  {#if toolbar}
    <div class="dv-bar">
      <span class="dv-sum">
        {$t("llm.central.vcs.diff.files", { count: files.length })}
        <span class="add">+{compactCount(totals.add)}</span>
        <span class="del">−{compactCount(totals.del)}</span>
      </span>
      <span class="dv-spacer"></span>
      <div class="seg" role="group" aria-label={$t("llm.central.vcs.diff.layout")}>
        <button class:on={mode === "unified"} onclick={() => (mode = "unified")}>{$t("llm.central.vcs.diff.unified")}</button>
        <button class:on={mode === "split"} onclick={() => (mode = "split")}>{$t("llm.central.vcs.diff.split")}</button>
      </div>
      <button class="ghost" onclick={() => setAll(collapsed.size > 0)}>
        {collapsed.size > 0 ? $t("llm.central.vcs.diff.expand_all") : $t("llm.central.vcs.diff.collapse_all")}
      </button>
    </div>
  {/if}

  {#if !files.length}
    <div class="dv-empty">{emptyText ?? $t("llm.central.vcs.diff.empty")}</div>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      role="region"
      aria-label={$t("llm.central.vcs.diff.title")}
      class="dv-scroll"
      class:split={mode === "split"}
      bind:this={scroller}
      bind:clientHeight={viewH}
      onscroll={(e) => (scrollTop = (e.currentTarget as HTMLDivElement).scrollTop)}
      onkeydown={onKey}
      tabindex="0"
    >
      <div class="dv-inner" style:height="{layout.total}px" style:--chars={maxChars}>
        {#if stickyFile >= 0}
          {@const f = files[stickyFile]}
          <div class="row file sticky" style:top="{scrollTop}px">
            <button class="file-btn" onclick={() => toggleFile(stickyFile)}>
              <span class="chev" class:closed={collapsed.has(stickyFile)}>▾</span>
              <span class="st st-{f.status}">{statusLetter[f.status] ?? "M"}</span>
              <span class="path">{f.path}</span>
              <span class="add">+{f.additions}</span><span class="del">−{f.deletions}</span>
            </button>
          </div>
        {/if}
        <div style:height="{visible.padTop}px"></div>
        {#each visible.items as p (p.top)}
          {@const r = p.row}
          {#if r.t === "file"}
            {@const f = files[r.fileIndex]}
            <div class="row file">
              <button class="file-btn" onclick={() => toggleFile(r.fileIndex)} title={f.path}>
                <span class="chev" class:closed={collapsed.has(r.fileIndex)}>▾</span>
                <span class="st st-{f.status}">{statusLetter[f.status] ?? "M"}</span>
                <span class="path">
                  {#if f.old_path && f.old_path !== f.path}<span class="old">{f.old_path} → </span>{/if}{f.path}
                </span>
                {#if f.truncated}<span class="badge">{$t("llm.central.vcs.diff.truncated")}</span>{/if}
                {#if f.binary}<span class="badge">{$t("llm.central.vcs.diff.binary_badge")}</span>{/if}
                <span class="add">+{f.additions}</span><span class="del">−{f.deletions}</span>
              </button>
            </div>
          {:else if r.t === "hunk"}
            <div class="row hunk"><span class="hunk-text">{r.label}</span></div>
          {:else if r.t === "gap"}
            <div class="row gap">
              <button class="gap-btn" disabled={!onrequestcontext} onclick={() => onrequestcontext?.()}>
                — {$t("llm.central.vcs.diff.hidden_lines", { count: r.hidden })} —
              </button>
            </div>
          {:else if r.t === "fold"}
            <div class="row fold">
              <button class="gap-btn" onclick={() => openFold(r.fileIndex, r.key)}>
                ⋯ {$t("llm.central.vcs.diff.unmodified_lines", { count: r.hidden })}
              </button>
            </div>
          {:else if r.t === "note"}
            <div class="row note">{r.text}</div>
          {:else if r.t === "line"}
            {@const l = r.line}
            <div class="row line k-{l.kind}" class:sel={isSelected(r, "unified")}>
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <span
                class="gut"
                onmousedown={(e) => startSel(e, r, "unified")}
                onmouseenter={() => moveSel(r, "unified")}
              ><span class="ln">{num(l.old)}</span><span class="ln">{num(l.new)}</span></span>
              <span class="mk">{l.kind === "add" ? "+" : l.kind === "del" ? "−" : " "}</span>
              <span class="code">{@html html(l)}{#if l.noEol}<span class="noeol" title={$t("llm.central.vcs.diff.no_eol")}>⏎̸</span>{/if}</span>
            </div>
          {:else if r.t === "pair"}
            <div class="row pair">
              {#each halves(r) as [side, l] (side)}
                {@const kind = !l ? "empty" : l.kind}
                <div class="half k-{kind}" class:sel={isSelected(r, side)}>
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <span class="gut one" onmousedown={(e) => startSel(e, r, side)} onmouseenter={() => moveSel(r, side)}>
                    <span class="ln">{l ? num(side === "left" ? l.old : l.new) : ""}</span>
                  </span>
                  <span class="mk">{kind === "add" ? "+" : kind === "del" ? "−" : " "}</span>
                  <span class="code">{@html html(l)}</span>
                </div>
              {/each}
            </div>
          {/if}
        {/each}

        {#if sel && selBox && oncomment}
          <div class="sel-pop" style:top="{selBox.top + 4}px">
            {#if composing}
              <!-- svelte-ignore a11y_autofocus -->
              <textarea
                bind:value={commentText}
                onkeydown={onCommentKey}
                placeholder={$t("llm.central.vcs.diff.comment_placeholder")}
                rows="3"
                autofocus
              ></textarea>
              <div class="pop-actions">
                <span class="hint">{$t("llm.central.vcs.diff.comment_hint")}</span>
                <button class="ghost" onclick={clearSelection}>{$t("llm.central.vcs.cancel")}</button>
                <button class="primary" disabled={!commentText.trim()} onclick={() => emit(commentText)}>
                  {$t("llm.central.vcs.diff.comment")}
                </button>
              </div>
            {:else}
              <div class="pop-actions">
                <span class="hint">{$t("llm.central.vcs.diff.lines_selected", { count: selectedLines().length })}</span>
                <button class="ghost" onclick={() => (composing = true)}>{$t("llm.central.vcs.diff.comment")}</button>
                <button class="primary" onclick={() => emit("")}>{$t("llm.central.vcs.diff.add_to_chat")}</button>
                <button class="ghost x" aria-label={$t("llm.central.vcs.cancel")} onclick={clearSelection}>×</button>
              </div>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .dv {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-md, 10px);
    overflow: hidden;
    font-size: 12px;
    --add-bg: color-mix(in srgb, var(--green, #34c759) 14%, transparent);
    --add-strong: color-mix(in srgb, var(--green, #34c759) 30%, transparent);
    --del-bg: color-mix(in srgb, var(--red, #ff3b30) 12%, transparent);
    --del-strong: color-mix(in srgb, var(--red, #ff3b30) 28%, transparent);
  }
  .dv-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--separator);
    font-size: 12px;
  }
  .dv-sum {
    color: var(--text-muted);
    display: inline-flex;
    gap: 6px;
  }
  .dv-spacer {
    flex: 1;
  }
  .seg {
    display: inline-flex;
    border: 1px solid var(--separator);
    border-radius: 7px;
    overflow: hidden;
  }
  .seg button {
    background: transparent;
    border: 0;
    padding: 3px 9px;
    font-size: 11px;
    color: var(--text-muted);
    cursor: pointer;
  }
  .seg button.on {
    background: var(--accent-soft);
    color: var(--accent-text, var(--accent));
  }
  button.ghost {
    background: transparent;
    border: 0;
    color: var(--text-muted);
    font-size: 11px;
    cursor: pointer;
    padding: 3px 6px;
    border-radius: 6px;
  }
  button.ghost:hover {
    background: var(--fill-2, var(--surface-hi));
  }
  button.primary {
    background: var(--accent);
    color: var(--on-accent, white);
    border: 0;
    border-radius: 6px;
    padding: 3px 10px;
    font-size: 11px;
    cursor: pointer;
  }
  button.primary:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .dv-empty {
    padding: 24px;
    text-align: center;
    color: var(--text-muted);
  }
  .dv-scroll {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: auto;
    outline: none;
    font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
  }
  .dv-inner {
    position: relative;
    min-width: calc(var(--chars) * 1ch + 110px);
  }
  .dv-scroll.split .dv-inner {
    min-width: calc(var(--chars) * 2ch + 140px);
  }
  .row {
    box-sizing: border-box;
    white-space: pre;
    overflow: hidden;
  }
  .row.file {
    height: 36px;
    display: flex;
    align-items: center;
    background: var(--surface-hi, var(--surface));
    border-top: 1px solid var(--separator);
    border-bottom: 1px solid var(--separator);
    font-family: var(--font-body, system-ui);
    position: sticky;
    left: 0;
  }
  .row.file.sticky {
    position: absolute;
    left: 0;
    right: 0;
    z-index: 3;
    box-shadow: 0 1px 0 var(--separator);
  }
  .file-btn {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 100%;
    border: 0;
    background: transparent;
    color: var(--text);
    padding: 0 10px;
    cursor: pointer;
    font-size: 12px;
    text-align: left;
  }
  .file-btn:hover {
    box-shadow: inset 3px 0 0 var(--accent);
  }
  .chev {
    display: inline-block;
    transition: transform 0.12s;
    color: var(--text-muted);
  }
  .chev.closed {
    transform: rotate(-90deg);
  }
  .st {
    font-family: var(--font-mono, monospace);
    font-size: 10px;
    font-weight: 700;
    width: 16px;
    height: 16px;
    border-radius: 4px;
    display: inline-grid;
    place-items: center;
    background: var(--fill-2, var(--surface-mut));
    color: var(--text-muted);
  }
  .st-added {
    color: var(--green, #34c759);
  }
  .st-deleted {
    color: var(--red, #ff3b30);
  }
  .st-modified,
  .st-renamed {
    color: var(--orange, #ff9500);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--font-mono, monospace);
  }
  .path .old {
    color: var(--text-muted);
  }
  .badge {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--fill-2, var(--surface-mut));
    color: var(--text-muted);
  }
  .add {
    color: var(--green, #34c759);
  }
  .del {
    color: var(--red, #ff3b30);
  }
  .row.hunk {
    height: 24px;
    line-height: 24px;
    padding-left: 110px;
    color: var(--text-muted);
    background: color-mix(in srgb, var(--accent) 7%, transparent);
  }
  .row.gap,
  .row.fold {
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .row.fold {
    height: 22px;
  }
  .gap-btn {
    border: 0;
    background: transparent;
    color: var(--text-muted);
    font-size: 11px;
    cursor: pointer;
    font-family: var(--font-body, system-ui);
  }
  .gap-btn:disabled {
    cursor: default;
  }
  .gap-btn:not(:disabled):hover {
    color: var(--accent);
  }
  .row.note {
    height: 30px;
    line-height: 30px;
    padding-left: 16px;
    color: var(--text-muted);
    font-family: var(--font-body, system-ui);
  }
  .row.line,
  .half {
    height: 20px;
    line-height: 20px;
    display: flex;
  }
  .row.pair {
    height: 20px;
    display: grid;
    grid-template-columns: 1fr 1fr;
  }
  .half {
    min-width: 0;
    overflow: hidden;
  }
  .half + .half {
    border-left: 1px solid var(--separator);
  }
  .gut {
    flex: none;
    display: inline-flex;
    user-select: none;
    cursor: pointer;
    color: var(--text-faint, var(--text-muted));
  }
  .gut .ln {
    width: 46px;
    text-align: right;
    padding-right: 6px;
    box-sizing: border-box;
  }
  .gut.one .ln {
    width: 50px;
  }
  .gut:hover {
    color: var(--accent);
  }
  .mk {
    flex: none;
    width: 14px;
    text-align: center;
    color: var(--text-muted);
    user-select: none;
  }
  .code {
    flex: 1;
    padding-right: 12px;
  }
  .k-add {
    background: var(--add-bg);
  }
  .k-add .gut {
    background: var(--add-strong);
  }
  .k-del {
    background: var(--del-bg);
  }
  .k-del .gut {
    background: var(--del-strong);
  }
  .k-empty {
    background: repeating-linear-gradient(
      -45deg,
      transparent 0 4px,
      color-mix(in srgb, var(--text-muted) 7%, transparent) 4px 5px
    );
  }
  .sel {
    background: color-mix(in srgb, var(--accent) 22%, transparent) !important;
    box-shadow: inset 4px 0 0 var(--accent);
  }
  .noeol {
    color: var(--red, #ff3b30);
    margin-left: 4px;
  }
  .sel-pop {
    position: absolute;
    left: 110px;
    z-index: 4;
    min-width: 280px;
    max-width: 520px;
    background: var(--popup-bg, var(--surface-hi));
    border: 1px solid var(--separator);
    border-radius: 10px;
    padding: 6px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
    font-family: var(--font-body, system-ui);
    white-space: normal;
  }
  .sel-pop textarea {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    background: var(--input-bg, var(--surface));
    color: var(--text);
    border: 1px solid var(--input-border, var(--separator));
    border-radius: 7px;
    padding: 6px 8px;
    font: inherit;
    font-size: 12px;
  }
  .pop-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 4px;
  }
  .pop-actions .hint {
    flex: 1;
    color: var(--text-muted);
    font-size: 11px;
  }
  .x {
    font-size: 14px !important;
  }
  :global(.hl-comment) {
    color: var(--text-muted);
    font-style: italic;
  }
  :global(.hl-string) {
    color: var(--green, #2e9d4a);
  }
  :global(.hl-number) {
    color: var(--orange, #c26b00);
  }
  :global(.hl-keyword) {
    color: var(--purple, #8e44ad);
    font-weight: 600;
  }
  :global(.hl-type) {
    color: var(--teal, #1f8a9e);
  }
  :global(.hl-fn) {
    color: var(--blue, #2f6fdb);
  }
  :global(.hl-meta) {
    color: var(--orange, #c26b00);
  }
</style>
