<script lang="ts">
  /** Arquivo pessoal das newsletters do Substack: cada post em Markdown, HTML e JSON. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openPath, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Publication = { host: string; name: string; dir: string; posts: number; locked: number; error: string | null };
  type Result = {
    used_session: boolean;
    dest: string;
    publications: Publication[];
    posts: number;
    locked: number;
    images: number;
    requests: number;
    files: string[];
  };

  let list = $state("");
  let dest = $state("");
  let limit = $state(50);
  let since = $state("");
  let markdown = $state(true);
  let html = $state(false);
  let json = $state(false);
  let images = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const publications = $derived(
    list
      .split(/[\s,]+/)
      .map((s) => s.trim())
      .filter(Boolean),
  );
  const canRun = $derived(!!dest && (markdown || html || json));

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "blog-substack") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!canRun || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_blog_substack", {
        opts: {
          publications,
          dest,
          limit,
          since: since.trim(),
          markdown,
          html,
          json,
          images,
          delay_ms: 1200,
          max_requests: 400,
        },
      });
      showToast("success", `${result.posts} ${$t("tools.substack.posts")}`);
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
          <div class="group-row-title">{$t("tools.substack.publications")}</div>
          <div class="group-row-sub">{$t("tools.substack.publications_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="text" placeholder={$t("tools.substack.publications_ph")} bind:value={list} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.output_folder")}</div>
          <div class="group-row-sub mono">{dest || "—"}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.substack.formats")}</div></div>
        <div class="group-row-trailing">
          <div class="segmented">
            <button class="segmented-btn" class:active={markdown} type="button" aria-pressed={markdown} onclick={() => (markdown = !markdown)}>Markdown</button>
            <button class="segmented-btn" class:active={html} type="button" aria-pressed={html} onclick={() => (html = !html)}>HTML</button>
            <button class="segmented-btn" class:active={json} type="button" aria-pressed={json} onclick={() => (json = !json)}>JSON</button>
          </div>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.substack.limit")} {limit === 0 ? $t("tools.substack.limit_all") : limit}</div>
          <div class="group-row-sub">{$t("tools.substack.limit_hint")}</div>
        </div>
        <div class="group-row-trailing"><input type="range" min="0" max="300" step="10" bind:value={limit} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.substack.since")}</div>
          <div class="group-row-sub">{$t("tools.substack.since_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="date" bind:value={since} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.substack.images")}</div>
          <div class="group-row-sub">{$t("tools.substack.images_hint")}</div>
        </div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={images} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-sub">{$t("tools.substack.limits_note")}</div>
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !canRun} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.substack.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.posts} {$t("tools.substack.posts")}</div>
            <div class="group-row-sub">
              {result.publications.length} {$t("tools.substack.publications_done")}
              {#if result.images > 0}· {result.images} {$t("tools.substack.images_done")}{/if}
              {#if result.locked > 0}· <span class="tag">{result.locked} {$t("tools.substack.locked")}</span>{/if}
              · {result.requests} {$t("tools.substack.requests")}
              · <span class="tag">{result.used_session ? $t("tools.substack.session_on") : $t("tools.substack.session_off")}</span>
            </div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result?.dest ?? "")}>{$t("tools.common.reveal")}</button></div>
        </div>
        {#each result.publications as p (p.host)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{p.name}</div>
              <div class="group-row-sub mono">{p.host}</div>
              {#if p.error}<div class="group-row-sub">{p.error}</div>{/if}
            </div>
            <div class="group-row-trailing btn-row">
              <span class="tag">{p.posts}</span>
              {#if p.locked > 0}<span class="tag">{p.locked} {$t("tools.substack.locked")}</span>{/if}
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(p.dir)}>↗</button>
            </div>
          </div>
        {/each}
      </div>
    </section>

    {#if result.files.length}
      <section>
        <div class="group">
          {#each result.files.slice(0, 40) as f (f)}
            <div class="group-row">
              <div class="group-row-content"><div class="group-row-sub mono">{baseName(f)}</div></div>
              <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => openPath(f)}>{$t("tools.common.open")}</button></div>
            </div>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
