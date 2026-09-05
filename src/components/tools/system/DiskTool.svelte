<script lang="ts">
  /** Analisador de disco (estudo 10, Kudu): volumes, treemap e maiores arquivos. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Volume = { path: string; name: string; total: number; free: number };
  type Node = { name: string; path: string; bytes: number; is_dir: boolean; files: number; children?: Node[] };
  type Scan = { root: Node; largest: { path: string; bytes: number }[]; scanned: number; skipped: number };

  let volumes = $state<Volume[]>([]);
  let scan = $state<Scan | null>(null);
  let trail = $state<Node[]>([]);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let tab = $state<"map" | "largest">("map");

  let unlisten: (() => void) | null = null;
  onMount(async () => {
    unlisten = await onToolProgress((p) => { if (p.id === "disk") progress = p; });
    try { volumes = await invoke<Volume[]>("tool_disk_volumes"); } catch (e) { showToast("error", errText(e)); }
  });
  onDestroy(() => unlisten?.());

  async function run(root: string) {
    if (busy) return;
    busy = true; scan = null; trail = []; progress = null;
    try {
      scan = await invoke<Scan>("tool_disk_scan", { root, depth: 5, children: 40 });
      trail = [scan.root];
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; progress = null; }
  }
  async function pick() { const d = await pickDir(); if (d) run(d); }

  let current = $derived(trail[trail.length - 1]);

  // Treemap "squarified" (Bruls, Huizing, van Wijk) numa caixa 1000×600.
  type Rect = { node: Node; x: number; y: number; w: number; h: number; color: string };
  const COLORS = ["#5AA9FF", "#FF6B6B", "#4CD964", "#FFB340", "#C77DFF", "#48CFDF", "#FF5E7A", "#D8A15C", "#6E8CFF", "#FFD426", "#A3A3A8"];
  function squarify(nodes: Node[], x: number, y: number, w: number, h: number): Rect[] {
    const items = nodes.filter((n) => n.bytes > 0).sort((a, b) => b.bytes - a.bytes);
    const total = items.reduce((s, n) => s + n.bytes, 0);
    if (!total || w <= 0 || h <= 0) return [];
    const out: Rect[] = [];
    let row: Node[] = [];
    let cx = x, cy = y, cw = w, ch = h;
    const area = (n: Node) => (n.bytes / total) * w * h;
    const worst = (r: Node[], side: number) => {
      const s = r.reduce((a, n) => a + area(n), 0);
      const mx = Math.max(...r.map(area)), mn = Math.min(...r.map(area));
      return Math.max((side * side * mx) / (s * s), (s * s) / (side * side * mn));
    };
    const layout = (r: Node[]) => {
      const s = r.reduce((a, n) => a + area(n), 0);
      const vertical = cw >= ch; // fatia numa coluna à esquerda
      if (vertical) {
        const colW = s / ch;
        let yy = cy;
        for (const n of r) { const hh = area(n) / colW; out.push({ node: n, x: cx, y: yy, w: colW, h: hh, color: "" }); yy += hh; }
        cx += colW; cw -= colW;
      } else {
        const rowH = s / cw;
        let xx = cx;
        for (const n of r) { const ww = area(n) / rowH; out.push({ node: n, x: xx, y: cy, w: ww, h: rowH, color: "" }); xx += ww; }
        cy += rowH; ch -= rowH;
      }
    };
    for (const n of items) {
      const side = Math.min(cw, ch);
      if (!row.length || worst([...row, n], side) <= worst(row, side)) row.push(n);
      else { layout(row); row = [n]; }
    }
    if (row.length) layout(row);
    let i = 0;
    for (const r of out) r.color = COLORS[i++ % COLORS.length];
    return out;
  }
  let rects = $derived(current ? squarify(current.children ?? [], 0, 0, 1000, 600) : []);

  function open(n: Node) { if (n.is_dir && n.children?.length) trail = [...trail, n]; }
  function up(i: number) { trail = trail.slice(0, i + 1); }
  async function trashPath(p: string) {
    if (!p) return;
    try {
      const r = await invoke<{ ok: string[]; failed: string[] }>("tool_disk_trash", { paths: [p] });
      if (r.ok.length) { showToast("success", $t("tools.disk.trashed") as string); if (scan) run(scan.root.path); }
      else showToast("error", $t("tools.common.failed") as string);
    } catch (e) { showToast("error", errText(e)); }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      {#each volumes as v (v.path)}
        {@const used = v.total - v.free}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{v.name} <span class="dim mono">{v.path}</span></div>
            <div class="group-row-sub">{fmtBytes(used)} {$t("tools.disk.used")} · {fmtBytes(v.free)} {$t("tools.disk.free")} · {fmtBytes(v.total)}</div>
            <div class="progress"><div class="progress-fill" class:warn={used / v.total > 0.9} style:width="{Math.round((used / v.total) * 100)}%"></div></div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={() => run(v.path)}>{$t("tools.disk.analyze")}</button></div>
        </div>
      {/each}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.disk.folder")}</div><div class="group-row-sub">{$t("tools.disk.folder_hint")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-primary btn-sm" type="button" disabled={busy} onclick={pick}>{$t("tools.common.choose")}</button></div>
      </div>
      {#if busy}
        <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{progress ? `${progress.done} · ${progress.message ?? ""}` : "…"}</div><div class="progress"><div class="progress-fill indeterminate"></div></div></div></div>
      {/if}
    </div>
  </section>

  {#if scan && current}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="crumbs">
              {#each trail as n, i (n.path)}
                {#if i > 0}<span class="dim">›</span>{/if}
                <button class="crumb" type="button" onclick={() => up(i)}>{n.name}</button>
              {/each}
            </div>
            <div class="group-row-sub">{fmtBytes(current.bytes)} · {current.files} {$t("tools.common.files")} · {scan.scanned} {$t("tools.disk.scanned")}{#if scan.skipped} · {scan.skipped} {$t("tools.disk.skipped")}{/if}</div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-sm" class:btn-secondary={tab === "map"} class:btn-ghost={tab !== "map"} type="button" onclick={() => (tab = "map")}>{$t("tools.disk.map")}</button>
            <button class="btn btn-sm" class:btn-secondary={tab === "largest"} class:btn-ghost={tab !== "largest"} type="button" onclick={() => (tab = "largest")}>{$t("tools.disk.largest")}</button>
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(current.path)}>{$t("tools.common.reveal")}</button>
          </div>
        </div>
      </div>
    </section>

    {#if tab === "map"}
      <div class="treemap" role="group">
        <svg viewBox="0 0 1000 600" preserveAspectRatio="none" aria-hidden="true">
          {#each rects as r (r.node.path + r.node.name)}
            <g role="button" tabindex="0" onclick={() => open(r.node)} onkeydown={(e) => { if (e.key === "Enter") open(r.node); }} class:clickable={r.node.is_dir && !!r.node.children?.length}>
              <rect x={r.x + 1} y={r.y + 1} width={Math.max(0, r.w - 2)} height={Math.max(0, r.h - 2)} rx="4" fill={r.color} fill-opacity={r.node.is_dir ? 0.85 : 0.55} />
              {#if r.w > 60 && r.h > 22}
                <text x={r.x + 8} y={r.y + 18} font-size="14" fill="#fff" font-weight="600">{r.node.name.slice(0, Math.floor(r.w / 9))}</text>
                {#if r.h > 40}<text x={r.x + 8} y={r.y + 36} font-size="12" fill="#fff" opacity="0.9">{fmtBytes(r.node.bytes)}</text>{/if}
              {/if}
              <title>{r.node.name} · {fmtBytes(r.node.bytes)}</title>
            </g>
          {/each}
        </svg>
      </div>
      <section>
        <div class="group">
          {#each (current.children ?? []).slice(0, 40) as n, i (n.path + n.name)}
            <div class="group-row">
              <span class="swatch" style:background={COLORS[i % COLORS.length]}></span>
              <div class="group-row-content">
                <div class="group-row-title">{n.name}{#if !n.is_dir} <span class="dim">{$t("tools.common.file")}</span>{/if}</div>
                <div class="group-row-sub">{fmtBytes(n.bytes)} · {Math.round((n.bytes / Math.max(1, current.bytes)) * 100)}%{#if n.is_dir} · {n.files} {$t("tools.common.files")}{/if}</div>
              </div>
              <div class="group-row-trailing btn-row">
                {#if n.is_dir && n.children?.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => open(n)}>{$t("tools.common.open")}</button>{/if}
                {#if n.path}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(n.path)}>{$t("tools.common.reveal")}</button><button class="btn btn-ghost btn-sm danger" type="button" onclick={() => trashPath(n.path)}>{$t("tools.disk.trash")}</button>{/if}
              </div>
            </div>
          {/each}
        </div>
      </section>
    {:else}
      <section>
        <div class="group">
          {#each scan.largest as f (f.path)}
            <div class="group-row">
              <div class="group-row-content"><div class="group-row-title">{baseName(f.path)}</div><div class="group-row-sub mono">{f.path}</div></div>
              <div class="group-row-trailing btn-row"><strong>{fmtBytes(f.bytes)}</strong><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(f.path)}>{$t("tools.common.reveal")}</button><button class="btn btn-ghost btn-sm danger" type="button" onclick={() => trashPath(f.path)}>{$t("tools.disk.trash")}</button></div>
            </div>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .progress { margin-top: var(--space-2); }
  .progress-fill.warn { background: var(--danger); }
  .progress-fill.indeterminate { width: 40%; animation: slide 1.2s ease-in-out infinite alternate; }
  @keyframes slide { from { margin-left: 0; } to { margin-left: 60%; } }
  .crumbs { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-1); }
  .crumb { background: none; border: 0; padding: 0; font: inherit; font-weight: 600; color: var(--accent-hi); cursor: pointer; }
  .crumb:hover { text-decoration: underline; }
  .treemap { width: 100%; aspect-ratio: 5 / 3; border-radius: var(--radius-lg); overflow: hidden; background: var(--surface); box-shadow: inset 0 0 0 var(--hairline) var(--content-border); }
  .treemap svg { width: 100%; height: 100%; display: block; }
  .treemap g.clickable { cursor: pointer; }
  .treemap g:hover rect { fill-opacity: 1; }
  .treemap text { pointer-events: none; font-family: var(--font-display); }
  .swatch { width: 10px; height: 10px; border-radius: 3px; flex: none; margin-right: var(--space-2); }
  .danger { color: var(--danger); }
</style>
