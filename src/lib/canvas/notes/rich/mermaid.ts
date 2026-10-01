// Mermaid, drawn by us (Maestri renders ```mermaid natively in notes and
// chat). A small parser + layered layout + SVG writer for the diagrams people
// actually paste: flowchart/graph (TD/TB/BT/LR/RL, the common node shapes,
// labelled and styled links, `&` fan-out), sequenceDiagram (participants,
// actors, the arrow family, notes, loop/alt/opt frames) and pie. Anything
// else returns null and the caller keeps the source as a code block.
//
// Output is an SVG string. Every piece of user text goes through `esc`.
// Colours are CSS custom properties with fallbacks (`var(--text, #222)`) so
// the diagram follows the theme; `resolveVars` bakes them for Copy as Image.

type MermaidKind = 'flowchart' | 'sequence' | 'pie';

export interface Rendered {
  kind: MermaidKind;
  svg: string;
  width: number;
  height: number;
}

const FONT = 13;
const LINE_H = 17;

const C = {
  text: 'var(--text, #1d1d1f)',
  muted: 'var(--text-muted, #6e6e73)',
  line: 'var(--text-muted, #6e6e73)',
  nodeFill: 'var(--mm-node, color-mix(in srgb, var(--accent, #0a84ff) 12%, var(--surface, #fff)))',
  nodeStroke: 'var(--accent, #0a84ff)',
  noteFill: 'var(--mm-note, color-mix(in srgb, #f5d76e 35%, var(--surface, #fff)))',
  noteStroke: '#c9a227',
  frame: 'var(--border, #c7c7cc)',
  bg: 'var(--surface, #fff)',
};

/** Categorical palette for pie slices (legible on light and dark). */
const PIE = ['#4e79a7', '#f28e2b', '#59a14f', '#e15759', '#76b7b2', '#edc948', '#b07aa1', '#ff9da7', '#9c755f', '#bab0ac'];

export function esc(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/** Rough text width at FONT px (no DOM needed; CJK counts double). */
function textWidth(s: string, size = FONT): number {
  let w = 0;
  for (const ch of s) {
    const code = ch.codePointAt(0) ?? 0;
    if (code > 0x2e80) w += 1.0;
    else if (/[mwMW@%]/.test(ch)) w += 0.82;
    else if (/[A-Z0-9#&]/.test(ch)) w += 0.66;
    else if (/[iljtf.,:;'|!()[\] ]/.test(ch)) w += 0.3;
    else w += 0.54;
  }
  return w * size;
}

function labelLines(label: string): string[] {
  return label
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/\\n/g, '\n')
    .split('\n')
    .map((l) => l.trim());
}

function textBlock(lines: string[], cx: number, cy: number, fill = C.text, weight = 400, size = FONT): string {
  const top = cy - ((lines.length - 1) * LINE_H) / 2;
  return lines
    .map(
      (l, i) =>
        `<text x="${r(cx)}" y="${r(top + i * LINE_H)}" text-anchor="middle" dominant-baseline="central" style="fill:${fill};font-size:${size}px;font-weight:${weight}">${esc(l)}</text>`,
    )
    .join('');
}

const r = (n: number) => Math.round(n * 10) / 10;

function cleanLines(src: string): string[] {
  return src
    .replace(/\r/g, '')
    .split('\n')
    .map((l) => l.replace(/%%.*$/, '').trim())
    .filter((l) => l.length > 0);
}

/** Diagram kind of a source, or null when we do not draw it. */
export function mermaidKind(src: string): MermaidKind | null {
  const first = cleanLines(src)[0] ?? '';
  if (/^(graph|flowchart)\b/i.test(first)) return 'flowchart';
  if (/^sequenceDiagram\b/.test(first)) return 'sequence';
  if (/^pie\b/.test(first)) return 'pie';
  return null;
}

export function renderMermaid(src: string): Rendered | null {
  try {
    switch (mermaidKind(src)) {
      case 'flowchart':
        return flowchart(src);
      case 'sequence':
        return sequence(src);
      case 'pie':
        return pie(src);
      default:
        return null;
    }
  } catch {
    return null;
  }
}

function wrapSvg(kind: MermaidKind, w: number, h: number, body: string, defs = ''): Rendered {
  const W = Math.ceil(w);
  const H = Math.ceil(h);
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" class="mermaid-svg" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" ` +
    `style="max-width:100%;height:auto;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif" role="img">` +
    `<defs>${defs}</defs>${body}</svg>`;
  return { kind, svg, width: W, height: H };
}

function markers(id: string): string {
  return (
    `<marker id="${id}-a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" style="fill:${C.line}"/></marker>` +
    `<marker id="${id}-x" viewBox="0 0 10 10" refX="5" refY="5" markerWidth="8" markerHeight="8" orient="auto"><path d="M1,1 L9,9 M9,1 L1,9" style="stroke:${C.line};stroke-width:1.6"/></marker>` +
    `<marker id="${id}-o" viewBox="0 0 10 10" refX="5" refY="5" markerWidth="7" markerHeight="7" orient="auto"><circle cx="5" cy="5" r="3.5" style="fill:${C.bg};stroke:${C.line};stroke-width:1.4"/></marker>` +
    `<marker id="${id}-open" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0,0 L10,5 L0,10" style="fill:none;stroke:${C.line};stroke-width:1.4"/></marker>`
  );
}

let seq = 0;
const uid = () => `mm${(++seq).toString(36)}`;

// ---------------------------------------------------------------------------
// Flowchart

type Shape = 'rect' | 'round' | 'stadium' | 'circle' | 'dcircle' | 'diamond' | 'hexagon' | 'cylinder' | 'subroutine' | 'asym' | 'para' | 'trap';

interface FNode {
  id: string;
  label: string;
  shape: Shape;
  w: number;
  h: number;
  rank: number;
  order: number;
  x: number;
  y: number;
  seen: number;
}

interface FEdge {
  from: string;
  to: string;
  label: string;
  style: 'solid' | 'dotted' | 'thick' | 'invisible';
  head: 'arrow' | 'none' | 'cross' | 'circle';
  tail: 'arrow' | 'none' | 'cross' | 'circle';
}

const SHAPES: [RegExp, Shape][] = [
  [/^\(\(\((.*?)\)\)\)/, 'dcircle'],
  [/^\(\((.*?)\)\)/, 'circle'],
  [/^\(\[(.*?)\]\)/, 'stadium'],
  [/^\[\[(.*?)\]\]/, 'subroutine'],
  [/^\[\((.*?)\)\]/, 'cylinder'],
  [/^\{\{(.*?)\}\}/, 'hexagon'],
  [/^\[\/(.*?)\/\]/, 'para'],
  [/^\[\\(.*?)\\\]/, 'para'],
  [/^\[\/(.*?)\\\]/, 'trap'],
  [/^\[\\(.*?)\/\]/, 'trap'],
  [/^\[(.*?)\]/, 'rect'],
  [/^\((.*?)\)/, 'round'],
  [/^\{(.*?)\}/, 'diamond'],
  [/^>(.*?)\]/, 'asym'],
];

const unquote = (s: string) => {
  const t = s.trim();
  return t.length >= 2 && t.startsWith('"') && t.endsWith('"') ? t.slice(1, -1) : t;
};

interface Cursor {
  s: string;
  i: number;
}

function skipWs(c: Cursor) {
  while (c.i < c.s.length && /\s/.test(c.s[c.i])) c.i++;
}

/** Node reference `id` + optional shape; returns id or null. */
function parseNodeRef(c: Cursor, nodes: Map<string, FNode>, counter: { n: number }): string | null {
  skipWs(c);
  const m = /^[A-Za-z0-9_À-￿][\wÀ-￿]*(?:-(?!-|\.|>)[\wÀ-￿]+)*/.exec(c.s.slice(c.i));
  if (!m) return null;
  const id = m[0];
  c.i += id.length;
  let label: string | null = null;
  let shape: Shape | null = null;
  const rest = c.s.slice(c.i);
  for (const [re, sh] of SHAPES) {
    const sm = re.exec(rest);
    if (sm) {
      label = unquote(sm[1]);
      shape = sh;
      c.i += sm[0].length;
      break;
    }
  }
  // `:::class` suffix is styling; ignore it.
  const cls = /^:::[\w-]+/.exec(c.s.slice(c.i));
  if (cls) c.i += cls[0].length;
  let n = nodes.get(id);
  if (!n) {
    n = { id, label: id, shape: 'rect', w: 0, h: 0, rank: 0, order: 0, x: 0, y: 0, seen: counter.n++ };
    nodes.set(id, n);
  }
  if (label !== null) n.label = label;
  if (shape) n.shape = shape;
  return id;
}

interface LinkTok {
  label: string;
  style: FEdge['style'];
  head: FEdge['head'];
  tail: FEdge['tail'];
}

function endOf(ch: string | undefined): FEdge['head'] {
  return ch === '>' || ch === '<' ? 'arrow' : ch === 'x' ? 'cross' : ch === 'o' ? 'circle' : 'none';
}

function parseLink(c: Cursor): LinkTok | null {
  skipWs(c);
  const rest = c.s.slice(c.i);
  // Text form: `-- text -->`, `== text ==>`, `-. text .->`
  let m = /^([<xo]?)(--|==|-\.)\s+([^\n]*?)\s+(-{2,}|={2,}|\.-+|-\.-*)([>xo]?)(?=\s|[\w"(\[{]|$)/.exec(rest);
  if (m && !/^[-=.]/.test(m[3])) {
    c.i += m[0].length;
    const body = m[2] + m[4];
    return {
      label: unquote(m[3]),
      style: body.includes('=') ? 'thick' : body.includes('.') ? 'dotted' : 'solid',
      head: endOf(m[5]),
      tail: endOf(m[1]),
    };
  }
  m = /^([<xo]?)(~~~|-{2,}|={2,}|-\.+-)([>xo]?)(?:\s*\|([^|]*)\|)?/.exec(rest);
  if (!m) return null;
  // `A --o B` vs a node named `o...`: an end char must be followed by space or a node.
  c.i += m[0].length;
  const body = m[2];
  return {
    label: m[4] !== undefined ? unquote(m[4]) : '',
    style: body === '~~~' ? 'invisible' : body.includes('=') ? 'thick' : body.includes('.') ? 'dotted' : 'solid',
    head: endOf(m[3]),
    tail: endOf(m[1]),
  };
}

function parseGroup(c: Cursor, nodes: Map<string, FNode>, counter: { n: number }): string[] | null {
  const ids: string[] = [];
  const first = parseNodeRef(c, nodes, counter);
  if (!first) return null;
  ids.push(first);
  for (;;) {
    const save = c.i;
    skipWs(c);
    if (c.s[c.i] !== '&') {
      c.i = save;
      break;
    }
    c.i++;
    const id = parseNodeRef(c, nodes, counter);
    if (!id) break;
    ids.push(id);
  }
  return ids;
}

interface FlowGraph {
  dir: 'TB' | 'BT' | 'LR' | 'RL';
  nodes: Map<string, FNode>;
  edges: FEdge[];
}

function parseFlowchart(src: string): FlowGraph {
  const lines = cleanLines(src);
  const head = /^(?:graph|flowchart)\s*(TB|TD|BT|LR|RL)?\s*;?\s*(.*)$/i.exec(lines[0] ?? '');
  const d = (head?.[1] ?? 'TB').toUpperCase();
  const dir = (d === 'TD' ? 'TB' : d) as FlowGraph['dir'];
  const nodes = new Map<string, FNode>();
  const edges: FEdge[] = [];
  const counter = { n: 0 };
  const stmts: string[] = [];
  const rest = [head?.[2] ?? '', ...lines.slice(1)];
  for (const l of rest) for (const s of l.split(/;(?=(?:[^"]*"[^"]*")*[^"]*$)/)) if (s.trim()) stmts.push(s.trim());
  for (const st of stmts) {
    if (/^(classDef|class|style|linkStyle|click|direction|accTitle|accDescr)\b/.test(st)) continue;
    if (/^end$/.test(st)) continue;
    if (/^subgraph\b/.test(st)) continue;
    const c: Cursor = { s: st, i: 0 };
    let left = parseGroup(c, nodes, counter);
    if (!left) continue;
    for (;;) {
      const link = parseLink(c);
      if (!link) break;
      const right = parseGroup(c, nodes, counter);
      if (!right) break;
      for (const a of left) for (const b of right) edges.push({ from: a, to: b, ...link });
      left = right;
    }
  }
  return { dir, nodes, edges };
}

function sizeNode(n: FNode) {
  const lines = labelLines(n.label);
  const tw = Math.max(20, ...lines.map((l) => textWidth(l)));
  const th = lines.length * LINE_H;
  let w = tw + 28;
  let h = th + 18;
  switch (n.shape) {
    case 'circle':
    case 'dcircle': {
      const d = Math.max(tw + 20, th + 20, 44);
      w = h = d + (n.shape === 'dcircle' ? 8 : 0);
      break;
    }
    case 'diamond': {
      const s = Math.max(tw + th + 30, 56);
      w = s * 1.15;
      h = s * 0.85;
      break;
    }
    case 'hexagon':
      w = tw + 44;
      break;
    case 'stadium':
      w = tw + h;
      break;
    case 'cylinder':
      h = th + 30;
      break;
    case 'subroutine':
      w = tw + 40;
      break;
    case 'para':
    case 'trap':
    case 'asym':
      w = tw + 44;
      break;
  }
  n.w = Math.max(w, 48);
  n.h = Math.max(h, 34);
}

/** Longest-path ranks with back edges (cycles) ignored; barycenter ordering. */
function layoutFlow(g: FlowGraph) {
  const nodes = [...g.nodes.values()].sort((a, b) => a.seen - b.seen);
  nodes.forEach(sizeNode);
  const out = new Map<string, string[]>();
  for (const n of nodes) out.set(n.id, []);
  // DFS to find back edges.
  const state = new Map<string, number>();
  const back = new Set<FEdge>();
  const adj = new Map<string, FEdge[]>();
  for (const e of g.edges) {
    if (!adj.has(e.from)) adj.set(e.from, []);
    adj.get(e.from)!.push(e);
  }
  const visit = (id: string) => {
    state.set(id, 1);
    for (const e of adj.get(id) ?? []) {
      const s = state.get(e.to) ?? 0;
      if (s === 1) back.add(e);
      else if (s === 0) visit(e.to);
    }
    state.set(id, 2);
  };
  for (const n of nodes) if (!state.get(n.id)) visit(n.id);
  const fwd = g.edges.filter((e) => !back.has(e) && e.from !== e.to);
  // Ranks by relaxation (graph is a DAG now).
  const rank = new Map<string, number>(nodes.map((n) => [n.id, 0]));
  for (let pass = 0; pass < nodes.length; pass++) {
    let changed = false;
    for (const e of fwd) {
      const want = (rank.get(e.from) ?? 0) + 1;
      if ((rank.get(e.to) ?? 0) < want) {
        rank.set(e.to, want);
        changed = true;
      }
    }
    if (!changed) break;
  }
  // Pull sources down next to their first child (avoids long first edges).
  for (const n of nodes) {
    const ins = fwd.filter((e) => e.to === n.id);
    const outs = fwd.filter((e) => e.from === n.id);
    if (!ins.length && outs.length) {
      const min = Math.min(...outs.map((e) => rank.get(e.to) ?? 0));
      rank.set(n.id, Math.max(0, min - 1));
    }
  }
  for (const n of nodes) n.rank = rank.get(n.id) ?? 0;
  const maxRank = Math.max(0, ...nodes.map((n) => n.rank));
  const layers: FNode[][] = Array.from({ length: maxRank + 1 }, () => []);
  for (const n of nodes) layers[n.rank].push(n);
  layers.forEach((l) => l.forEach((n, i) => (n.order = i)));
  const nb = (id: string, up: boolean) =>
    fwd.filter((e) => (up ? e.to === id : e.from === id)).map((e) => g.nodes.get(up ? e.from : e.to)!);
  for (let sweep = 0; sweep < 6; sweep++) {
    const down = sweep % 2 === 0;
    const order = down ? layers.map((_, i) => i) : layers.map((_, i) => layers.length - 1 - i);
    for (const li of order) {
      const layer = layers[li];
      const bary = new Map<string, number>();
      for (const n of layer) {
        const ns = nb(n.id, down);
        bary.set(n.id, ns.length ? ns.reduce((s, m) => s + m.order, 0) / ns.length : n.order);
      }
      layer.sort((a, b) => bary.get(a.id)! - bary.get(b.id)! || a.seen - b.seen);
      layer.forEach((n, i) => (n.order = i));
    }
  }
  // Coordinates. Main axis = rank; cross axis = order.
  const vertical = g.dir === 'TB' || g.dir === 'BT';
  const hasLabels = g.edges.some((e) => e.label);
  const RANK_GAP = hasLabels ? 64 : 48;
  const NODE_GAP = 28;
  const mainSize = (n: FNode) => (vertical ? n.h : n.w);
  const crossSize = (n: FNode) => (vertical ? n.w : n.h);
  const layerMain = layers.map((l) => Math.max(0, ...l.map(mainSize)));
  const layerCross = layers.map((l) => l.reduce((s, n) => s + crossSize(n), 0) + NODE_GAP * Math.max(0, l.length - 1));
  const crossTotal = Math.max(0, ...layerCross);
  let main = 0;
  layers.forEach((l, li) => {
    let cross = (crossTotal - layerCross[li]) / 2;
    const mid = main + layerMain[li] / 2;
    for (const n of l) {
      const cs = crossSize(n);
      const c = cross + cs / 2;
      if (vertical) {
        n.x = c;
        n.y = mid;
      } else {
        n.x = mid;
        n.y = c;
      }
      cross += cs + NODE_GAP;
    }
    main += layerMain[li] + RANK_GAP;
  });
  const mainTotal = Math.max(0, main - RANK_GAP);
  if (g.dir === 'BT') for (const n of nodes) n.y = mainTotal - n.y;
  if (g.dir === 'RL') for (const n of nodes) n.x = mainTotal - n.x;
  return { width: vertical ? crossTotal : mainTotal, height: vertical ? mainTotal : crossTotal, back };
}

function shapeSvg(n: FNode): string {
  const { x, y, w, h } = n;
  const L = x - w / 2;
  const T = y - h / 2;
  const st = `style="fill:${C.nodeFill};stroke:${C.nodeStroke};stroke-width:1.3"`;
  switch (n.shape) {
    case 'round':
      return `<rect x="${r(L)}" y="${r(T)}" width="${r(w)}" height="${r(h)}" rx="9" ${st}/>`;
    case 'stadium':
      return `<rect x="${r(L)}" y="${r(T)}" width="${r(w)}" height="${r(h)}" rx="${r(h / 2)}" ${st}/>`;
    case 'circle':
      return `<circle cx="${r(x)}" cy="${r(y)}" r="${r(w / 2)}" ${st}/>`;
    case 'dcircle':
      return `<circle cx="${r(x)}" cy="${r(y)}" r="${r(w / 2)}" ${st}/><circle cx="${r(x)}" cy="${r(y)}" r="${r(w / 2 - 4)}" style="fill:none;stroke:${C.nodeStroke};stroke-width:1.3"/>`;
    case 'diamond':
      return `<polygon points="${r(x)},${r(T)} ${r(L + w)},${r(y)} ${r(x)},${r(T + h)} ${r(L)},${r(y)}" ${st}/>`;
    case 'hexagon': {
      const k = 14;
      return `<polygon points="${r(L + k)},${r(T)} ${r(L + w - k)},${r(T)} ${r(L + w)},${r(y)} ${r(L + w - k)},${r(T + h)} ${r(L + k)},${r(T + h)} ${r(L)},${r(y)}" ${st}/>`;
    }
    case 'para': {
      const k = 12;
      return `<polygon points="${r(L + k)},${r(T)} ${r(L + w)},${r(T)} ${r(L + w - k)},${r(T + h)} ${r(L)},${r(T + h)}" ${st}/>`;
    }
    case 'trap': {
      const k = 12;
      return `<polygon points="${r(L + k)},${r(T)} ${r(L + w - k)},${r(T)} ${r(L + w)},${r(T + h)} ${r(L)},${r(T + h)}" ${st}/>`;
    }
    case 'asym': {
      const k = 14;
      return `<polygon points="${r(L)},${r(T)} ${r(L + w)},${r(T)} ${r(L + w)},${r(T + h)} ${r(L)},${r(T + h)} ${r(L + k)},${r(y)}" ${st}/>`;
    }
    case 'cylinder': {
      const ry = 7;
      return (
        `<path d="M${r(L)},${r(T + ry)} a${r(w / 2)},${ry} 0 0,0 ${r(w)},0 a${r(w / 2)},${ry} 0 0,0 ${r(-w)},0 v${r(h - 2 * ry)} a${r(w / 2)},${ry} 0 0,0 ${r(w)},0 v${r(-(h - 2 * ry))}" ${st}/>`
      );
    }
    case 'subroutine':
      return (
        `<rect x="${r(L)}" y="${r(T)}" width="${r(w)}" height="${r(h)}" ${st}/>` +
        `<path d="M${r(L + 8)},${r(T)} v${r(h)} M${r(L + w - 8)},${r(T)} v${r(h)}" style="stroke:${C.nodeStroke};stroke-width:1.3"/>`
      );
    default:
      return `<rect x="${r(L)}" y="${r(T)}" width="${r(w)}" height="${r(h)}" rx="3" ${st}/>`;
  }
}

/** Point where the ray from the centre towards (tx,ty) leaves the node box. */
function border(n: FNode, tx: number, ty: number): { x: number; y: number } {
  const dx = tx - n.x;
  const dy = ty - n.y;
  if (!dx && !dy) return { x: n.x, y: n.y };
  if (n.shape === 'circle' || n.shape === 'dcircle') {
    const d = Math.hypot(dx, dy);
    return { x: n.x + (dx / d) * (n.w / 2), y: n.y + (dy / d) * (n.w / 2) };
  }
  if (n.shape === 'diamond') {
    const s = 1 / (Math.abs(dx) / (n.w / 2) + Math.abs(dy) / (n.h / 2));
    return { x: n.x + dx * s, y: n.y + dy * s };
  }
  const sx = Math.abs(dx) > 0 ? n.w / 2 / Math.abs(dx) : Infinity;
  const sy = Math.abs(dy) > 0 ? n.h / 2 / Math.abs(dy) : Infinity;
  const s = Math.min(sx, sy);
  return { x: n.x + dx * s, y: n.y + dy * s };
}

const between = (x: number, a: number, b: number) => x >= Math.min(a, b) && x <= Math.max(a, b);
const strictlyBetween = (x: number, a: number, b: number) => x > Math.min(a, b) && x < Math.max(a, b);

function flowchart(src: string): Rendered | null {
  const g = parseFlowchart(src);
  if (!g.nodes.size) return null;
  const { width, height, back } = layoutFlow(g);
  const PAD = 16;
  const id = uid();
  const vertical = g.dir === 'TB' || g.dir === 'BT';
  let edgesSvg = '';
  let labelsSvg = '';
  for (const e of g.edges) {
    if (e.style === 'invisible') continue;
    const a = g.nodes.get(e.from)!;
    const b = g.nodes.get(e.to)!;
    const stroke =
      e.style === 'dotted'
        ? `stroke:${C.line};stroke-width:1.4;stroke-dasharray:4 4`
        : e.style === 'thick'
          ? `stroke:${C.text};stroke-width:2.6`
          : `stroke:${C.line};stroke-width:1.4`;
    const mk = (k: FEdge['head']) => (k === 'arrow' ? `url(#${id}-a)` : k === 'cross' ? `url(#${id}-x)` : k === 'circle' ? `url(#${id}-o)` : '');
    const me = mk(e.head);
    const ms = mk(e.tail);
    const attrs = `${me ? ` marker-end="${me}"` : ''}${ms ? ` marker-start="${ms}"` : ''}`;
    let d: string;
    let mid: { x: number; y: number };
    if (a === b) {
      // Self loop on the right/bottom side.
      const s = { x: a.x + a.w / 2, y: a.y - 6 };
      const t = { x: a.x + a.w / 2, y: a.y + 6 };
      d = `M${r(s.x)},${r(s.y)} C${r(s.x + 40)},${r(s.y - 30)} ${r(t.x + 40)},${r(t.y + 30)} ${r(t.x)},${r(t.y)}`;
      mid = { x: a.x + a.w / 2 + 34, y: a.y };
    } else if (back.has(e)) {
      // Back edges run around the outside (right in TB/BT, below in LR/RL).
      const far = vertical
        ? Math.max(...[...g.nodes.values()].filter((n) => between(n.rank, a.rank, b.rank)).map((n) => n.x + n.w / 2)) + 26
        : Math.max(...[...g.nodes.values()].filter((n) => between(n.rank, a.rank, b.rank)).map((n) => n.y + n.h / 2)) + 26;
      const s = vertical ? { x: a.x + a.w / 2, y: a.y } : { x: a.x, y: a.y + a.h / 2 };
      const t = vertical ? { x: b.x + b.w / 2, y: b.y } : { x: b.x, y: b.y + b.h / 2 };
      const c1 = vertical ? { x: far, y: s.y } : { x: s.x, y: far };
      const c2 = vertical ? { x: far, y: t.y } : { x: t.x, y: far };
      d = `M${r(s.x)},${r(s.y)} C${r(c1.x)},${r(c1.y)} ${r(c2.x)},${r(c2.y)} ${r(t.x)},${r(t.y)}`;
      mid = { x: (s.x + 3 * c1.x + 3 * c2.x + t.x) / 8, y: (s.y + 3 * c1.y + 3 * c2.y + t.y) / 8 };
    } else if (a.rank === b.rank) {
      const s = border(a, b.x, b.y);
      const t = border(b, a.x, a.y);
      d = `M${r(s.x)},${r(s.y)} L${r(t.x)},${r(t.y)}`;
      mid = { x: (s.x + t.x) / 2, y: (s.y + t.y) / 2 };
    } else {
      const s = border(a, vertical ? a.x : b.x, vertical ? b.y : a.y);
      const t = border(b, vertical ? b.x : a.x, vertical ? a.y : b.y);
      const k = vertical ? (t.y - s.y) / 2 : (t.x - s.x) / 2;
      // Edges that skip a rank bow sideways so the node in between does not hide them.
      const skip = Math.abs(b.rank - a.rank) > 1 && [...g.nodes.values()].some((n) => n !== a && n !== b && strictlyBetween(n.rank, a.rank, b.rank) && (vertical ? Math.abs(n.x - (a.x + b.x) / 2) < n.w / 2 + 6 : Math.abs(n.y - (a.y + b.y) / 2) < n.h / 2 + 6));
      const bow = skip ? (vertical ? a.w / 2 + 30 : a.h / 2 + 30) : 0;
      const c1 = vertical ? { x: s.x + bow, y: s.y + k } : { x: s.x + k, y: s.y + bow };
      const c2 = vertical ? { x: t.x + bow, y: t.y - k } : { x: t.x - k, y: t.y + bow };
      d = `M${r(s.x)},${r(s.y)} C${r(c1.x)},${r(c1.y)} ${r(c2.x)},${r(c2.y)} ${r(t.x)},${r(t.y)}`;
      mid = { x: (s.x + 3 * c1.x + 3 * c2.x + t.x) / 8, y: (s.y + 3 * c1.y + 3 * c2.y + t.y) / 8 };
    }
    edgesSvg += `<path d="${d}" style="fill:none;${stroke}"${attrs}/>`;
    if (e.label) {
      const lines = labelLines(e.label);
      const lw = Math.max(...lines.map((l) => textWidth(l, 12))) + 10;
      const lh = lines.length * LINE_H + 2;
      labelsSvg +=
        `<rect x="${r(mid.x - lw / 2)}" y="${r(mid.y - lh / 2)}" width="${r(lw)}" height="${r(lh)}" rx="3" style="fill:${C.bg};opacity:0.92"/>` +
        textBlock(lines, mid.x, mid.y, C.muted, 400, 12);
    }
  }
  let nodesSvg = '';
  for (const n of g.nodes.values()) nodesSvg += shapeSvg(n) + textBlock(labelLines(n.label), n.x, n.y);
  const hasBack = back.size > 0 || g.edges.some((e) => Math.abs(g.nodes.get(e.from)!.rank - g.nodes.get(e.to)!.rank) > 1) ? 60 : 0;
  const body = `<g transform="translate(${PAD},${PAD})">${edgesSvg}${nodesSvg}${labelsSvg}</g>`;
  return wrapSvg('flowchart', width + PAD * 2 + (vertical ? hasBack : 0), height + PAD * 2 + (vertical ? 0 : hasBack), body, markers(id));
}

// ---------------------------------------------------------------------------
// Sequence diagram

interface SPart {
  id: string;
  label: string;
  actor: boolean;
  x: number;
}

type SStep =
  | { t: 'msg'; from: string; to: string; text: string; dashed: boolean; head: 'arrow' | 'open' | 'cross' | 'async' }
  | { t: 'note'; where: 'left' | 'right' | 'over'; a: string; b?: string; text: string }
  | { t: 'open'; kind: string; label: string }
  | { t: 'else'; label: string }
  | { t: 'close' };

function parseSequence(src: string): { parts: SPart[]; steps: SStep[]; title: string } {
  const lines = cleanLines(src).slice(1);
  const parts: SPart[] = [];
  const byId = new Map<string, SPart>();
  const steps: SStep[] = [];
  let title = '';
  const part = (id: string, label?: string, actor = false) => {
    const k = id.trim();
    let p = byId.get(k);
    if (!p) {
      p = { id: k, label: label ?? k, actor, x: 0 };
      byId.set(k, p);
      parts.push(p);
    } else if (label) p.label = label;
    return p;
  };
  for (const l of lines) {
    let m: RegExpExecArray | null;
    if ((m = /^title\s*:?\s*(.*)$/i.exec(l))) {
      title = m[1];
    } else if ((m = /^(participant|actor)\s+(.+?)(?:\s+as\s+(.+))?$/i.exec(l))) {
      part(m[2], m[3] ? unquote(m[3]) : undefined, m[1].toLowerCase() === 'actor');
    } else if ((m = /^note\s+(left of|right of|over)\s+([^,:]+?)(?:\s*,\s*([^:]+?))?\s*:\s*(.*)$/i.exec(l))) {
      const where = m[1].toLowerCase().startsWith('left') ? 'left' : m[1].toLowerCase().startsWith('right') ? 'right' : 'over';
      part(m[2]);
      if (m[3]) part(m[3]);
      steps.push({ t: 'note', where, a: m[2].trim(), b: m[3]?.trim(), text: m[4] });
    } else if ((m = /^(loop|alt|opt|par|critical|break|rect)\b\s*(.*)$/i.exec(l))) {
      steps.push({ t: 'open', kind: m[1].toLowerCase(), label: m[1].toLowerCase() === 'rect' ? '' : m[2] });
    } else if ((m = /^(else|and|option)\b\s*(.*)$/i.exec(l))) {
      steps.push({ t: 'else', label: m[2] });
    } else if (/^end$/i.test(l)) {
      steps.push({ t: 'close' });
    } else if (/^(activate|deactivate|autonumber|box|links?|properties|details|create|destroy)\b/i.test(l)) {
      continue;
    } else if ((m = /^([^-+>:]+?)\s*(-{1,2})(>>|>|x|\))\s*[+-]?\s*([^:]+?)\s*(?::\s*(.*))?$/.exec(l))) {
      part(m[1]);
      part(m[4]);
      steps.push({
        t: 'msg',
        from: m[1].trim(),
        to: m[4].trim(),
        text: m[5] ?? '',
        dashed: m[2] === '--',
        head: m[3] === '>>' ? 'arrow' : m[3] === '>' ? 'open' : m[3] === 'x' ? 'cross' : 'async',
      });
    }
  }
  return { parts, steps, title };
}

function sequence(src: string): Rendered | null {
  const { parts, steps, title } = parseSequence(src);
  if (!parts.length) return null;
  const id = uid();
  const BOX_H = 36;
  const idx = new Map(parts.map((p, i) => [p.id, i]));
  // Column gaps: fit the widest message between each neighbouring pair.
  const gap = parts.map(() => 0);
  for (const s of steps) {
    if (s.t !== 'msg') continue;
    const a = idx.get(s.from)!;
    const b = idx.get(s.to)!;
    const lo = Math.min(a, b);
    const hi = Math.max(a, b);
    const need = textWidth(s.text, 12) + 30;
    if (hi > lo) {
      const per = need / (hi - lo);
      for (let k = lo; k < hi; k++) gap[k] = Math.max(gap[k], per);
    }
  }
  const boxW = parts.map((p) => Math.max(80, textWidth(p.label) + 26));
  let x = 16 + boxW[0] / 2;
  parts.forEach((p, i) => {
    p.x = x;
    const next = boxW[i + 1] ?? 0;
    x += Math.max(boxW[i] / 2 + next / 2 + 24, gap[i], 110);
  });
  const width = parts[parts.length - 1].x + boxW[parts.length - 1] / 2 + 16 + 60;
  let y = (title ? 30 : 0) + 16 + BOX_H + 22;
  let body = '';
  let over = '';
  const frames: { kind: string; label: string; y: number; elses: { y: number; label: string }[] }[] = [];
  const frameOut: string[] = [];
  for (const s of steps) {
    if (s.t === 'msg') {
      const a = byIdX(parts, s.from);
      const b = byIdX(parts, s.to);
      const lines = labelLines(s.text);
      const th = lines.length * LINE_H;
      const style = `stroke:${C.line};stroke-width:1.4${s.dashed ? ';stroke-dasharray:5 4' : ''}`;
      const mk = s.head === 'arrow' ? `url(#${id}-a)` : s.head === 'cross' ? `url(#${id}-x)` : `url(#${id}-open)`;
      over += textBlock(lines, a === b ? a + 40 : (a + b) / 2, y + th / 2 - 2, C.text, 400, 12);
      const ly = y + th + 4;
      if (a === b) {
        over += `<path d="M${r(a)},${r(ly)} h36 v18 h-34" style="fill:none;${style}" marker-end="${mk}"/>`;
        y = ly + 34;
      } else {
        const dir = b > a ? -1 : 1;
        over += `<line x1="${r(a)}" y1="${r(ly)}" x2="${r(b + dir * 2)}" y2="${r(ly)}" style="${style}" marker-end="${mk}"/>`;
        y = ly + 22;
      }
    } else if (s.t === 'note') {
      const lines = labelLines(s.text);
      const tw = Math.max(...lines.map((l) => textWidth(l, 12))) + 20;
      const h = lines.length * LINE_H + 12;
      const ax = byIdX(parts, s.a);
      const bx = s.b ? byIdX(parts, s.b) : ax;
      let L: number;
      let w = tw;
      if (s.where === 'over') {
        const lo = Math.min(ax, bx);
        const hi = Math.max(ax, bx);
        w = Math.max(tw, hi - lo + 60);
        L = (lo + hi) / 2 - w / 2;
      } else if (s.where === 'left') L = ax - 12 - tw;
      else L = ax + 12;
      over +=
        `<rect x="${r(L)}" y="${r(y)}" width="${r(w)}" height="${r(h)}" style="fill:${C.noteFill};stroke:${C.noteStroke};stroke-width:1"/>` +
        textBlock(lines, L + w / 2, y + h / 2, C.text, 400, 12);
      y += h + 14;
    } else if (s.t === 'open') {
      frames.push({ kind: s.kind, label: s.label, y, elses: [] });
      y += 30;
    } else if (s.t === 'else') {
      const f = frames[frames.length - 1];
      if (f) {
        f.elses.push({ y, label: s.label });
        y += 26;
      }
    } else if (s.t === 'close') {
      const f = frames.pop();
      if (!f) continue;
      const depth = frames.length;
      const L = 6 + depth * 6;
      const R = width - 6 - depth * 6;
      const tag = f.kind;
      const tw = textWidth(tag, 11) + 16;
      let fr =
        `<rect x="${r(L)}" y="${r(f.y)}" width="${r(R - L)}" height="${r(y - f.y + 4)}" style="fill:none;stroke:${C.frame};stroke-width:1"/>` +
        `<path d="M${r(L)},${r(f.y)} h${r(tw)} v12 l-6,8 h${r(-(tw - 6))} z" style="fill:${C.frame}"/>` +
        `<text x="${r(L + 6)}" y="${r(f.y + 11)}" dominant-baseline="central" style="fill:${C.text};font-size:11px;font-weight:600">${esc(tag)}</text>`;
      if (f.label) fr += `<text x="${r(L + tw + 8)}" y="${r(f.y + 11)}" dominant-baseline="central" style="fill:${C.muted};font-size:11px">[${esc(f.label)}]</text>`;
      for (const e of f.elses) {
        fr += `<line x1="${r(L)}" y1="${r(e.y)}" x2="${r(R)}" y2="${r(e.y)}" style="stroke:${C.frame};stroke-dasharray:4 3"/>`;
        if (e.label) fr += `<text x="${r(L + 8)}" y="${r(e.y + 12)}" dominant-baseline="central" style="fill:${C.muted};font-size:11px">[${esc(e.label)}]</text>`;
      }
      frameOut.push(fr);
      y += 10;
    }
  }
  const bottom = y + 8;
  const top = (title ? 30 : 0) + 16;
  let heads = '';
  parts.forEach((p, i) => {
    const w = boxW[i];
    body += `<line x1="${r(p.x)}" y1="${r(top + BOX_H)}" x2="${r(p.x)}" y2="${r(bottom)}" style="stroke:${C.frame};stroke-width:1.2"/>`;
    for (const yy of [top, bottom]) {
      if (p.actor) {
        const cy = yy + 8;
        heads +=
          `<circle cx="${r(p.x)}" cy="${r(cy)}" r="6" style="fill:none;stroke:${C.nodeStroke};stroke-width:1.4"/>` +
          `<path d="M${r(p.x)},${r(cy + 6)} v10 M${r(p.x - 9)},${r(cy + 10)} h18 M${r(p.x)},${r(cy + 16)} l-7,9 M${r(p.x)},${r(cy + 16)} l7,9" style="fill:none;stroke:${C.nodeStroke};stroke-width:1.4"/>` +
          textBlock([p.label], p.x, yy + BOX_H + (yy === top ? 6 : 6), C.text, 500, 12);
      } else {
        heads +=
          `<rect x="${r(p.x - w / 2)}" y="${r(yy)}" width="${r(w)}" height="${BOX_H}" rx="4" style="fill:${C.nodeFill};stroke:${C.nodeStroke};stroke-width:1.3"/>` +
          textBlock([p.label], p.x, yy + BOX_H / 2, C.text, 500);
      }
    }
  });
  const titleSvg = title ? textBlock([title], width / 2, 16, C.text, 600, 14) : '';
  const height = bottom + BOX_H + (parts.some((p) => p.actor) ? 16 : 0) + 16;
  return wrapSvg('sequence', width, height, titleSvg + frameOut.join('') + body + over + heads, markers(id));
}

function byIdX(parts: SPart[], id: string): number {
  return parts.find((p) => p.id === id)?.x ?? 0;
}

// ---------------------------------------------------------------------------
// Pie

function parsePie(src: string): { title: string; showData: boolean; slices: { label: string; value: number }[] } {
  const lines = cleanLines(src);
  const head = /^pie\b\s*(showData)?\s*(?:title\s+(.*))?$/i.exec(lines[0] ?? '');
  let title = head?.[2] ?? '';
  const showData = !!head?.[1];
  const slices: { label: string; value: number }[] = [];
  for (const l of lines.slice(1)) {
    let m: RegExpExecArray | null;
    if ((m = /^title\s+(.*)$/i.exec(l))) title = m[1];
    else if ((m = /^"([^"]*)"\s*:\s*([0-9]*\.?[0-9]+)/.exec(l))) {
      const v = Number(m[2]);
      if (Number.isFinite(v) && v > 0) slices.push({ label: m[1], value: v });
    }
  }
  return { title, showData, slices };
}

function pie(src: string): Rendered | null {
  const { title, showData, slices } = parsePie(src);
  if (!slices.length) return null;
  const total = slices.reduce((s, x) => s + x.value, 0);
  const R = 96;
  const top = title ? 36 : 12;
  const cx = 16 + R;
  const cy = top + R;
  let a0 = -Math.PI / 2;
  let body = '';
  slices.forEach((s, i) => {
    const frac = s.value / total;
    const a1 = a0 + frac * Math.PI * 2;
    const col = PIE[i % PIE.length];
    if (slices.length === 1) {
      body += `<circle cx="${cx}" cy="${cy}" r="${R}" style="fill:${col};stroke:${C.bg};stroke-width:1.5"/>`;
    } else {
      const large = a1 - a0 > Math.PI ? 1 : 0;
      const p0 = { x: cx + R * Math.cos(a0), y: cy + R * Math.sin(a0) };
      const p1 = { x: cx + R * Math.cos(a1), y: cy + R * Math.sin(a1) };
      body += `<path d="M${cx},${cy} L${r(p0.x)},${r(p0.y)} A${R},${R} 0 ${large} 1 ${r(p1.x)},${r(p1.y)} z" style="fill:${col};stroke:${C.bg};stroke-width:1.5"/>`;
    }
    if (frac >= 0.05) {
      const am = (a0 + a1) / 2;
      body += `<text x="${r(cx + R * 0.62 * Math.cos(am))}" y="${r(cy + R * 0.62 * Math.sin(am))}" text-anchor="middle" dominant-baseline="central" style="fill:#fff;font-size:12px;font-weight:600">${Math.round(frac * 100)}%</text>`;
    }
    a0 = a1;
  });
  let ly = top + 8;
  const lx = cx + R + 28;
  let legendW = 0;
  slices.forEach((s, i) => {
    const label = showData ? `${s.label} [${s.value}]` : s.label;
    legendW = Math.max(legendW, textWidth(label, 12));
    body +=
      `<rect x="${lx}" y="${r(ly)}" width="12" height="12" rx="2" style="fill:${PIE[i % PIE.length]}"/>` +
      `<text x="${lx + 18}" y="${r(ly + 6)}" dominant-baseline="central" style="fill:${C.text};font-size:12px">${esc(label)}</text>`;
    ly += 20;
  });
  const width = lx + 18 + legendW + 16;
  const height = Math.max(cy + R + 16, ly + 8);
  const titleSvg = title ? textBlock([title], width / 2, 18, C.text, 600, 14) : '';
  return wrapSvg('pie', width, height, titleSvg + body);
}

// ---------------------------------------------------------------------------
// Export

/** Replaces `var(--x, fallback)` with the values computed on `host`. */
export function resolveVars(svg: string, host: Element | null): string {
  const cs = host && typeof getComputedStyle === 'function' ? getComputedStyle(host) : null;
  const re = /var\(\s*(--[\w-]+)\s*(?:,\s*((?:[^()]|\([^()]*(?:\([^()]*\)[^()]*)*\))*))?\)/;
  let out = svg;
  // Innermost first, repeatedly (fallbacks can nest vars).
  for (let guard = 0; guard < 2000 && re.test(out); guard++) {
    out = out.replace(re, (_m, name: string, fb?: string) => {
      const v = cs?.getPropertyValue(name).trim();
      return v || (fb ?? '').trim() || 'currentColor';
    });
  }
  return out;
}
