<script lang="ts">
  /** arXiv -> Markdown com o LaTeX da matematica preservado. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import type { ToolProgress } from "$lib/tools/rt";
  import { baseName, dirName, errText, onToolProgress, openUrl, pickDir, reveal, saveAs } from "$lib/tools/rt";

  type Meta = {
    id: string; version: number | null; title: string; authors: string[]; summary: string;
    categories: string[]; primary_category: string; published: string; updated: string;
    doi: string | null; journal_ref: string | null; comment: string | null;
    abs_url: string; pdf_url: string;
  };
  type Doc = {
    meta: Meta; markdown: string; body_source: string; path: string | null;
    chars: number; math_blocks: number; source_files: string[]; fallback_reason: string | null;
  };

  let input = $state("");
  let prefer = $state("auto");
  let outputDir = $state("");
  let doc = $state<Doc | null>(null);
  let busy = $state(false);
  let stage = $state("");
  let unlisten: (() => void) | null = null;

  async function run(save: boolean) {
    if (!input.trim()) return;
    busy = true;
    stage = "";
    try {
      doc = await invoke<Doc>("tool_arxiv_md", {
        opts: { input, prefer, output_dir: outputDir, save: save && !!outputDir },
      });
      if (doc.fallback_reason) showToast("info", doc.fallback_reason);
      if (doc.path) showToast("success", doc.path);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
      stage = "";
    }
  }

  async function chooseDir() {
    const d = await pickDir();
    if (d) outputDir = d;
  }

  async function saveFile() {
    if (!doc) return;
    const target = await saveAs(`${doc.meta.id.replace("/", "_")}.md`, [{ name: "Markdown", extensions: ["md"] }]);
    if (!target) return;
    try {
      await invoke("tools_save_text", { outputDir: dirName(target), fileName: baseName(target), content: doc.markdown });
      showToast("success", target);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function copy() {
    if (!doc) return;
    await navigator.clipboard.writeText(doc.markdown);
    showToast("success", $t("tools.common.copied"));
  }

  onMount(async () => {
    unlisten = await onToolProgress((p: ToolProgress) => {
      if (p.id !== "ai-arxiv-md") return;
      stage = p.message ?? p.stage;
    });
  });

  onDestroy(() => unlisten?.());
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <input class="input" bind:value={input} placeholder={$t("tools.arxiv.input")} onkeydown={(e) => { if (e.key === "Enter") run(false); }} />
        </div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={prefer}>
            <option value="auto">{$t("tools.arxiv.prefer_auto")}</option>
            <option value="source">{$t("tools.arxiv.prefer_source")}</option>
            <option value="html">{$t("tools.arxiv.prefer_html")}</option>
            <option value="abstract">{$t("tools.arxiv.prefer_abstract")}</option>
          </select>
          <button class="btn btn-primary" type="button" disabled={busy || !input.trim()} onclick={() => run(false)}>{$t("tools.arxiv.run")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-sub">{$t("tools.common.output_folder")}</div></div>
        <div class="group-row-trailing btn-row">
          <span class="dim mono">{outputDir || $t("tools.arxiv.no_folder")}</span>
          <button class="btn btn-secondary btn-sm" type="button" onclick={chooseDir}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row"><div class="group-row-sub">{busy ? stage || $t("tools.common.working") : $t("tools.arxiv.intro")}</div></div>
    </div>
  </section>

  {#if doc}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{doc.meta.title}</div>
            <div class="group-row-sub">{doc.meta.authors.slice(0, 6).join(", ")}{doc.meta.authors.length > 6 ? " …" : ""}</div>
          </div>
          <div class="group-row-trailing btn-row">
            <span class="tag" class:tag-success={doc.body_source === "latex"}>{doc.body_source}</span>
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(doc?.meta.abs_url ?? "")}>arXiv</button>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-sub">
            {doc.meta.categories.join(" · ")}{#if doc.meta.published} · {doc.meta.published.slice(0, 10)}{/if} · {doc.chars} {$t("tools.arxiv.chars")} · {doc.math_blocks} {$t("tools.arxiv.math_blocks")}
          </div>
        </div>
        {#if doc.meta.doi}
          <div class="group-row"><div class="group-row-sub mono">DOI {doc.meta.doi}</div></div>
        {/if}
        <div class="group-row">
          <div class="group-row-content"></div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary btn-sm" type="button" onclick={copy}>{$t("tools.common.copy")}</button>
            <button class="btn btn-secondary btn-sm" type="button" onclick={saveFile}>{$t("tools.common.save")}</button>
            {#if doc.path}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(doc?.path ?? "")}>{$t("tools.common.reveal")}</button>{/if}
          </div>
        </div>
      </div>
    </section>

    <section>
      <pre class="preview">{doc.markdown}</pre>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-4); }
  .dim { color: var(--text-dim); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .preview { max-height: 460px; overflow: auto; padding: var(--space-3); border-radius: var(--radius-lg); background: var(--surface); font-family: var(--font-mono); font-size: var(--text-xs); white-space: pre-wrap; overflow-wrap: anywhere; margin: 0; }
</style>
