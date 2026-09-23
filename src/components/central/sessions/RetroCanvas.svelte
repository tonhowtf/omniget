<script lang="ts">
  /**
   * "Replay" do período em canvas 2D próprio.
   *
   * Um cursor de data anda do primeiro ao último dia; no centro fica você,
   * à esquerda os modelos, à direita as tools. Cada nó nasce no dia do
   * primeiro uso (marco `new_model`) ou no primeiro dia ativo e cresce com a
   * atividade acumulada do período até chegar ao seu total; feixes saem do
   * centro para os nós na proporção das mensagens e tool calls do dia. Os
   * marcos aparecem como avisos. Play/pausa/reiniciar e uma régua para
   * arrastar o cursor (teclado incluso). Com `prefers-reduced-motion` não há
   * autoplay nem feixes: a régua mostra qualquer dia parado.
   *
   * O laço de quadros só existe enquanto toca e a janela está visível.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { compact, fill, type Milestone, type Retro } from "$lib/central/sessions";

  interface Props {
    retro: Retro;
    milestoneText: (m: Milestone) => string;
  }

  let { retro, milestoneText }: Props = $props();

  const DURATION_MS = 14000;

  let canvas: HTMLCanvasElement | undefined = $state();
  let box: HTMLDivElement | undefined = $state();
  let playing = $state(false);
  let progress = $state(0); // 0..1
  let reduced = $state(false);

  type NodeSpec = { name: string; kind: "model" | "tool"; total: number; appear: number; x: number; y: number };
  type Beam = { node: number; t: number; speed: number };

  let days = $derived(retro.heatmap.length ? retro.heatmap : []);
  let dayCount = $derived(Math.max(1, days.length));

  /** Atividade acumulada normalizada por dia (mensagens + tool calls). */
  let cum = $derived.by(() => {
    const out: number[] = [];
    let acc = 0;
    for (const d of days) {
      acc += d.messages + d.tool_calls;
      out.push(acc);
    }
    const tot = acc || 1;
    return out.map((v) => v / tot);
  });

  let firstActive = $derived(Math.max(0, days.findIndex((d) => d.messages > 0 || d.tool_calls > 0)));

  let nodes = $derived.by<NodeSpec[]>(() => {
    const dayIdx = new Map(days.map((d, i) => [d.date, i]));
    const firstUse = new Map<string, number>();
    for (const m of retro.milestones) if (m.kind === "new_model") firstUse.set(m.label, dayIdx.get(m.date) ?? firstActive);
    const models = retro.top_models.slice(0, 8).map((r) => ({ name: r.name, kind: "model" as const, total: r.count, appear: firstUse.get(r.name) ?? firstActive, x: 0, y: 0 }));
    const tools = retro.top_tools.slice(0, 10).map((r) => ({ name: r.name, kind: "tool" as const, total: r.count, appear: firstActive, x: 0, y: 0 }));
    return [...models, ...tools];
  });

  let maxTotal = $derived(Math.max(1, ...nodes.map((n) => n.total)));

  let dayIndex = $derived(Math.min(dayCount - 1, Math.floor(progress * dayCount)));
  let curDay = $derived(days[dayIndex] ?? null);
  let running = $derived.by(() => {
    let messages = 0;
    let tokens = 0;
    let sessions = 0;
    let tools = 0;
    for (let i = 0; i <= dayIndex && i < days.length; i++) {
      messages += days[i].messages;
      tokens += days[i].tokens;
      sessions += days[i].sessions;
      tools += days[i].tool_calls;
    }
    return { messages, tokens, sessions, tools };
  });

  let toasts = $derived(
    retro.milestones.filter((m) => {
      const i = days.findIndex((d) => d.date === m.date);
      return i >= 0 && i <= dayIndex && dayIndex - i < Math.max(4, dayCount / 12);
    }).slice(-3),
  );

  let beams: Beam[] = [];
  let raf = 0;
  let last = 0;
  let colors = { accent: "#ff9f0a", text: "#eee", dim: "#999", faint: "#555", surface: "#222", grid: "#333" };
  let W = 0;
  let H = 0;

  function readColors() {
    if (!canvas) return;
    const cs = getComputedStyle(canvas);
    const v = (n: string, d: string) => cs.getPropertyValue(n).trim() || d;
    colors = {
      accent: v("--accent", colors.accent),
      text: v("--secondary", colors.text),
      dim: v("--tertiary", colors.dim),
      faint: v("--tertiary", colors.faint),
      surface: v("--surface-src", colors.surface),
      grid: v("--content-border", colors.grid),
    };
  }

  function layout() {
    if (!canvas || !box) return;
    const dpr = window.devicePixelRatio || 1;
    W = box.clientWidth;
    H = Math.max(340, Math.min(500, Math.round(W * 0.6)));
    canvas.width = Math.round(W * dpr);
    canvas.height = Math.round(H * dpr);
    canvas.style.height = `${H}px`;
    const ctx = canvas.getContext("2d");
    ctx?.setTransform(dpr, 0, 0, dpr, 0, 0);
    const cx = W / 2;
    const cy = centerY();
    const rx = Math.min(W * 0.3, 380);
    const ry = ry0();
    // Coluna vertical com espaço igual (rótulos não se chocam), levemente
    // curvada para o centro nas pontas.
    const place = (list: NodeSpec[], side: -1 | 1) => {
      list.forEach((n, i) => {
        const tt = list.length === 1 ? 0 : -1 + (2 * i) / (list.length - 1);
        n.x = cx + side * rx * (0.72 + 0.28 * Math.sqrt(1 - tt * tt));
        n.y = cy + ry * tt;
      });
    };
    place(nodes.filter((n) => n.kind === "model"), -1);
    place(nodes.filter((n) => n.kind === "tool"), 1);
    draw();
  }

  // Faixa de cima fica para a data e os avisos; a de baixo, para a régua.
  const TOP = 104;
  const BOTTOM = 56;

  function centerY(): number {
    return (TOP + (H - BOTTOM)) / 2;
  }

  /** Metade da altura útil das colunas de nós. */
  function ry0(): number {
    return Math.max(60, (H - BOTTOM - TOP) / 2 - 10);
  }

  function nodeValue(n: NodeSpec): number {
    if (dayIndex < n.appear) return 0;
    const base = n.appear > 0 ? cum[n.appear - 1] ?? 0 : 0;
    const span = 1 - base || 1;
    const f = Math.max(0, Math.min(1, ((cum[dayIndex] ?? 1) - base) / span));
    return n.total * f;
  }

  function draw() {
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, W, H);
    const cx = W / 2;
    const cy = centerY();

    // Arestas finas centro → nós visíveis.
    for (const n of nodes) {
      const v = nodeValue(n);
      if (v <= 0) continue;
      ctx.strokeStyle = colors.grid;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.lineTo(n.x, n.y);
      ctx.stroke();
    }

    // Feixes.
    for (const b of beams) {
      const n = nodes[b.node];
      if (!n) continue;
      const x = cx + (n.x - cx) * b.t;
      const y = cy + (n.y - cy) * b.t;
      ctx.fillStyle = n.kind === "model" ? colors.accent : colors.text;
      ctx.globalAlpha = 0.35 + 0.65 * (1 - Math.abs(0.5 - b.t) * 2) * 0.8;
      ctx.beginPath();
      ctx.arc(x, y, 2, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.globalAlpha = 1;

    // Nós.
    ctx.textBaseline = "middle";
    ctx.font = `500 11px ${getComputedStyle(canvas).fontFamily || "system-ui"}`;
    for (const n of nodes) {
      const v = nodeValue(n);
      if (v <= 0) continue;
      const r = 4 + Math.min(18, ry0() / Math.max(2, nodes.length / 2)) * Math.sqrt(v / maxTotal);
      ctx.fillStyle = n.kind === "model" ? colors.accent : colors.surface;
      ctx.strokeStyle = n.kind === "model" ? colors.surface : colors.dim;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(n.x, n.y, r, 0, Math.PI * 2);
      ctx.fill();
      ctx.stroke();
      ctx.fillStyle = colors.text;
      ctx.textAlign = n.kind === "model" ? "right" : "left";
      const lx = n.kind === "model" ? n.x - r - 6 : n.x + r + 6;
      const label = n.name.length > 22 ? `${n.name.slice(0, 21)}…` : n.name;
      ctx.fillText(label, lx, n.y - 6);
      ctx.fillStyle = colors.dim;
      ctx.fillText(compact(Math.round(v)), lx, n.y + 8);
    }

    // Centro.
    const act = curDay ? curDay.messages + curDay.tool_calls : 0;
    ctx.fillStyle = colors.text;
    ctx.beginPath();
    ctx.arc(cx, cy, 10 + Math.min(10, Math.sqrt(act) / 2), 0, Math.PI * 2);
    ctx.fill();

    // Régua de atividade (heat) + cursor.
    const y0 = H - 26;
    const bw = W / dayCount;
    const peak = Math.max(1, ...days.map((d) => d.messages));
    days.forEach((d, i) => {
      const h = 3 + 14 * Math.sqrt(d.messages / peak);
      const lit = i <= dayIndex && d.messages > 0;
      ctx.fillStyle = lit ? colors.accent : colors.grid;
      ctx.globalAlpha = lit ? 0.35 + 0.65 * (d.level / 4) : 1;
      ctx.fillRect(i * bw, y0 + 16 - h, Math.max(1, bw - (bw > 3 ? 1 : 0)), h);
    });
    ctx.globalAlpha = 1;
    ctx.fillStyle = colors.text;
    ctx.fillRect(Math.min(W - 2, (dayIndex + 0.5) * bw), y0 - 4, 2, 22);
  }

  function frame(now: number) {
    const dt = last ? Math.min(64, now - last) : 16;
    last = now;
    progress = Math.min(1, progress + dt / DURATION_MS);
    // Novos feixes do dia corrente.
    const d = curDay;
    if (d && !reduced) {
      const rate = Math.min(6, (d.messages + d.tool_calls) / 40);
      let n = Math.floor(rate) + (Math.random() < rate % 1 ? 1 : 0);
      const visible = nodes.map((nd, i) => ({ i, w: nodeValue(nd) })).filter((x) => x.w > 0);
      const sum = visible.reduce((s, x) => s + x.w, 0);
      while (n-- > 0 && sum > 0 && beams.length < 400) {
        let r = Math.random() * sum;
        const pick = visible.find((x) => (r -= x.w) <= 0) ?? visible[visible.length - 1];
        beams.push({ node: pick.i, t: 0, speed: 0.0015 + Math.random() * 0.0015 });
      }
    }
    beams = beams.filter((b) => (b.t += b.speed * dt) < 1);
    draw();
    if (progress >= 1) {
      playing = false;
      beams = [];
      draw();
      raf = 0;
      last = 0;
      return;
    }
    raf = requestAnimationFrame(frame);
  }

  function play() {
    if (progress >= 1) progress = 0;
    playing = true;
    last = 0;
    if (!raf) raf = requestAnimationFrame(frame);
  }

  function pause() {
    playing = false;
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
    last = 0;
  }

  function restart() {
    pause();
    beams = [];
    progress = 0;
    if (reduced) draw();
    else play();
  }

  function onScrub(e: Event) {
    pause();
    beams = [];
    progress = Number((e.currentTarget as HTMLInputElement).value) / dayCount;
    draw();
  }

  onMount(() => {
    const mq = matchMedia("(prefers-reduced-motion: reduce)");
    reduced = mq.matches || document.documentElement.dataset.reduceMotion === "true";
    readColors();
    layout();
    const ro = new ResizeObserver(() => layout());
    if (box) ro.observe(box);
    const onVis = () => {
      if (document.hidden && playing) pause();
    };
    document.addEventListener("visibilitychange", onVis);
    if (reduced) {
      progress = 1;
      draw();
    } else play();
    return () => {
      pause();
      ro.disconnect();
      document.removeEventListener("visibilitychange", onVis);
    };
  });

  // Dados novos (outro período): recomeça.
  let seen: Retro | null = null;
  $effect(() => {
    if (retro !== seen) {
      const first = seen === null;
      seen = retro;
      if (!first) {
        layout();
        restart();
      }
    }
  });

  let dateLabel = $derived(
    curDay
      ? (() => {
          const [y, m, d] = curDay.date.split("-").map(Number);
          return new Date(y, m - 1, d).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
        })()
      : "",
  );
</script>

<div class="stage" bind:this={box}>
  <canvas bind:this={canvas} aria-hidden="true"></canvas>
  <p class="sr">{fill($t("llm.central.sessions.retro.canvas_label", { date: dateLabel, n: running.sessions }), { date: dateLabel, n: running.sessions })}</p>
  <div class="hud" aria-hidden="true">
    <div class="date">{dateLabel}</div>
    <div class="counters">
      <span><strong>{running.sessions.toLocaleString()}</strong> {$t("llm.central.sessions.sessions_unit")}</span>
      <span><strong>{running.messages.toLocaleString()}</strong> {$t("llm.central.sessions.messages_unit")}</span>
      <span><strong>{compact(running.tokens)}</strong> tokens</span>
      <span><strong>{running.tools.toLocaleString()}</strong> {$t("llm.central.sessions.tool_calls_unit")}</span>
    </div>
  </div>
  <ul class="toasts" aria-live="polite">
    {#each toasts as m (m.date + m.kind + m.label)}
      <li>{milestoneText(m)}</li>
    {/each}
  </ul>
  <div class="legend" aria-hidden="true">
    <span><i class="m"></i>{$t("llm.central.sessions.retro.models")}</span>
    <span><i class="tl"></i>{$t("llm.central.sessions.retro.tools")}</span>
  </div>
</div>
<div class="controls">
  {#if playing}
    <button type="button" class="btn btn-secondary btn-sm" onclick={pause}>{$t("llm.central.sessions.retro.pause")}</button>
  {:else}
    <button type="button" class="btn btn-primary btn-sm" onclick={play}>{$t("llm.central.sessions.retro.play")}</button>
  {/if}
  <button type="button" class="btn btn-ghost btn-sm" onclick={restart}>{$t("llm.central.sessions.retro.restart")}</button>
  <input
    type="range"
    min="0"
    max={dayCount - 1}
    step="1"
    value={dayIndex}
    oninput={onScrub}
    aria-label={$t("llm.central.sessions.retro.scrub")}
    aria-valuetext={dateLabel}
  />
</div>

<style>
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .stage {
    position: relative;
    width: 100%;
    border-radius: var(--radius-lg);
    background: var(--pane);
    box-shadow: inset 0 0 0 1px var(--content-border);
    overflow: hidden;
  }
  canvas {
    display: block;
    width: 100%;
  }
  .hud {
    position: absolute;
    top: 12px;
    right: 14px;
    text-align: right;
    pointer-events: none;
  }
  .date {
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 700;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .counters {
    display: flex;
    flex-direction: column;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .counters strong {
    color: var(--text);
  }
  .toasts {
    position: absolute;
    top: 12px;
    left: 14px;
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-width: 45%;
    pointer-events: none;
  }
  .toasts li {
    font-size: var(--text-xs);
    color: var(--text);
    background: var(--popup-bg);
    padding: 5px 9px;
    border-radius: var(--radius-md);
    box-shadow: 0 2px 8px rgba(var(--shadow-ink), var(--elev-alpha-1)), inset 0 0 0 1px var(--content-border);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    animation: in var(--duration-slow) var(--ease-out);
  }
  @keyframes in {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  .legend {
    position: absolute;
    left: 14px;
    bottom: 48px;
    display: flex;
    gap: 12px;
    font-size: var(--text-caption);
    color: var(--text-dim);
    pointer-events: none;
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .legend i {
    width: 9px;
    height: 9px;
    border-radius: 50%;
  }
  .legend .m {
    background: var(--accent);
  }
  .legend .tl {
    background: var(--surface);
    box-shadow: inset 0 0 0 2px var(--text-dim);
  }
  .controls {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 8px;
  }
  .controls input[type="range"] {
    flex: 1;
    accent-color: var(--accent);
  }
  @media (max-width: 640px) {
    .hud {
      position: static;
      text-align: left;
      padding: 8px 12px 0;
    }
    .toasts {
      max-width: 90%;
      top: auto;
      bottom: 64px;
    }
  }
</style>
