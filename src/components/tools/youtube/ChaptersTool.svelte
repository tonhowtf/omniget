<script lang="ts">
  /** Capítulos automáticos por silêncio + mudança de cena, no formato da descrição do YouTube. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtSecs, FILTERS, onToolProgress, pct, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Chapter = { start: number; end: number; title: string; source: string };
  type Result = { input: string; duration_seconds: number; chapters: Chapter[]; description: string; ffmetadata: string; description_path: string; metadata_path: string; silence_marks: number; scene_marks: number; titled_from_transcript: boolean; youtube_ready: boolean; youtube_note: string | null };

  const SOURCE_KEY: Record<string, string> = { inicio: "tools.ytchapters.src_start", silencio: "tools.ytchapters.src_silence", cena: "tools.ytchapters.src_scene", ambos: "tools.ytchapters.src_both" };

  let input = $state("");
  let subtitlePath = $state("");
  let useSilence = $state(true);
  let useScene = $state(true);
  let minChapter = $state(45);
  let sceneThreshold = $state(0.4);
  let writeFiles = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "yt-chapters") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!input || busy) return;
    busy = true;
    progress = null;
    result = null;
    try {
      result = await invoke<Result>("tool_yt_chapters", {
        opts: {
          input,
          subtitle_path: subtitlePath,
          use_silence: useSilence,
          use_scene: useScene,
          scene_threshold: Number(sceneThreshold),
          fuse: { min_chapter: Number(minChapter), max_chapters: 0 },
          write_files: writeFiles,
        },
      });
      showToast("success", `${result.chapters.length} ${$t("tools.ytchapters.chapters")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function copy(text: string) {
    await navigator.clipboard.writeText(text);
    showToast("success", $t("tools.common.copied") as string);
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytchapters.video")}</div><div class="group-row-sub mono">{input ? baseName(input) : "—"}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(FILTERS.video); if (f) input = f; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytchapters.subtitle")}</div><div class="group-row-sub mono">{subtitlePath ? baseName(subtitlePath) : $t("tools.ytchapters.subtitle_sub")}</div></div>
        <div class="group-row-trailing btn-row">
          {#if subtitlePath}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (subtitlePath = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(FILTERS.subtitles); if (f) subtitlePath = f; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytchapters.signals")}</div><div class="group-row-sub">{$t("tools.ytchapters.signals_sub")}</div></div>
        <div class="group-row-trailing btn-row">
          <label class="chk"><input type="checkbox" bind:checked={useSilence} /> {$t("tools.ytchapters.silence")}</label>
          <label class="chk"><input type="checkbox" bind:checked={useScene} /> {$t("tools.ytchapters.scene")}</label>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytchapters.min_chapter")} {minChapter}s</div><div class="group-row-sub">{$t("tools.ytchapters.min_chapter_sub")}</div></div>
        <div class="group-row-trailing"><input type="range" min="15" max="300" step="5" bind:value={minChapter} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytchapters.scene_threshold")} {sceneThreshold}</div><div class="group-row-sub">{$t("tools.ytchapters.scene_threshold_sub")}</div></div>
        <div class="group-row-trailing"><input type="range" min="0.1" max="0.9" step="0.05" bind:value={sceneThreshold} disabled={!useScene} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytchapters.write_files")}</div><div class="group-row-sub">{$t("tools.ytchapters.write_files_sub")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={writeFiles} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub">{$t("tools.ytchapters.scanning")}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !input || (!useSilence && !useScene)} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.ytchapters.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.chapters.length} {$t("tools.ytchapters.chapters")} · {fmtSecs(result.duration_seconds)}</div>
            <div class="group-row-sub">{result.silence_marks} {$t("tools.ytchapters.silence_marks")} · {result.scene_marks} {$t("tools.ytchapters.scene_marks")}</div>
            {#if result.youtube_note}<div class="group-row-sub warn">{result.youtube_note}</div>{:else}<div class="group-row-sub"><span class="tag tag-success">{$t("tools.ytchapters.ready")}</span></div>{/if}
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-primary btn-sm" type="button" onclick={() => copy(result!.description)}>{$t("tools.ytchapters.copy_description")}</button>
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => copy(result!.ffmetadata)}>{$t("tools.ytchapters.copy_ffmeta")}</button>
            {#if result.description_path}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result!.description_path)}>↗</button>{/if}
          </div>
        </div>
      </div>
    </section>
    <section>
      <div class="group">
        {#each result.chapters as c, i (i)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title mono">{result.description.split("\n")[i]}</div>
              <div class="group-row-sub">{fmtSecs(c.end - c.start)} · {$t(SOURCE_KEY[c.source] ?? "tools.ytchapters.src_scene")}</div>
            </div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-sm); overflow-wrap: anywhere; }
  .chk { display: inline-flex; align-items: center; gap: 4px; font-size: var(--text-sm); }
  .warn { color: var(--warning, #d97706); }
</style>
