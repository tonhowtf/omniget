export type Volume = {
  name: string;
  mount: string;
  device: string;
  file_system: string;
  total: number;
  free: number;
  available: number;
  used: number;
  purgeable: number | null;
  removable: boolean;
  read_only: boolean;
  kind: "ssd" | "hdd" | "unknown";
  holds_home: boolean;
};

export type UsageNode = {
  name: string;
  path?: string;
  bytes: number;
  files: number;
  kind: "dir" | "file" | "other";
  expandable?: boolean;
  count?: number;
  children?: UsageNode[];
};

export type UsageTree = UsageNode & {
  root: string;
  scanned_entries: number;
  errors: { permission_denied: number; other: number };
  cached: boolean;
  scanned_at: number;
};

type LargeFile = {
  id: string;
  label: string;
  detail: string | null;
  path: string;
  group: "video" | "disk_image" | "archive" | "other";
  bytes: number;
  risk: "reversible";
  modified: number | null;
};

export type LargeFiles = { items: LargeFile[]; total_bytes: number; plan_id: string | null; scanned: number };

type DiskHealth = {
  id: string | null;
  name: string | null;
  bus: string | null;
  media: "ssd" | "hdd" | null;
  size: number | null;
  status: "good" | "warning" | "failing" | "unknown";
  smart_status: string | null;
  warnings: string[];
  temperature_c: number | null;
  percent_used?: number | null;
  power_on_hours?: number | null;
  reason?: string;
};

export type Health = { disks: DiskHealth[]; reason?: string };

const PALETTE = ["#0A84FF", "#FF9F0A", "#30D158", "#BF5AF2", "#FF375F", "#64D2FF", "#FFD60A", "#5E5CE6", "#AC8E68", "#FF6961"];
const OTHER_COLOR = "#8E8E93";

export function colorFor(index: number, node: UsageNode): string {
  if (node.kind === "other") return OTHER_COLOR;
  return PALETTE[index % PALETTE.length];
}

export type Segment = { key: "used" | "purgeable" | "free"; bytes: number; fraction: number };

export function volumeSegments(v: Pick<Volume, "total" | "free" | "purgeable">): Segment[] {
  const total = Math.max(1, v.total);
  const purgeable = Math.max(0, v.purgeable ?? 0);
  const free = Math.min(total, Math.max(0, v.free));
  const used = Math.max(0, total - free - purgeable);
  const out: Segment[] = [
    { key: "used", bytes: used, fraction: used / total },
    { key: "purgeable", bytes: purgeable, fraction: purgeable / total },
    { key: "free", bytes: free, fraction: free / total },
  ];
  return out.filter((s) => s.bytes > 0);
}

export function usedFraction(v: Pick<Volume, "total" | "free">): number {
  return v.total > 0 ? Math.min(1, Math.max(0, (v.total - v.free) / v.total)) : 0;
}

export type Tile = { node: UsageNode; index: number; x: number; y: number; w: number; h: number };

export function squarify(nodes: UsageNode[], x: number, y: number, w: number, h: number): Tile[] {
  const items = nodes
    .map((node, index) => ({ node, index }))
    .filter((it) => it.node.bytes > 0)
    .sort((a, b) => b.node.bytes - a.node.bytes);
  const total = items.reduce((s, it) => s + it.node.bytes, 0);
  if (!total || w <= 0 || h <= 0) return [];
  const scale = (w * h) / total;
  const out: Tile[] = [];
  let cx = x;
  let cy = y;
  let cw = w;
  let ch = h;
  let row: { node: UsageNode; index: number; area: number }[] = [];

  const worst = (r: typeof row, side: number) => {
    const s = r.reduce((a, it) => a + it.area, 0);
    let mx = 0;
    let mn = Infinity;
    for (const it of r) {
      mx = Math.max(mx, it.area);
      mn = Math.min(mn, it.area);
    }
    return Math.max((side * side * mx) / (s * s), (s * s) / (side * side * mn));
  };
  const place = (r: typeof row) => {
    const s = r.reduce((a, it) => a + it.area, 0);
    if (cw >= ch) {
      const colW = ch > 0 ? s / ch : 0;
      let yy = cy;
      for (const it of r) {
        const hh = colW > 0 ? it.area / colW : 0;
        out.push({ node: it.node, index: it.index, x: cx, y: yy, w: colW, h: hh });
        yy += hh;
      }
      cx += colW;
      cw -= colW;
    } else {
      const rowH = cw > 0 ? s / cw : 0;
      let xx = cx;
      for (const it of r) {
        const ww = rowH > 0 ? it.area / rowH : 0;
        out.push({ node: it.node, index: it.index, x: xx, y: cy, w: ww, h: rowH });
        xx += ww;
      }
      cy += rowH;
      ch -= rowH;
    }
  };

  for (const it of items) {
    const cell = { node: it.node, index: it.index, area: it.node.bytes * scale };
    const side = Math.min(cw, ch);
    if (!row.length || worst([...row, cell], side) <= worst(row, side)) {
      row.push(cell);
    } else {
      place(row);
      row = [cell];
    }
  }
  if (row.length) place(row);
  return out;
}

export function labelTier(w: number, h: number): "full" | "name" | "none" {
  if (w >= 90 && h >= 40) return "full";
  if (w >= 48 && h >= 20) return "name";
  return "none";
}

export type Arc = { node: UsageNode; depth: number; color: string; a0: number; a1: number; r0: number; r1: number };

export function sunburst(root: UsageNode, maxDepth: number, radius: number, hole: number): Arc[] {
  const out: Arc[] = [];
  const ring = (radius - hole) / Math.max(1, maxDepth);
  const total = Math.max(1, root.bytes);
  const walk = (node: UsageNode, depth: number, a0: number, color: string | null) => {
    if (depth > maxDepth) return;
    let a = a0;
    const kids = [...(node.children ?? [])].sort((x, y) => y.bytes - x.bytes);
    kids.forEach((k, i) => {
      const span = (k.bytes / total) * Math.PI * 2;
      if (span <= 0) return;
      const c = color ?? colorFor(i, k);
      const own = k.kind === "other" ? OTHER_COLOR : c;
      out.push({ node: k, depth, color: own, a0: a, a1: a + span, r0: hole + (depth - 1) * ring, r1: hole + depth * ring });
      if (k.kind === "dir") walk(k, depth + 1, a, c);
      a += span;
    });
  };
  walk(root, 1, 0, null);
  return out;
}

export function arcPath(a0: number, a1: number, r0: number, r1: number): string {
  const full = a1 - a0 >= Math.PI * 2 - 1e-6;
  if (full) a1 = a0 + Math.PI * 2 - 1e-4;
  const p = (a: number, r: number) => `${(r * Math.sin(a)).toFixed(2)},${(-r * Math.cos(a)).toFixed(2)}`;
  const large = a1 - a0 > Math.PI ? 1 : 0;
  return [
    `M${p(a0, r1)}`,
    `A${r1},${r1} 0 ${large} 1 ${p(a1, r1)}`,
    `L${p(a1, r0)}`,
    `A${r0},${r0} 0 ${large} 0 ${p(a0, r0)}`,
    "Z",
  ].join(" ");
}

type Crumb = { name: string; path: string };

export function crumbs(root: string, path: string, rootName: string): Crumb[] {
  const sep = root.includes("\\") && !root.includes("/") ? "\\" : "/";
  const out: Crumb[] = [{ name: rootName, path: root }];
  if (path === root || !path.startsWith(root)) return out;
  const rest = path.slice(root.length).split(/[\\/]/).filter(Boolean);
  let cur = root;
  for (const part of rest) {
    cur = cur.endsWith(sep) ? `${cur}${part}` : `${cur}${sep}${part}`;
    out.push({ name: part, path: cur });
  }
  return out;
}
