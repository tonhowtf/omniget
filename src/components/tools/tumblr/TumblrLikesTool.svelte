<script lang="ts">
  /** Índice pesquisável (CSV/JSON) dos Likes do Tumblr, separado da pasta de mídia. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openUrl, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Row = {
    post_id: string; blog: string; date: string; kind: string; tags: string[];
    post_url: string; summary: string; note_count: number;
    media_urls: string[]; local_files: string[];
  };
  type Result = {
    posts: number; media: number; matched_local: number; used_session: boolean;
    csv_path: string | null; json_path: string | null; out_dir: string; sample: Row[];
  };

  let blog = $state("");
  let outDir = $state("");
  let mediaDir = $state("");
  let limit = $state(500);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const ready = $derived(blog.trim().length > 0 && outDir.length > 0);
  const files = $derived([result?.csv_path, result?.json_path].filter((f): f is string => !!f));

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "tumblr-likes-index") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!ready || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_tumblr_likes", {
        opts: {
          blog: blog.trim(),
          out_dir: outDir,
          media_dir: mediaDir || null,
          limit: Number(limit) || null,
          write_csv: true,
          write_json: true,
          account_slug: null,
        },
      });
      showToast("success", `${result.posts} ${$t("tools.tbllikes.posts")}`);
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
          <div class="group-row-title">{$t("tools.tbllikes.blog")}</div>
          <div class="group-row-sub">{$t("tools.tbllikes.blog_hint")}</div>
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
          <div class="group-row-title">{$t("tools.tbllikes.media_dir")}</div>
          <div class="group-row-sub mono">{mediaDir || $t("tools.common.optional")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if mediaDir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (mediaDir = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) mediaDir = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.tbllikes.limit")}</div>
          <div class="group-row-sub">{$t("tools.tbllikes.limit_hint")}</div>
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
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !ready} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.tbllikes.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.posts} {$t("tools.tbllikes.posts")}</div>
            <div class="group-row-sub">
              <span class="tag" class:tag-success={result.used_session}>{result.used_session ? $t("tools.tumblr.session_on") : $t("tools.tumblr.session_off")}</span>
              {result.media} {$t("tools.tbllikes.media")} · {result.matched_local} {$t("tools.tbllikes.matched")}
            </div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result?.out_dir ?? "")}>{$t("tools.common.reveal")}</button></div>
        </div>
        {#each files as f (f)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub mono">{baseName(f)}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(f)}>{$t("tools.common.reveal")}</button></div>
          </div>
        {/each}
      </div>
    </section>

    <section>
      <div class="group">
        {#each result.sample as r (r.post_id)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{r.blog || "—"} <span class="dim">{r.date}</span></div>
              <div class="group-row-sub">{r.summary || "—"}</div>
              <div class="group-row-sub">
                <span class="tag">{r.kind}</span>
                {r.tags.length ? r.tags.map((x) => `#${x}`).join(" ") : ""}
                {#if r.local_files.length}<span class="tag tag-success">{$t("tools.tbllikes.on_disk")}</span>{/if}
              </div>
            </div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(r.post_url)}>↗</button></div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
