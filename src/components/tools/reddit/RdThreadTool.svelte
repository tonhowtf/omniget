<script lang="ts">
  /** Backup de uma thread do Reddit: post + comentários em Markdown, HTML e JSON. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openPath, openUrl, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Result = {
    used_session: boolean;
    title: string; subreddit: string; author: string; permalink: string;
    comments: number; missing: number; requests: number; files: string[]; dest: string;
  };

  let url = $state("");
  let dest = $state("");
  let sort = $state("top");
  let order = $state("score");
  let markdown = $state(true);
  let html = $state(true);
  let json = $state(false);
  let maxRequests = $state(40);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const canRun = $derived(!!url.trim() && !!dest && (markdown || html || json));

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "rd-thread") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!canRun || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_rd_thread", {
        opts: {
          url: url.trim(),
          dest,
          sort,
          order,
          markdown,
          html,
          json,
          max_requests: maxRequests,
          delay_ms: 1100,
        },
      });
      showToast("success", `${result.comments} ${$t("tools.rdthread.comments")}`);
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
          <div class="group-row-title">{$t("tools.rdthread.url")}</div>
          <div class="group-row-sub">{$t("tools.rdthread.url_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="text" placeholder={$t("tools.rdthread.url_ph")} bind:value={url} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.output_folder")}</div>
          <div class="group-row-sub mono">{dest || "—"}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.rdthread.formats")}</div></div>
        <div class="group-row-trailing">
          <div class="segmented">
            <button class="segmented-btn" class:active={markdown} type="button" aria-pressed={markdown} onclick={() => (markdown = !markdown)}>Markdown</button>
            <button class="segmented-btn" class:active={html} type="button" aria-pressed={html} onclick={() => (html = !html)}>HTML</button>
            <button class="segmented-btn" class:active={json} type="button" aria-pressed={json} onclick={() => (json = !json)}>JSON</button>
          </div>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.rdthread.sort")}</div><div class="group-row-sub">{$t("tools.rdthread.sort_hint")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={sort}>
            <option value="top">{$t("tools.rdthread.sort_top")}</option>
            <option value="confidence">{$t("tools.rdthread.sort_best")}</option>
            <option value="new">{$t("tools.rdthread.sort_new")}</option>
            <option value="old">{$t("tools.rdthread.sort_old")}</option>
            <option value="controversial">{$t("tools.rdthread.sort_controversial")}</option>
            <option value="qa">{$t("tools.rdthread.sort_qa")}</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.rdthread.order")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={order}>
            <option value="score">{$t("tools.rdthread.order_score")}</option>
            <option value="new">{$t("tools.rdthread.order_new")}</option>
            <option value="old">{$t("tools.rdthread.order_old")}</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.rdthread.depth")} {maxRequests}</div><div class="group-row-sub">{$t("tools.rdthread.depth_hint")}</div></div>
        <div class="group-row-trailing"><input type="range" min="0" max="200" step="10" bind:value={maxRequests} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !canRun} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.rdthread.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.title}</div>
            <div class="group-row-sub">
              r/{result.subreddit} · u/{result.author} · <strong>{result.comments}</strong> {$t("tools.rdthread.comments")}
              {#if result.missing > 0}· <span class="tag">{result.missing} {$t("tools.rdthread.missing")}</span>{/if}
              · {result.requests} {$t("tools.rdthread.requests")}
              · <span class="tag">{result.used_session ? $t("tools.rdthread.session_on") : $t("tools.rdthread.session_off")}</span>
            </div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(result?.permalink ?? "")}>{$t("tools.rdthread.open_post")}</button></div>
        </div>
        {#each result.files as f (f)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub mono">{baseName(f)}</div></div>
            <div class="group-row-trailing btn-row">
              <button class="btn btn-secondary btn-sm" type="button" onclick={() => openPath(f)}>{$t("tools.common.open")}</button>
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(f)}>{$t("tools.common.reveal")}</button>
            </div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
