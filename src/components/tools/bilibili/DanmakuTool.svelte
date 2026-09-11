<script lang="ts">
  /**
   * Danmaku do Bilibili: `mode="export"` puxa os comentários flutuantes de
   * uma parte e grava em XML/ASS/JSON; `mode="burn"` queima o ASS num clipe.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    baseName,
    errText,
    fmtBytes,
    fmtSecs,
    onToolProgress,
    openPath,
    pct,
    pickDir,
    pickFile,
    reveal,
    FILTERS,
    type ToolProgress,
  } from "$lib/tools/rt";

  let { mode = "export" }: { mode?: "export" | "burn" } = $props();

  type Part = { page: number; title: string; cid: number; duration_secs: number };
  type ExportFile = { format: string; path: string; bytes: number };
  type ExportResult = {
    title: string;
    part: Part;
    parts: Part[];
    fetched: number;
    kept: number;
    files: ExportFile[];
    with_account: boolean;
  };
  type BurnResult = {
    output: string;
    bytes: number;
    duration_secs: number;
    dialogues: number;
    video_kbps: number;
    audio_kbps: number;
  };

  const KINDS = ["scroll", "top", "bottom"] as const;
  const FORMATS = ["xml", "ass", "json"] as const;
  const ASS_FILTER = [{ name: "ASS", extensions: ["ass", "ssa", "srt", "vtt"] }];

  // Export
  let url = $state("");
  let page = $state(0);
  let formats = $state<string[]>(["xml", "ass", "json"]);
  let keyword = $state("");
  let useRegex = $state(false);
  let invert = $state(false);
  let kinds = $state<string[]>([]);
  let maxPerSecond = $state(0);
  let fontName = $state("Microsoft YaHei");
  let fontSize = $state(42);
  let opacity = $state(75);
  let lanes = $state(0);
  let speed = $state(12);
  let exportDir = $state("");
  let exported = $state<ExportResult | null>(null);

  // Burn
  let video = $state("");
  let subtitle = $state("");
  let start = $state(0);
  let clipSecs = $state(0);
  let targetMb = $state(0);
  let crf = $state(20);
  let maxHeight = $state(0);
  let fontsDir = $state("");
  let burnDir = $state("");
  let burned = $state<BurnResult | null>(null);

  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let unlisten: (() => void) | null = null;

  const toolId = $derived(mode === "burn" ? "bili-danmaku-burn" : "bili-danmaku");
  const canRun = $derived(mode === "burn" ? !!video && !!subtitle : url.trim().length > 0);

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === toolId) progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  function toggle(list: string[], value: string): string[] {
    return list.includes(value) ? list.filter((v) => v !== value) : [...list, value];
  }

  async function runExport() {
    exported = null;
    const out = await invoke<ExportResult>("tool_bili_danmaku_export", {
      opts: {
        url: url.trim(),
        page,
        formats,
        filter: {
          keyword: useRegex ? "" : keyword,
          regex: useRegex ? keyword : "",
          invert,
          kinds,
          max_per_second: maxPerSecond,
        },
        style: {
          font_name: fontName,
          font_size: fontSize,
          // O ASS conta transparência, não opacidade.
          alpha: Math.round((1 - opacity / 100) * 255),
          scroll_lanes: lanes,
          scroll_duration_secs: speed,
        },
        output_dir: exportDir,
      },
    });
    exported = out;
    page = out.part.page;
    // Encadeia com a queima: o ASS recém-gravado já entra no outro modo.
    const ass = out.files.find((f) => f.format === "ass");
    if (ass) subtitle = ass.path;
    showToast("success", `${out.kept} / ${out.fetched}`);
  }

  async function runBurn() {
    burned = null;
    const out = await invoke<BurnResult>("tool_bili_danmaku_burn", {
      opts: {
        video,
        subtitle,
        start,
        duration: clipSecs,
        target_mb: targetMb,
        crf,
        preset: "veryfast",
        max_height: maxHeight,
        fonts_dir: fontsDir,
        output_dir: burnDir,
        suffix: "",
      },
    });
    burned = out;
    showToast("success", fmtBytes(out.bytes));
  }

  async function run() {
    if (!canRun || busy) return;
    busy = true;
    progress = null;
    try {
      if (mode === "burn") await runBurn();
      else await runExport();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  {#if mode === "export"}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.danmaku.url")}</div>
            <div class="group-row-sub">{$t("tools.danmaku.url_hint")}</div>
          </div>
          <div class="group-row-trailing">
            <input class="input" type="text" bind:value={url} placeholder="BV1xx411c7mu" />
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.danmaku.part")}</div>
            <div class="group-row-sub">{$t("tools.danmaku.part_hint")}</div>
          </div>
          <div class="group-row-trailing">
            {#if exported && exported.parts.length > 1}
              <select class="input" bind:value={page}>
                {#each exported.parts as p (p.cid)}
                  <option value={p.page}>P{p.page} · {p.title}</option>
                {/each}
              </select>
            {:else}
              <input class="input" type="number" min="0" step="1" bind:value={page} style:width="6em" />
            {/if}
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.danmaku.formats")}</div></div>
          <div class="group-row-trailing btn-row">
            {#each FORMATS as f (f)}
              <button
                class="btn btn-sm {formats.includes(f) ? 'btn-primary' : 'btn-secondary'}"
                type="button"
                onclick={() => (formats = toggle(formats, f))}>{f.toUpperCase()}</button>
            {/each}
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.common.output_folder")}</div>
            <div class="group-row-sub mono">{exportDir || $t("tools.danmaku.downloads_folder")}</div>
          </div>
          <div class="group-row-trailing btn-row">
            {#if exportDir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (exportDir = "")}>×</button>{/if}
            <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) exportDir = d; }}>{$t("tools.common.choose")}</button>
          </div>
        </div>
      </div>
    </section>

    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{useRegex ? $t("tools.danmaku.regex") : $t("tools.danmaku.keyword")}</div>
            <div class="group-row-sub">{invert ? $t("tools.danmaku.invert_on") : $t("tools.danmaku.invert_off")}</div>
          </div>
          <div class="group-row-trailing btn-row">
            <input class="input" type="text" bind:value={keyword} />
            <button class="btn btn-sm {useRegex ? 'btn-primary' : 'btn-secondary'}" type="button" onclick={() => (useRegex = !useRegex)}>.*</button>
            <button class="btn btn-sm {invert ? 'btn-primary' : 'btn-secondary'}" type="button" onclick={() => (invert = !invert)}>≠</button>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.danmaku.kinds")}</div>
            <div class="group-row-sub">{kinds.length ? "" : $t("tools.danmaku.kinds_all")}</div>
          </div>
          <div class="group-row-trailing btn-row">
            {#each KINDS as k (k)}
              <button
                class="btn btn-sm {kinds.includes(k) ? 'btn-primary' : 'btn-secondary'}"
                type="button"
                onclick={() => (kinds = toggle(kinds, k))}>{$t(`tools.danmaku.kind_${k}`)}</button>
            {/each}
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.danmaku.density")}</div>
            <div class="group-row-sub">{maxPerSecond === 0 ? $t("tools.danmaku.density_off") : $t("tools.danmaku.density_hint")}</div>
          </div>
          <div class="group-row-trailing"><input type="range" min="0" max="30" step="1" bind:value={maxPerSecond} /></div>
        </div>
      </div>
    </section>

    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.danmaku.font")}</div><div class="group-row-sub">{$t("tools.danmaku.font_hint")}</div></div>
          <div class="group-row-trailing btn-row">
            <input class="input" type="text" bind:value={fontName} />
            <input class="input" type="number" min="12" max="120" step="2" bind:value={fontSize} style:width="5em" />
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.danmaku.opacity")} {opacity}%</div></div>
          <div class="group-row-trailing"><input type="range" min="10" max="100" step="5" bind:value={opacity} /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.danmaku.lanes")}</div>
            <div class="group-row-sub">{lanes === 0 ? $t("tools.danmaku.lanes_auto") : ""}</div>
          </div>
          <div class="group-row-trailing"><input type="range" min="0" max="24" step="1" bind:value={lanes} /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.danmaku.speed")}</div><div class="group-row-sub">{speed}s</div></div>
          <div class="group-row-trailing"><input type="range" min="4" max="24" step="1" bind:value={speed} /></div>
        </div>
      </div>
    </section>
  {:else}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.danmaku.video")}</div><div class="group-row-sub mono">{video ? baseName(video) : "—"}</div></div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(FILTERS.video); if (f) video = f; }}>{$t("tools.common.choose")}</button></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.danmaku.ass")}</div><div class="group-row-sub mono">{subtitle ? baseName(subtitle) : "—"}</div></div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(ASS_FILTER); if (f) subtitle = f; }}>{$t("tools.common.choose")}</button></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.danmaku.clip")}</div><div class="group-row-sub">{$t("tools.danmaku.clip_hint")}</div></div>
          <div class="group-row-trailing btn-row">
            <input class="input" type="number" min="0" step="1" bind:value={start} style:width="6em" />
            <span class="dim">→</span>
            <input class="input" type="number" min="0" step="1" bind:value={clipSecs} style:width="6em" />
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{targetMb > 0 ? $t("tools.danmaku.target") : $t("tools.danmaku.quality")}</div>
            <div class="group-row-sub">{targetMb > 0 ? `${targetMb} MB` : `CRF ${crf}`}</div>
          </div>
          <div class="group-row-trailing btn-row">
            {#if targetMb > 0}
              <input class="input" type="number" min="1" step="1" bind:value={targetMb} style:width="6em" />
            {:else}
              <input type="range" min="14" max="32" step="1" bind:value={crf} />
            {/if}
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => (targetMb = targetMb > 0 ? 0 : 10)}>{targetMb > 0 ? $t("tools.danmaku.quality") : $t("tools.danmaku.target")}</button>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.danmaku.max_height")}</div><div class="group-row-sub">{maxHeight === 0 ? $t("tools.danmaku.keep_size") : `${maxHeight}p`}</div></div>
          <div class="group-row-trailing"><input type="range" min="0" max="1080" step="120" bind:value={maxHeight} /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.danmaku.fonts_dir")}</div>
            <div class="group-row-sub mono">{fontsDir || $t("tools.danmaku.fonts_system")}</div>
          </div>
          <div class="group-row-trailing btn-row">
            {#if fontsDir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (fontsDir = "")}>×</button>{/if}
            <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) fontsDir = d; }}>{$t("tools.common.choose")}</button>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.common.output_folder")}</div>
            <div class="group-row-sub mono">{burnDir || $t("tools.common.same_folder")}</div>
          </div>
          <div class="group-row-trailing btn-row">
            {#if burnDir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (burnDir = "")}>×</button>{/if}
            <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) burnDir = d; }}>{$t("tools.common.choose")}</button>
          </div>
        </div>
      </div>
    </section>
  {/if}

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-primary" type="button" disabled={busy || !canRun} onclick={run}>
            {busy ? $t("tools.common.working") : mode === "burn" ? $t("tools.danmaku.run_burn") : $t("tools.danmaku.run_export")}
          </button>
        </div>
      </div>
    </div>
  </section>

  {#if mode === "export" && exported}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{exported.title}</div>
            <div class="group-row-sub">
              P{exported.part.page} · {fmtSecs(exported.part.duration_secs)} ·
              <strong>{exported.kept}</strong> / {exported.fetched} {$t("tools.danmaku.comments")}
              {#if exported.with_account}<span class="tag tag-success">{$t("tools.danmaku.with_account")}</span>{/if}
            </div>
          </div>
        </div>
        {#each exported.files as f (f.path)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title"><span class="tag">{f.format.toUpperCase()}</span> {fmtBytes(f.bytes)}</div>
              <div class="group-row-sub mono">{f.path}</div>
            </div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(f.path)}>{$t("tools.common.reveal")}</button></div>
          </div>
        {/each}
      </div>
    </section>
  {/if}

  {#if mode === "burn" && burned}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{baseName(burned.output)}</div>
            <div class="group-row-sub">
              {fmtBytes(burned.bytes)} · {fmtSecs(burned.duration_secs)} ·
              {burned.dialogues} {$t("tools.danmaku.comments")}
              {#if burned.video_kbps}· {burned.video_kbps} kbps{/if}
            </div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => openPath(burned!.output)}>{$t("tools.common.play")}</button>
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(burned!.output)}>{$t("tools.common.reveal")}</button>
          </div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
