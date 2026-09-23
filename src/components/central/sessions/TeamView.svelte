<script lang="ts">
  /**
   * Time de uma sessão: o líder no centro, subagentes e colegas em anéis
   * (ordem de início), arestas de spawn e de mensagem com espessura pelo
   * peso. Clique (ou Enter) num nó abre o painel com tarefas, mensagens,
   * tools e tempo; abaixo, a linha do tempo de cada agente.
   */
  import { t } from "$lib/i18n";
  import {
    compact,
    dateTime,
    duration,
    errText,
    sessionHref,
    sessionsTeam,
    toMs,
    totalTokens,
    type TeamGraph,
    type TeamNode,
  } from "$lib/central/sessions";

  interface Props {
    tool: string;
    id: string;
  }

  let { tool, id }: Props = $props();

  let g = $state<TeamGraph | null>(null);
  let error = $state("");
  let selected = $state<string | null>(null);
  let hoverEdge = $state<number>(-1);

  $effect(() => {
    g = null;
    error = "";
    sessionsTeam(tool, id)
      .then((r) => {
        g = r;
        selected = r.nodes.find((n) => n.kind === "lead")?.id ?? r.nodes[0]?.id ?? null;
      })
      .catch((e) => (error = errText(e)));
  });

  const C = 400;
  const RING0 = 120;
  const RING_STEP = 78;
  const SPACING = 34;

  type Pos = { x: number; y: number; r: number };

  let layout = $derived.by(() => {
    const pos = new Map<string, Pos>();
    if (!g) return { pos, size: 2 * C };
    const lead = g.nodes.find((n) => n.kind === "lead") ?? g.nodes[0];
    const maxCalls = Math.max(1, ...g.nodes.map((n) => n.tool_calls));
    const radius = (n: TeamNode) => (n.kind === "lead" ? 24 : 7 + 13 * Math.sqrt(n.tool_calls / maxCalls));
    const byId = new Map(g.nodes.map((n) => [n.id, n]));
    const depth = (n: TeamNode): number => {
      let d = 0;
      let cur: TeamNode | undefined = n;
      const seen = new Set<string>();
      while (cur && cur.spawned_by && !seen.has(cur.id) && d < 20) {
        seen.add(cur.id);
        cur = byId.get(cur.spawned_by);
        d++;
      }
      return Math.max(1, d);
    };
    if (lead) pos.set(lead.id, { x: C, y: C, r: radius(lead) });
    const rest = g.nodes
      .filter((n) => n !== lead)
      .sort((a, b) => depth(a) - depth(b) || toMs(a.started) - toMs(b.started) || a.name.localeCompare(b.name));
    let ring = 0;
    let i = 0;
    let maxR = RING0;
    while (i < rest.length) {
      const rr = RING0 + ring * RING_STEP;
      const cap = Math.max(6, Math.floor((2 * Math.PI * rr) / SPACING));
      const take = rest.slice(i, i + cap);
      take.forEach((n, k) => {
        const ang = -Math.PI / 2 + (2 * Math.PI * k) / take.length + (ring % 2 ? Math.PI / take.length : 0);
        pos.set(n.id, { x: C + rr * Math.cos(ang), y: C + rr * Math.sin(ang), r: radius(n) });
      });
      maxR = rr;
      i += cap;
      ring++;
    }
    return { pos, size: 2 * (maxR + 60) };
  });

  let view = $derived(`${C - layout.size / 2} ${C - layout.size / 2} ${layout.size} ${layout.size}`);
  let maxCount = $derived(Math.max(1, ...(g?.edges ?? []).map((e) => e.count)));
  let showLabels = $derived((g?.nodes.length ?? 0) <= 36);

  function edgePath(i: number): string {
    const e = g!.edges[i];
    const a = layout.pos.get(e.from);
    const b = layout.pos.get(e.to);
    if (!a || !b) return "";
    if (e.kind === "spawn") return `M${a.x} ${a.y}L${b.x} ${b.y}`;
    // Mensagem: curva para não sobrepor o spawn; ida e volta ficam separadas.
    const mx = (a.x + b.x) / 2;
    const my = (a.y + b.y) / 2;
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    const len = Math.hypot(dx, dy) || 1;
    const bend = Math.min(60, len * 0.25);
    return `M${a.x} ${a.y}Q${mx - (dy / len) * bend} ${my + (dx / len) * bend} ${b.x} ${b.y}`;
  }

  let node = $derived(g?.nodes.find((n) => n.id === selected) ?? null);
  let nodeTasks = $derived(node && g ? g.tasks.filter((tk) => tk.owner === node!.name || tk.owner === node!.id || tk.created_by === node!.name || tk.created_by === node!.id) : []);
  let nodeMsgs = $derived(node && g ? g.messages.filter((m) => m.from === node!.id || m.to === node!.id || m.from === node!.name || m.to === node!.name) : []);

  /** Subagente sem nome próprio usa a descrição da tarefa (o tipo vira subtítulo). */
  function label(n: TeamNode): string {
    return n.kind !== "lead" && n.description && (!n.name || n.name === n.agent_type) ? n.description : n.name;
  }

  function nodeName(idOrName: string): string {
    const n = g?.nodes.find((x) => x.id === idOrName);
    return n ? label(n) : idOrName;
  }

  function onNodeKey(e: KeyboardEvent, nid: string) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      selected = nid;
    }
  }

  // ---- linha do tempo ----
  let span = $derived.by(() => {
    const xs = (g?.nodes ?? []).flatMap((n) => [toMs(n.started), toMs(n.ended)]).filter((v) => v > 0);
    const a = Math.min(...xs);
    const b = Math.max(...xs);
    return xs.length ? { a, b: Math.max(b, a + 1) } : { a: 0, b: 1 };
  });
  let timeline = $derived([...(g?.nodes ?? [])].filter((n) => toMs(n.started) > 0).sort((a, b) => toMs(a.started) - toMs(b.started)));
</script>

{#if error}
  <p class="err">{error}</p>
{:else if !g}
  <p class="muted">{$t("llm.central.sessions.loading")}</p>
{:else if g.nodes.length <= 1}
  <div class="empty-state">
    <div class="empty-state-title">{$t("llm.central.sessions.team.empty_title")}</div>
    <div class="empty-state-hint">{$t("llm.central.sessions.team.empty_hint")}</div>
  </div>
{:else}
  <div class="team">
    <div class="graph surface-card">
      <p class="graph-head">
        {g.is_team ? $t("llm.central.sessions.team.agent_team") : $t("llm.central.sessions.team.subagents")}
        · {g.nodes.length - 1} {$t("llm.central.sessions.team.members")} · {g.edges.length} {$t("llm.central.sessions.team.links")}
      </p>
      <svg viewBox={view} role="group" aria-label={$t("llm.central.sessions.team.graph")}>
        <defs>
          <marker id="team-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
            <path d="M0 0L8 4L0 8Z" class="arrow" />
          </marker>
        </defs>
        {#each g.edges as e, i (i)}
          {@const on = selected && (e.from === selected || e.to === selected)}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <path
            d={edgePath(i)}
            class="edge {e.kind}"
            class:on
            class:dim={selected && !on}
            stroke-width={1 + 3 * Math.log2(1 + e.count) / Math.log2(1 + maxCount)}
            marker-end={e.kind === "message" ? "url(#team-arrow)" : undefined}
            onpointerenter={() => (hoverEdge = i)}
            onpointerleave={() => (hoverEdge = -1)}
          >
            <title>{nodeName(e.from)} → {nodeName(e.to)} · {e.kind === "spawn" ? $t("llm.central.sessions.team.spawn") : $t("llm.central.sessions.team.message")} ×{e.count}{e.samples[0] ? `\n${e.samples[0]}` : ""}</title>
          </path>
        {/each}
        {#each g.nodes as n (n.id)}
          {@const p = layout.pos.get(n.id)}
          {#if p}
            <g
              class="node {n.kind}"
              class:sel={selected === n.id}
              transform="translate({p.x} {p.y})"
              role="button"
              tabindex="0"
              aria-label={`${label(n)}, ${$t(`llm.central.sessions.team.kind_${n.kind}`)}, ${n.tool_calls} ${$t("llm.central.sessions.tool_calls_unit")}`}
              aria-pressed={selected === n.id}
              onclick={() => (selected = n.id)}
              onkeydown={(e) => onNodeKey(e, n.id)}
            >
              <circle r={p.r + 6} class="hit" />
              <circle r={p.r} class="dot" />
              {#if showLabels || selected === n.id || n.kind === "lead"}
                <text y={p.r + 13} class="lbl">{label(n).length > 22 ? `${label(n).slice(0, 21)}…` : label(n)}</text>
              {/if}
              <title>{label(n)}{n.agent_type ? ` (${n.agent_type})` : ""}</title>
            </g>
          {/if}
        {/each}
      </svg>
      {#if hoverEdge >= 0 && g.edges[hoverEdge]}
        {@const e = g.edges[hoverEdge]}
        <p class="edge-read" role="status">{nodeName(e.from)} → {nodeName(e.to)} · ×{e.count}{e.samples[0] ? ` · “${e.samples[0].slice(0, 120)}”` : ""}</p>
      {/if}
      <ul class="legend">
        <li><i class="lg lead"></i>{$t("llm.central.sessions.team.kind_lead")}</li>
        <li><i class="lg teammate"></i>{$t("llm.central.sessions.team.kind_teammate")}</li>
        <li><i class="lg subagent"></i>{$t("llm.central.sessions.team.kind_subagent")}</li>
        <li><i class="ln"></i>{$t("llm.central.sessions.team.spawn")}</li>
        <li><i class="ln msg"></i>{$t("llm.central.sessions.team.message")}</li>
      </ul>
    </div>

    <aside class="panel surface-card" aria-live="polite">
      {#if node}
        <h3>{label(node)}</h3>
        <p class="kind">{$t(`llm.central.sessions.team.kind_${node.kind}`)}{node.agent_type ? ` · ${node.agent_type}` : ""}</p>
        {#if node.description && label(node) !== node.description}<p class="desc">{node.description}</p>{/if}
        <dl class="facts">
          <div><dt>{$t("llm.central.sessions.team.time")}</dt><dd>{node.started ? `${dateTime(node.started)} · ${duration(toMs(node.ended) - toMs(node.started))}` : "—"}</dd></div>
          <div><dt>{$t("llm.central.sessions.turns_unit")}</dt><dd>{node.turns}</dd></div>
          <div><dt>{$t("llm.central.sessions.tool_calls_unit")}</dt><dd>{node.tool_calls}</dd></div>
          <div><dt>{$t("llm.central.sessions.team.sent_received")}</dt><dd>{node.messages_sent} / {node.messages_received}</dd></div>
          <div><dt>tokens</dt><dd>{compact(totalTokens(node.usage))}</dd></div>
          {#if node.spawned_by}<div><dt>{$t("llm.central.sessions.team.spawned_by")}</dt><dd>{nodeName(node.spawned_by)}</dd></div>{/if}
        </dl>
        {#if node.session_id && node.kind !== "lead"}
          <a class="btn btn-secondary btn-sm" href={sessionHref(tool, node.session_id)}>{$t("llm.central.sessions.team.open_transcript")}</a>
        {/if}
        {#if node.tools_used.length}
          <details open>
            <summary>{$t("llm.central.sessions.analysis.tools")} ({node.tools_used.length})</summary>
            <p class="chips">{#each node.tools_used as [n, c] (n)}<span class="chip">{n} <strong>{c}</strong></span>{/each}</p>
          </details>
        {/if}
        {#if nodeTasks.length}
          <details open>
            <summary>{$t("llm.central.sessions.team.tasks")} ({nodeTasks.length})</summary>
            <ul class="tasks">
              {#each nodeTasks as tk (tk.id)}
                <li><span class="st">{tk.status ?? "?"}</span> {tk.subject ?? tk.id}</li>
              {/each}
            </ul>
          </details>
        {/if}
        {#if nodeMsgs.length}
          <details>
            <summary>{$t("llm.central.sessions.team.messages")} ({nodeMsgs.length})</summary>
            <ul class="msgs">
              {#each nodeMsgs.slice(0, 80) as m, i (i)}
                <li><span class="who">{nodeName(m.from)} → {nodeName(m.to)}</span> <span class="when">{new Date(m.ts).toLocaleTimeString()}</span><p>{m.text.slice(0, 400)}</p></li>
              {/each}
            </ul>
          </details>
        {/if}
      {/if}
    </aside>
  </div>

  <section class="tl surface-card">
    <h3>{$t("llm.central.sessions.team.timeline")}</h3>
    <ul>
      {#each timeline as n (n.id)}
        {@const a = ((toMs(n.started) - span.a) / (span.b - span.a)) * 100}
        {@const w = Math.max(0.6, ((Math.max(toMs(n.ended), toMs(n.started)) - toMs(n.started)) / (span.b - span.a)) * 100)}
        <li>
          <button type="button" class="tl-row" class:sel={selected === n.id} onclick={() => (selected = n.id)}>
            <span class="tl-name" title={label(n)}>{label(n)}</span>
            <span class="tl-track"><i class={n.kind} style:left="{a}%" style:width="{Math.min(w, 100 - a)}%"></i></span>
            <span class="tl-dur">{duration(toMs(n.ended) - toMs(n.started))}</span>
          </button>
        </li>
      {/each}
    </ul>
    {#if g.tasks.length}
      <h3>{$t("llm.central.sessions.team.tasks")}</h3>
      <ul class="tasks all">
        {#each g.tasks as tk (tk.id)}
          <li><span class="st">{tk.status ?? "?"}</span> <strong>{tk.subject ?? tk.id}</strong>{tk.owner ? ` · ${nodeName(tk.owner)}` : ""}</li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  .team {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 300px;
    gap: var(--space-3);
    align-items: start;
  }
  .graph {
    padding: var(--space-3);
    min-width: 0;
  }
  .graph-head {
    margin: 0 0 6px;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  svg {
    width: 100%;
    height: auto;
    max-height: 560px;
    display: block;
    pointer-events: all;
  }
  .edge {
    fill: none;
    stroke: var(--fill-4);
    transition: opacity var(--duration-fast) var(--ease-out);
  }
  .edge.message {
    stroke: color-mix(in srgb, var(--viz-seq) 60%, transparent);
  }
  .edge.on {
    stroke: var(--viz-seq);
  }
  .edge.dim {
    opacity: 0.25;
  }
  .arrow {
    fill: color-mix(in srgb, var(--viz-seq) 80%, transparent);
  }
  .node {
    cursor: pointer;
    outline: none;
  }
  .hit {
    fill: transparent;
  }
  .dot {
    fill: var(--surface-hi);
    stroke: var(--text-dim);
    stroke-width: 1.5;
  }
  .node.lead .dot {
    fill: var(--viz-seq);
    stroke: var(--surface);
    stroke-width: 2;
  }
  .node.teammate .dot {
    fill: color-mix(in srgb, var(--viz-seq) 35%, var(--surface));
    stroke: var(--viz-seq);
  }
  .node.sel .dot,
  .node:focus-visible .dot {
    stroke: var(--text);
    stroke-width: 3;
  }
  .lbl {
    font-size: 11px;
    fill: var(--text-muted);
    text-anchor: middle;
  }
  .edge-read {
    margin: 4px 0 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .legend {
    list-style: none;
    margin: 8px 0 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .legend li {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .lg {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.5px solid var(--text-dim);
    background: var(--surface-hi);
  }
  .lg.lead {
    background: var(--viz-seq);
    border-color: transparent;
  }
  .lg.teammate {
    background: color-mix(in srgb, var(--viz-seq) 35%, var(--surface));
    border-color: var(--viz-seq);
  }
  .ln {
    width: 16px;
    height: 2px;
    background: var(--fill-4);
  }
  .ln.msg {
    background: color-mix(in srgb, var(--viz-seq) 60%, transparent);
  }
  .panel {
    padding: var(--space-3) var(--space-4);
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: var(--text-sm);
    color: var(--text-muted);
    max-height: 640px;
    overflow: auto;
  }
  .panel h3 {
    margin: 0;
    font-size: var(--text-md);
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .kind {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .desc {
    margin: 0;
  }
  .facts {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .facts div {
    display: flex;
    justify-content: space-between;
    gap: 8px;
  }
  .facts dt {
    color: var(--text-dim);
  }
  .facts dd {
    margin: 0;
    color: var(--text);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  details summary {
    cursor: pointer;
    font-weight: 600;
    color: var(--text);
    font-size: var(--text-sm);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin: 6px 0 0;
  }
  .chip {
    font-size: var(--text-xs);
    background: var(--fill-1);
    border-radius: var(--radius-full);
    padding: 1px 8px;
  }
  .chip strong {
    color: var(--text);
  }
  .tasks,
  .msgs {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .st {
    font-size: var(--text-caption);
    background: var(--fill-2);
    border-radius: var(--radius-xs);
    padding: 0 4px;
  }
  .msgs li p {
    margin: 2px 0 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--text);
    font-size: var(--text-xs);
  }
  .who {
    font-weight: 600;
    font-size: var(--text-xs);
  }
  .when {
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .tl {
    margin-top: var(--space-3);
    padding: var(--space-3) var(--space-4);
  }
  .tl h3 {
    margin: 0 0 8px;
    font-size: var(--text-sm);
    color: var(--text);
  }
  .tl ul {
    list-style: none;
    margin: 0 0 var(--space-3);
    padding: 0;
    max-height: 360px;
    overflow: auto;
  }
  .tl-row {
    width: 100%;
    display: grid;
    grid-template-columns: minmax(80px, 180px) 1fr 64px;
    gap: 8px;
    align-items: center;
    padding: 3px 4px;
    background: none;
    border: none;
    font: inherit;
    font-size: var(--text-xs);
    color: var(--text-muted);
    cursor: pointer;
    border-radius: var(--radius-sm);
    text-align: left;
  }
  .tl-row.sel {
    background: var(--fill-1);
    color: var(--text);
  }
  .tl-row:focus-visible {
    outline: var(--focus-ring);
  }
  .tl-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tl-track {
    position: relative;
    height: 8px;
    background: var(--viz-empty);
    border-radius: var(--radius-full);
  }
  .tl-track i {
    position: absolute;
    top: 0;
    bottom: 0;
    border-radius: var(--radius-full);
    background: var(--fill-4);
  }
  .tl-track i.lead {
    background: var(--viz-seq);
  }
  .tl-track i.teammate {
    background: color-mix(in srgb, var(--viz-seq) 55%, var(--viz-empty));
  }
  .tl-dur {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .err {
    color: var(--danger);
  }
  .muted {
    color: var(--text-dim);
  }
  @media (max-width: 860px) {
    .team {
      grid-template-columns: 1fr;
    }
  }
</style>
