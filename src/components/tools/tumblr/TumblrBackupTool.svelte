<script lang="ts">
  /** Espelho offline de um blog do Tumblr: HTML navegável com a cadeia de reblog. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openPath, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Result = {
    blog: string; posts: number; pages: number; tags: number; media_files: number;
    used_session: boolean; dir: string; index_path: string;
  };

  let blog = $state("");
  let outDir = $state("");
  let downloadMedia = $state(true);
  let perPage = $state(50);
  let limit = $state(0);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const ready = $derived(blog.trim().length > 0 && outDir.length > 0);

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "tumblr-blog-backup") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!ready || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_tumblr_backup", {
        opts: {
          blog: blog.trim(),
          out_dir: outDir,
          download_media: downloadMedia,
          limit: Number(limit) || null,
          per_page: Number(perPage) || 50,
          account_slug: null,
        },
      });
      showToast("success", `${result.posts} ${$t("tools.tblbackup.posts")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.tblbackup.blog")}</div>
          <div class="group-row-sub">{$t("tools.tblbackup.blog_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="text" placeholder="meublog" bind:value={blog} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.output_folder")}</div>
          <div class="group-row-sub mono">{outDir || "—"}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.tblbackup.download_media")}</div>
          <div class="group-row-sub">{$t("tools.tblbackup.download_media_hint")}</div>
        </div>
        <div class="group-row-trailing"><button class="toggle" class:on={downloadMedia} type="button" role="switch" aria-checked={downloadMedia} aria-label={$t("tools.tblbackup.download_media")} onclick={() => (downloadMedia = !downloadMedia)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.tblbackup.per_page")}</div>
          <div class="group-row-sub">{$t("tools.tblbackup.per_page_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="number" min="5" step="5" bind:value={perPage} style:width="6em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.tblbackup.limit")}</div>
          <div class="group-row-sub">{$t("tools.tblbackup.limit_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="number" min="0" step="100" bind:value={limit} style:width="6em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {:else}
            <div class="group-row-sub">{$t("tools.tumblr.session_hint")}</div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !ready} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.tblbackup.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.blog}</div>
            <div class="group-row-sub">
              <span class="tag" class:tag-success={result.used_session}>{result.used_session ? $t("tools.tumblr.session_on") : $t("tools.tumblr.session_off")}</span>
              {result.posts} {$t("tools.tblbackup.posts")} · {result.pages} {$t("tools.tblbackup.pages")} · {result.tags} {$t("tools.tblbackup.tags")} · {result.media_files} {$t("tools.tblbackup.media")}
            </div>
            <div class="group-row-sub mono">{result.dir}</div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result?.dir ?? "")}>{$t("tools.common.reveal")}</button>
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => openPath(result?.index_path ?? "")}>{$t("tools.tblbackup.open_mirror")}</button>
          </div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
