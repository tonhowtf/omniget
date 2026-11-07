<script lang="ts">
  import { t } from "$lib/i18n";
  import { cssColor, toHex, type Oklch } from "$lib/palette/color";
  import { contrastReport, suggestFix, wcagRatio, levelOf, ratioText, type FixTarget } from "$lib/palette/contrast";
  import { scaleOf } from "$lib/palette/project";
  import { workspace } from "$lib/palette/workspace.svelte";
  import ColorField from "./ColorField.svelte";
  import StatusIcon from "$components/converter/StatusIcon.svelte";

  let project = $derived(workspace.project!);
  let text = $derived(project.contrast.text);
  let bg = $derived(project.contrast.background);
  let report = $derived(contrastReport(text, bg));
  let fixTarget = $state<FixTarget>("text");
  let fixMin = $state(4.5);
  let fix = $derived(suggestFix(text, bg, fixTarget, fixMin));

  let options = $derived(
    project.palettes.flatMap((pal) => scaleOf(pal, project.gamut).map((s) => ({ key: `${pal.id}:${s.step}`, label: `${pal.name} ${s.step}`, color: s.color }))),
  );

  let rowsId = $state("");
  let colsId = $state("");
  $effect(() => {
    if (!project.palettes.some((p) => p.id === rowsId)) rowsId = project.palettes[0]?.id ?? "";
    if (!project.palettes.some((p) => p.id === colsId)) colsId = (project.palettes[1] ?? project.palettes[0])?.id ?? "";
  });
  let rowPal = $derived(project.palettes.find((p) => p.id === rowsId));
  let colPal = $derived(project.palettes.find((p) => p.id === colsId));
  let rowScale = $derived(rowPal ? scaleOf(rowPal, project.gamut) : []);
  let colScale = $derived(colPal ? scaleOf(colPal, project.gamut) : []);

  function setText(c: Oklch) {
    project.contrast.text = c;
    workspace.touch();
  }

  function setBg(c: Oklch) {
    project.contrast.background = { ...c, alpha: 1 };
    workspace.touch();
  }

  function pickOption(key: string, which: "text" | "bg") {
    const opt = options.find((o) => o.key === key);
    if (!opt) return;
    if (which === "text") setText(opt.color);
    else setBg(opt.color);
  }

  function swap() {
    const a = project.contrast.text;
    project.contrast.text = { ...project.contrast.background };
    project.contrast.background = { ...a, alpha: 1 };
    workspace.touch();
  }

  function applyFix() {
    if (!fix) return;
    if (fixTarget === "text") setText(fix.color);
    else setBg(fix.color);
    workspace.showToast($t("palette.contrast.fixed", { ratio: fix.ratio.toFixed(2) }) as string, "success");
  }

  function useCell(fg: Oklch, back: Oklch) {
    setText(fg);
    setBg(back);
  }

  function mark(level: string): string {
    if (level === "aaa") return "AAA";
    if (level === "aa") return "AA";
    return $t(level === "aa_large" ? "palette.contrast.mark_large" : "palette.contrast.mark_fail") as string;
  }

  function apcaHint(lc: number): string {
    const v = Math.abs(lc);
    if (v >= 90) return "palette.apca.preferred";
    if (v >= 75) return "palette.apca.body";
    if (v >= 60) return "palette.apca.content";
    if (v >= 45) return "palette.apca.large";
    if (v >= 30) return "palette.apca.spot";
    return "palette.apca.low";
  }

  let checks = $derived([
    { key: "normal_aa", pass: report.normalAA, need: "4.5" },
    { key: "normal_aaa", pass: report.normalAAA, need: "7" },
    { key: "large_aa", pass: report.largeAA, need: "3" },
    { key: "large_aaa", pass: report.largeAAA, need: "4.5" },
  ]);
</script>

<div class="pl-contrast">
  <div class="pl-pair">
    <div class="pl-pair-inputs">
      <div class="field">
        <span class="field-label">{$t("palette.contrast.text")}</span>
        <ColorField color={text} label={$t("palette.contrast.text")} onchange={setText} />
        {#if options.length}
          <select class="input" aria-label={$t("palette.contrast.from_palette_text")} value="" onchange={(e) => { pickOption(e.currentTarget.value, "text"); e.currentTarget.value = ""; }}>
            <option value="" disabled>{$t("palette.contrast.from_palette")}</option>
            {#each options as o (o.key)}<option value={o.key}>{o.label} · {toHex(o.color)}</option>{/each}
          </select>
        {/if}
      </div>
      <button type="button" class="btn btn-ghost btn-sm pl-swap" onclick={swap} aria-label={$t("palette.contrast.swap")} title={$t("palette.contrast.swap")}>
        <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 5h9l-2.5-2.5M12 11H3l2.5 2.5" /></svg>
      </button>
      <div class="field">
        <span class="field-label">{$t("palette.contrast.background")}</span>
        <ColorField color={bg} label={$t("palette.contrast.background")} onchange={setBg} />
        {#if options.length}
          <select class="input" aria-label={$t("palette.contrast.from_palette_bg")} value="" onchange={(e) => { pickOption(e.currentTarget.value, "bg"); e.currentTarget.value = ""; }}>
            <option value="" disabled>{$t("palette.contrast.from_palette")}</option>
            {#each options as o (o.key)}<option value={o.key}>{o.label} · {toHex(o.color)}</option>{/each}
          </select>
        {/if}
      </div>
    </div>

    <div class="pl-sample" style:--fg={cssColor(text)} style:--bg={cssColor(bg)}>
      <p class="pl-sample-large">{$t("palette.contrast.sample_large")}</p>
      <p class="pl-sample-body">{$t("palette.contrast.sample_body")}</p>
    </div>

    <div class="pl-scores" aria-live="polite">
      <div class="pl-score">
        <span class="pl-score-label">{$t("palette.contrast.wcag")}</span>
        <span class="pl-score-num">{report.ratio.toFixed(2)}<small>:1</small></span>
      </div>
      <ul class="pl-checks">
        {#each checks as c (c.key)}
          <li class:pass={c.pass}>
            <StatusIcon name={c.pass ? "done" : "failed"} size={14} />
            <span>{$t(`palette.contrast.${c.key}`)}</span>
            <span class="pl-check-state">{c.pass ? $t("palette.contrast.pass") : $t("palette.contrast.fail_need", { ratio: c.need })}</span>
          </li>
        {/each}
      </ul>
      <div class="pl-score">
        <span class="pl-score-label">{$t("palette.contrast.apca")}</span>
        <span class="pl-score-num">Lc {report.apca.toFixed(1)}</span>
        <span class="pl-score-hint">{$t(apcaHint(report.apca))}</span>
      </div>
    </div>
  </div>

  <section class="pl-fixbox" aria-labelledby="pl-fix-title">
    <h3 id="pl-fix-title" class="pl-subtitle">{$t("palette.contrast.fix_title")}</h3>
    <div class="pl-fix-row">
      <div class="segmented" role="radiogroup" aria-label={$t("palette.contrast.fix_move")}>
        <button type="button" class="segmented-btn" role="radio" aria-checked={fixTarget === "text"} onclick={() => (fixTarget = "text")}>{$t("palette.contrast.move_text")}</button>
        <button type="button" class="segmented-btn" role="radio" aria-checked={fixTarget === "background"} onclick={() => (fixTarget = "background")}>{$t("palette.contrast.move_bg")}</button>
      </div>
      <div class="segmented" role="radiogroup" aria-label={$t("palette.contrast.fix_goal")}>
        <button type="button" class="segmented-btn" role="radio" aria-checked={fixMin === 4.5} onclick={() => (fixMin = 4.5)}>{$t("palette.contrast.goal_aa")}</button>
        <button type="button" class="segmented-btn" role="radio" aria-checked={fixMin === 3} onclick={() => (fixMin = 3)}>{$t("palette.contrast.goal_large")}</button>
        <button type="button" class="segmented-btn" role="radio" aria-checked={fixMin === 7} onclick={() => (fixMin = 7)}>{$t("palette.contrast.goal_aaa")}</button>
      </div>
    </div>
    {#if !fix}
      <p class="pl-hint">{$t("palette.contrast.fix_impossible")}</p>
    {:else if fix.deltaL === 0}
      <p class="pl-hint pl-ok"><StatusIcon name="done" size={14} /> {$t("palette.contrast.fix_already")}</p>
    {:else}
      <div class="pl-fix-suggest">
        <span class="pl-fix-chip" style:--c={cssColor(fix.color)} aria-hidden="true"></span>
        <p>{$t("palette.contrast.fix_suggest", { hex: toHex(fix.color), ratio: fix.ratio.toFixed(2), delta: (fix.deltaL * 100).toFixed(1) })}</p>
        <button type="button" class="btn btn-primary btn-sm" onclick={applyFix}>{fixTarget === "text" ? $t("palette.contrast.apply_text") : $t("palette.contrast.apply_bg")}</button>
      </div>
    {/if}
  </section>

  {#if project.palettes.length}
    <section class="pl-matrix-box" aria-labelledby="pl-matrix-title">
      <div class="pl-matrix-head">
        <h3 id="pl-matrix-title" class="pl-subtitle">{$t("palette.contrast.matrix")}</h3>
        <label class="pl-inline">
          <span>{$t("palette.contrast.matrix_rows")}</span>
          <select class="input" bind:value={rowsId}>{#each project.palettes as p (p.id)}<option value={p.id}>{p.name}</option>{/each}</select>
        </label>
        <label class="pl-inline">
          <span>{$t("palette.contrast.matrix_cols")}</span>
          <select class="input" bind:value={colsId}>{#each project.palettes as p (p.id)}<option value={p.id}>{p.name}</option>{/each}</select>
        </label>
      </div>
      <p class="pl-hint">{$t("palette.contrast.matrix_hint")}</p>
      <div class="pl-matrix-scroll">
        <table class="pl-matrix">
          <thead>
            <tr>
              <th scope="col"><span class="pl-sr">{$t("palette.contrast.matrix_corner")}</span></th>
              {#each colScale as c (c.step)}<th scope="col"><span class="pl-th-chip" style:--c={cssColor(c.color)}></span>{c.step}</th>{/each}
            </tr>
          </thead>
          <tbody>
            {#each rowScale as r (r.step)}
              <tr>
                <th scope="row"><span class="pl-th-chip" style:--c={cssColor(r.color)}></span>{r.step}</th>
                {#each colScale as c (c.step)}
                  {@const ratio = Math.floor(wcagRatio(r.color, c.color) * 100) / 100}
                  {@const level = levelOf(ratio)}
                  <td>
                    <button
                      type="button"
                      class="pl-cell pl-cell-{level}"
                      style:--fg={cssColor(r.color)}
                      style:--bg={cssColor(c.color)}
                      title={$t("palette.contrast.cell_title", { text: `${rowPal?.name} ${r.step}`, bg: `${colPal?.name} ${c.step}`, ratio: ratio.toFixed(2) })}
                      onclick={() => useCell(r.color, c.color)}
                    >
                      <span class="pl-cell-ratio">{ratioText(ratio, 1)}</span>
                      <span class="pl-cell-mark">{mark(level)}</span>
                    </button>
                  </td>
                {/each}
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <p class="pl-legend">{$t("palette.contrast.legend")}</p>
    </section>
  {/if}
</div>

<style>
  .pl-contrast {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .pl-pair {
    display: grid;
    grid-template-columns: minmax(220px, 260px) minmax(0, 1fr) minmax(220px, 260px);
    gap: var(--space-4);
    align-items: stretch;
  }

  @media (max-width: 900px) {
    .pl-pair {
      grid-template-columns: 1fr;
    }
  }

  .pl-pair-inputs {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .pl-pair-inputs .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .pl-swap {
    align-self: center;
  }

  .pl-sample {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-5);
    border-radius: var(--radius-lg);
    background: var(--bg);
    color: var(--fg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-sample-large {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 700;
    line-height: var(--leading-xl);
  }

  .pl-sample-body {
    margin: 0;
    font-size: var(--text-base);
    line-height: var(--leading-base);
  }

  .pl-scores {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-score {
    display: flex;
    flex-direction: column;
  }

  .pl-score-label,
  .pl-subtitle {
    margin: 0;
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
    color: var(--text-dim);
  }

  .pl-score-num {
    font-size: var(--text-2xl);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }

  .pl-score-num small {
    font-size: var(--text-md);
    color: var(--text-dim);
  }

  .pl-score-hint {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .pl-checks {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .pl-checks li {
    display: grid;
    grid-template-columns: 14px minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-2);
  }

  .pl-check-state {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .pl-fixbox,
  .pl-matrix-box {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-fix-row,
  .pl-matrix-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .pl-hint,
  .pl-legend {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .pl-legend {
    font-size: var(--text-xs);
  }

  .pl-ok {
    color: var(--text);
  }

  .pl-fix-suggest {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .pl-fix-suggest p {
    margin: 0;
    flex: 1;
    min-width: 200px;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .pl-fix-chip {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    background: var(--c);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-inline {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .pl-matrix-scroll {
    overflow-x: auto;
  }

  .pl-matrix {
    border-collapse: separate;
    border-spacing: 2px;
    font-size: var(--text-xs);
  }

  .pl-matrix th {
    padding: 0 var(--space-1);
    font-weight: 600;
    color: var(--text-dim);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  .pl-th-chip {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: 4px;
    border-radius: var(--radius-full);
    background: var(--c);
    vertical-align: middle;
  }

  .pl-cell {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    width: 52px;
    height: 40px;
    border: 0;
    border-radius: var(--radius-xs);
    background: var(--bg);
    color: var(--fg);
    cursor: pointer;
    font-variant-numeric: tabular-nums;
  }

  .pl-cell:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 1px;
  }

  .pl-cell-ratio {
    font-weight: 650;
  }

  .pl-cell-mark {
    font-size: var(--text-caption);
    font-weight: 700;
    letter-spacing: var(--track-caps);
  }

  .pl-cell-fail {
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
