import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { t } from '$lib/i18n';
import { highlightInto, langOf } from './highlight';
import { mermaidKind, renderMermaid, resolveVars } from './mermaid';
import { copyFlavours, copyPng, copyText, svgToPng } from './clipboard';
import { readTable, toHtml, toMarkdown, toRtf, toTsv } from './tables';
import { findPaths, type PathMatch } from './links';
import './rich.css';

interface RichOptions {
  baseDir?: string | null;
  onOpenPath?: (abs: string, isDir: boolean, line?: number) => void;
  linkPaths?: boolean;
}

const tr = (key: string, fallback: string) => {
  try {
    return get(t)(key, { default: fallback }) || fallback;
  } catch {
    return fallback;
  }
};

interface Hit {
  abs: string;
  isDir: boolean;
}
const PROBE_TTL = 15_000;
const probeCache = new Map<string, { at: number; hit: Hit | null }>();
const imageCache = new Map<string, string | null>();

async function probe(paths: string[], base: string | null): Promise<Map<string, Hit | null>> {
  const out = new Map<string, Hit | null>();
  const now = Date.now();
  const ask: string[] = [];
  for (const p of paths) {
    const c = probeCache.get(`${base ?? ''}|${p}`);
    if (c && now - c.at < PROBE_TTL) out.set(p, c.hit);
    else if (!ask.includes(p)) ask.push(p);
  }
  if (ask.length) {
    const res = await invoke<(Hit | null)[]>('canvas_paths_probe', { paths: ask, base }).catch(() => ask.map(() => null));
    ask.forEach((p, i) => {
      const hit = res[i] ?? null;
      probeCache.set(`${base ?? ''}|${p}`, { at: now, hit });
      out.set(p, hit);
    });
    if (probeCache.size > 2000) probeCache.clear();
  }
  return out;
}

async function imageUrl(path: string, base: string | null): Promise<string | null> {
  const key = `${base ?? ''}|${path}`;
  if (imageCache.has(key)) return imageCache.get(key) ?? null;
  const url = await invoke<string | null>('canvas_image_data', { path, base }).catch(() => null);
  if (imageCache.size > 64) imageCache.delete(imageCache.keys().next().value as string);
  imageCache.set(key, url);
  return url;
}

function button(label: string, title: string, onClick: (btn: HTMLButtonElement) => void): HTMLButtonElement {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = 'rich-btn';
  b.textContent = label;
  b.title = title;
  b.setAttribute('aria-label', title);
  b.setAttribute('data-canvas-chrome', '');
  b.addEventListener('click', (e) => {
    e.preventDefault();
    e.stopPropagation();
    onClick(b);
  });
  b.addEventListener('pointerdown', (e) => e.stopPropagation());
  b.addEventListener('dblclick', (e) => e.stopPropagation());
  return b;
}

function flash(el: HTMLElement, ok: boolean, label?: string) {
  const prev = el.dataset.flash;
  el.dataset.flash = ok ? 'ok' : 'fail';
  if (label && el instanceof HTMLButtonElement) {
    const old = el.textContent;
    el.textContent = ok ? tr('canvas.rich.copied', 'Copied') : tr('canvas.rich.copyFailed', 'Copy failed');
    setTimeout(() => (el.textContent = old), 1200);
  }
  setTimeout(() => {
    if (prev === undefined) delete el.dataset.flash;
    else el.dataset.flash = prev;
  }, 900);
}

let menuEl: HTMLDivElement | null = null;
function closeMenu() {
  menuEl?.remove();
  menuEl = null;
  document.removeEventListener('pointerdown', onDocDown, true);
  document.removeEventListener('keydown', onDocKey, true);
}
function onDocDown(e: PointerEvent) {
  if (menuEl && !menuEl.contains(e.target as Node)) closeMenu();
}
function onDocKey(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    e.stopPropagation();
    closeMenu();
  }
}

function openMenu(host: HTMLElement, e: MouseEvent, items: { label: string; run: () => void }[], container: HTMLElement = host) {
  closeMenu();
  const m = document.createElement('div');
  m.className = 'rich-menu';
  m.setAttribute('role', 'menu');
  m.setAttribute('data-canvas-chrome', '');
  for (const it of items) {
    const b = document.createElement('button');
    b.type = 'button';
    b.setAttribute('role', 'menuitem');
    b.textContent = it.label;
    b.addEventListener('click', (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      closeMenu();
      it.run();
    });
    m.appendChild(b);
  }
  const r = host.getBoundingClientRect();
  const sx = host.offsetWidth ? r.width / host.offsetWidth : 1;
  const sy = host.offsetHeight ? r.height / host.offsetHeight : 1;
  m.style.left = `${Math.max(0, (e.clientX - r.left) / (sx || 1))}px`;
  m.style.top = `${Math.max(0, (e.clientY - r.top) / (sy || 1))}px`;
  if (getComputedStyle(host).position === 'static') host.style.position = 'relative';
  container.appendChild(m);
  menuEl = m;
  document.addEventListener('pointerdown', onDocDown, true);
  document.addEventListener('keydown', onDocKey, true);
}

function stopCanvas(el: HTMLElement) {
  el.addEventListener('pointerdown', (e) => {
    if (e.button === 2) e.stopPropagation();
  });
}

function background(host: Element): string {
  let el: Element | null = host;
  while (el) {
    const bg = getComputedStyle(el).backgroundColor;
    if (bg && bg !== 'transparent' && !/rgba\(\s*0,\s*0,\s*0,\s*0\s*\)/.test(bg)) return bg;
    el = el.parentElement;
  }
  return getComputedStyle(document.body).backgroundColor || '#ffffff';
}

function enhanceMermaid(pre: HTMLElement, code: HTMLElement) {
  const src = code.textContent ?? '';
  pre.dataset.rich = 'mermaid';
  const out = mermaidKind(src) ? renderMermaid(src) : null;
  if (!out) {
    pre.classList.add('rich-code');
    const note = document.createElement('span');
    note.className = 'rich-note';
    note.textContent = tr('canvas.rich.mermaidUnsupported', 'Diagram type not drawn yet — showing the source');
    pre.appendChild(note);
    addCopy(pre, () => src);
    return;
  }
  pre.classList.add('rich-mermaid');
  code.hidden = true;
  const view = document.createElement('div');
  view.className = 'rich-mermaid-view';
  view.innerHTML = out.svg;
  pre.appendChild(view);
  const svgEl = view.querySelector('svg');
  const asImage = (btn?: HTMLElement) => {
    const baked = resolveVars(out.svg, svgEl ?? pre);
    const png = svgToPng(baked, out.width, out.height, background(pre));
    void copyPng(png, src).then((ok) => btn && flash(btn, ok, 'x'));
  };
  const tools = document.createElement('div');
  tools.className = 'rich-tools';
  tools.setAttribute('data-canvas-chrome', '');
  const copySrc = button(tr('canvas.rich.copy', 'Copy'), tr('canvas.rich.copyMermaidSource', 'Copy Mermaid Source'), (btn) =>
    void copyText(src).then((ok) => flash(btn, ok, 'x')),
  );
  const copyImg = button(tr('canvas.rich.image', 'Image'), tr('canvas.rich.copyAsImage', 'Copy as Image'), (btn) => asImage(btn));
  tools.append(copySrc, copyImg);
  pre.appendChild(tools);
  stopCanvas(view);
  view.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    e.stopPropagation();
    openMenu(pre, e, [
      { label: tr('canvas.rich.copyMermaidSource', 'Copy Mermaid Source'), run: () => void copyText(src) },
      { label: tr('canvas.rich.copyAsImage', 'Copy as Image'), run: () => asImage() },
    ]);
  });
}

function addCopy(pre: HTMLElement, text: () => string) {
  const b = button(tr('canvas.rich.copy', 'Copy'), tr('canvas.rich.copyCode', 'Copy code'), (btn) =>
    void copyText(text()).then((ok) => flash(btn, ok, 'x')),
  );
  b.classList.add('rich-copy');
  pre.appendChild(b);
}

function enhanceCode(pre: HTMLElement, code: HTMLElement) {
  pre.dataset.rich = 'code';
  pre.classList.add('rich-code');
  const src = code.textContent ?? '';
  const cls = Array.from(code.classList).find((c) => c.startsWith('language-'));
  const lang = langOf(cls?.slice(9));
  if (lang) {
    highlightInto(code, src, lang);
    pre.dataset.lang = lang;
  }
  addCopy(pre, () => src);
}

function copyTable(table: HTMLTableElement, btn?: HTMLElement) {
  const data = readTable(table);
  const tsv = toTsv(data);
  void copyFlavours({ 'text/html': toHtml(data), 'text/rtf': toRtf(data), 'text/plain': tsv }, tsv).then((ok) => btn && flash(btn, ok, 'x'));
}

function copyMarkdown(table: HTMLTableElement, btn?: HTMLElement) {
  void copyText(toMarkdown(readTable(table))).then((ok) => btn && flash(btn, ok, 'x'));
}

function copyCell(cell: HTMLElement) {
  const sel = window.getSelection();
  if (sel) {
    const range = document.createRange();
    range.selectNodeContents(cell);
    sel.removeAllRanges();
    sel.addRange(range);
  }
  void copyText((cell.textContent ?? '').trim()).then((ok) => flash(cell, ok));
}

function enhanceTable(table: HTMLTableElement) {
  table.dataset.rich = 'table';
  table.classList.add('rich-table');
  const cap = document.createElement('caption');
  cap.className = 'rich-table-tools';
  cap.setAttribute('data-canvas-chrome', '');
  cap.append(
    button(tr('canvas.rich.copyTable', 'Copy as Table'), tr('canvas.rich.copyTableHint', 'Copy as a table (HTML, RTF and tab-separated text)'), (btn) =>
      copyTable(table, btn),
    ),
    button(tr('canvas.rich.copyMarkdown', 'Copy Markdown'), tr('canvas.rich.copyMarkdown', 'Copy Markdown'), (btn) =>
      copyMarkdown(table, btn),
    ),
  );
  table.insertBefore(cap, table.firstChild);
  table.addEventListener('dblclick', (e) => {
    const cell = (e.target as HTMLElement).closest('td,th') as HTMLElement | null;
    if (!cell || !table.contains(cell) || cap.contains(cell)) return;
    e.preventDefault();
    e.stopPropagation();
    copyCell(cell);
  });
  stopCanvas(table);
  table.addEventListener('contextmenu', (e) => {
    if (cap.contains(e.target as Node)) return;
    e.preventDefault();
    e.stopPropagation();
    const cell = (e.target as HTMLElement).closest('td,th') as HTMLElement | null;
    const items = [
      { label: tr('canvas.rich.copyTable', 'Copy as Table'), run: () => copyTable(table) },
      { label: tr('canvas.rich.copyMarkdown', 'Copy Markdown'), run: () => copyMarkdown(table) },
    ];
    if (cell) items.push({ label: tr('canvas.rich.copyCell', 'Copy Cell'), run: () => copyCell(cell) });
    openMenu(table, e, items, cap);
  });
}

function enhanceImage(img: HTMLImageElement, base: string | null) {
  const path = img.dataset.localSrc;
  if (!path) return;
  img.dataset.rich = 'img';
  img.draggable = false;
  img.classList.add('rich-img', 'rich-img-loading');
  void imageUrl(path, base).then((url) => {
    if (!img.isConnected) return;
    img.classList.remove('rich-img-loading');
    if (url) img.src = url;
    else {
      img.classList.add('rich-img-missing');
      img.title = tr('canvas.rich.imageMissing', 'Image not found') + `: ${path}`;
    }
  });
}

const SKIP = new Set(['A', 'PRE', 'CODE', 'BUTTON', 'CAPTION', 'SCRIPT', 'STYLE', 'TEXTAREA']);

function textNodes(root: HTMLElement): Text[] {
  const out: Text[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(n) {
      const p = n.parentElement;
      if (!p || p === root) return NodeFilter.FILTER_REJECT;
      for (let el: HTMLElement | null = p; el && el !== root; el = el.parentElement) {
        if (SKIP.has(el.tagName) || el.dataset.richPath !== undefined) return NodeFilter.FILTER_REJECT;
      }
      return /[/~]/.test(n.nodeValue ?? '') ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP;
    },
  });
  for (let n = walker.nextNode(); n; n = walker.nextNode()) out.push(n as Text);
  return out;
}

async function linkPaths(root: HTMLElement, base: string | null) {
  const found: { node: Text; matches: PathMatch[] }[] = [];
  const codes: { el: HTMLElement; match: PathMatch }[] = [];
  for (const n of textNodes(root)) {
    const matches = findPaths(n.nodeValue ?? '');
    if (matches.length) found.push({ node: n, matches });
  }
  for (const el of Array.from(root.querySelectorAll<HTMLElement>('code:not([data-rich-path])'))) {
    if (el.closest('pre') || el.closest('a')) continue;
    const m = findPaths(el.textContent ?? '');
    if (m.length === 1 && m[0].start === 0 && m[0].end === (el.textContent ?? '').length) codes.push({ el, match: m[0] });
    else el.dataset.richPath = '';
  }
  const all = [...found.flatMap((f) => f.matches.map((m) => m.path)), ...codes.map((c) => c.match.path)];
  if (!all.length) return;
  const hits = await probe(all, base);
  for (const { el, match } of codes) {
    const hit = hits.get(match.path);
    el.dataset.richPath = hit ? hit.abs : '';
    if (hit) markPath(el, hit, match.line);
  }
  for (const { node, matches } of found) {
    if (!node.isConnected) continue;
    const text = node.nodeValue ?? '';
    const real = matches.filter((m) => hits.get(m.path));
    if (!real.length) continue;
    const frag = document.createDocumentFragment();
    let at = 0;
    for (const m of real) {
      if (m.start < at) continue;
      frag.appendChild(document.createTextNode(text.slice(at, m.start)));
      const a = document.createElement('a');
      a.textContent = text.slice(m.start, m.end);
      const hit = hits.get(m.path)!;
      a.dataset.richPath = hit.abs;
      markPath(a, hit, m.line);
      frag.appendChild(a);
      at = m.end;
    }
    frag.appendChild(document.createTextNode(text.slice(at)));
    node.replaceWith(frag);
  }
}

function markPath(el: HTMLElement, hit: Hit, line?: number) {
  el.classList.add('rich-path');
  el.dataset.richPath = hit.abs;
  if (hit.isDir) el.dataset.richDir = '';
  if (line) el.dataset.richLine = String(line);
  el.title = `${hit.abs}\n${tr('canvas.rich.pathHint', '⌘-click to open on the canvas')}`;
  el.setAttribute('data-canvas-chrome', '');
}

function onClick(e: MouseEvent, opts: RichOptions) {
  if (e.defaultPrevented) return;
  const target = e.target as HTMLElement | null;
  const path = target?.closest<HTMLElement>('[data-rich-path]');
  if (path && path.dataset.richPath) {
    e.preventDefault();
    if (!(e.metaKey || e.ctrlKey)) return;
    e.stopPropagation();
    const line = path.dataset.richLine ? Number(path.dataset.richLine) : undefined;
    opts.onOpenPath?.(path.dataset.richPath, path.dataset.richDir !== undefined, line);
    return;
  }
  const a = target?.closest<HTMLAnchorElement>('a[href]');
  if (!a) return;
  e.preventDefault();
  const href = a.getAttribute('href') ?? '';
  if (!(e.metaKey || e.ctrlKey)) return;
  e.stopPropagation();
  if (/^(https?|mailto):/i.test(href)) {
    void import('@tauri-apps/plugin-shell').then((m) => m.open(href)).catch(() => window.open(href, '_blank'));
  }
}

function prepLinks(root: HTMLElement) {
  for (const a of Array.from(root.querySelectorAll<HTMLAnchorElement>('a[href]:not([data-rich])'))) {
    a.dataset.rich = 'link';
    const href = a.getAttribute('href') ?? '';
    if (/^(https?|mailto):/i.test(href)) {
      a.title = `${href}\n${tr('canvas.rich.linkHint', '⌘-click to open in the browser')}`;
      a.setAttribute('data-canvas-chrome', '');
    }
  }
}

function enhance(root: HTMLElement, opts: RichOptions) {
  const base = opts.baseDir ?? null;
  for (const code of Array.from(root.querySelectorAll<HTMLElement>('pre > code'))) {
    const pre = code.parentElement as HTMLElement;
    if (pre.dataset.rich) continue;
    if (code.classList.contains('language-mermaid')) enhanceMermaid(pre, code);
    else enhanceCode(pre, code);
  }
  for (const table of Array.from(root.querySelectorAll<HTMLTableElement>('table:not([data-rich])'))) enhanceTable(table);
  for (const img of Array.from(root.querySelectorAll<HTMLImageElement>('img[data-local-src]:not([data-rich])'))) enhanceImage(img, base);
  prepLinks(root);
  if (opts.linkPaths !== false) void linkPaths(root, base);
}

export function richMarkdown(node: HTMLElement, initial: RichOptions = {}) {
  let opts = initial;
  let queued = false;
  const run = () => {
    queued = false;
    observer.disconnect();
    try {
      enhance(node, opts);
    } finally {
      observer.observe(node, { childList: true, subtree: true });
    }
  };
  const observer = new MutationObserver(() => {
    if (queued) return;
    queued = true;
    queueMicrotask(run);
  });
  const click = (e: MouseEvent) => onClick(e, opts);
  node.addEventListener('click', click);
  node.classList.add('rich-md');
  run();
  return {
    update(next: RichOptions = {}) {
      const rerun = next.baseDir !== opts.baseDir;
      opts = next;
      if (rerun) run();
    },
    destroy() {
      observer.disconnect();
      node.removeEventListener('click', click);
      if (menuEl && node.contains(menuEl)) closeMenu();
    },
  };
}
