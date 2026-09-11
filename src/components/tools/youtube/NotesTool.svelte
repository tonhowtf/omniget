<script lang="ts">
  /** Transcrição virando artigo em Markdown: parágrafos costurados, capítulos e timestamps. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, FILTERS, onToolProgress, pct, pickFile, reveal, saveAs, type ToolProgress } from "$lib/tools/rt";

  type Result = { markdown: string; path: string; source: string; title: string; channel: string; url: string; duration_seconds: number; language: string; words: number; paragraphs: number; chapters: number; used_session: boolean };

  const SOURCE_KEY: Record<string, string> = { "legenda-local": "tools.ytnotes.src_file", "legenda-youtube": "tools.ytnotes.src_yt", whisper: "tools.ytnotes.src_whisper" };

  let url = $state("");
  let subtitlePath = $state("");
  let timestamps = $state(true);
  let useChapters = $state(true);
  let allowWhisper = $state(false);
  let gap = $state(2.5);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const preview = $derived(result ? result.markdown.slice(0, 4000) : "");

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "yt-notes") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run(outputPath = "") {
    if (busy || (!url.trim() && !subtitlePath)) return;
    busy = true;
    progress = null;
    try {
      result = await invoke<Result>("tool_yt_notes", {
        opts: {
          url,
          subtitle_path: subtitlePath,
          media_path: "",
          allow_whisper: allowWhisper,
          timestamps,
          use_chapters: useChapters,
          output_path: outputPath,
          stitch: { gap_seconds: Number(gap) || 2.5 },
          account_slug: null,
        },
      });
      showToast("success", `${result.words} ${$t("tools.ytnotes.words")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function save() {
    const name = (result?.title || "notas").replace(/[\\/:*?"<>|]/g, "-");
    const dest = await saveAs(`${name}.md`, [{ name: "Markdown", extensions: ["md"] }]);
    if (!dest) return;
    await run(dest);
    if (result?.path) showToast("success", baseName(result.path));
  }

  async function copy() {
    if (!result) return;
    await navigator.clipboard.writeText(result.markdown);
    showToast("success", $t("tools.common.copied") as string);
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder={$t("tools.common.yt_url")} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytnotes.subtitle")}</div><div class="group-row-sub mono">{subtitlePath ? baseName(subtitlePath) : $t("tools.ytnotes.subtitle_sub")}</div></div>
        <div class="group-row-trailing btn-row">
          {#if subtitlePath}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (subtitlePath = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(FILTERS.subtitles); if (f) subtitlePath = f; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytnotes.timestamps")}</div><div class="group-row-sub">{$t("tools.ytnotes.timestamps_sub")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={timestamps} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytnotes.chapters")}</div><div class="group-row-sub">{$t("tools.ytnotes.chapters_sub")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={useChapters} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytnotes.whisper")}</div><div class="group-row-sub">{$t("tools.ytnotes.whisper_sub")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={allowWhisper} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytnotes.gap")} {gap}s</div><div class="group-row-sub">{$t("tools.ytnotes.gap_sub")}</div></div>
        <div class="group-row-trailing"><input type="range" min="1" max="6" step="0.5" bind:value={gap} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || (!url.trim() && !subtitlePath)} onclick={() => run()}>{busy ? $t("tools.common.working") : $t("tools.ytnotes.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <span class="group-label">{result.title || $t("tools.ytnotes.result")}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.words} {$t("tools.ytnotes.words")} · {result.paragraphs} {$t("tools.ytnotes.paragraphs")}{#if result.chapters} · {result.chapters} {$t("tools.ytnotes.chapters_n")}{/if}</div>
            <div class="group-row-sub">{$t(SOURCE_KEY[result.source] ?? "tools.ytnotes.src_file")}{#if result.channel} · {result.channel}{/if}</div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary btn-sm" type="button" onclick={copy}>{$t("tools.common.copy")}</button>
            <button class="btn btn-primary btn-sm" type="button" disabled={busy} onclick={save}>{$t("tools.common.save")}</button>
            {#if result.path}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result!.path)}>↗</button>{/if}
          </div>
        </div>
      </div>
    </section>
    <section>
      <div class="group">
        <div class="group-row"><pre class="md">{preview}</pre></div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .md { font-family: var(--font-mono); font-size: var(--text-xs); white-space: pre-wrap; overflow-wrap: anywhere; max-height: 460px; overflow-y: auto; margin: 0; width: 100%; }
</style>
