<script lang="ts">
  /** Exporta as histórias que o próprio usuário publicou no Medium, com os rascunhos. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openPath, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Result = {
    used_session: boolean;
    source: string;
    user: string;
    dest: string;
    posts: number;
    drafts: number;
    images: number;
    requests: number;
    files: string[];
    note: string | null;
  };

  let user = $state("");
  let dest = $state("");
  let drafts = $state(true);
  let limit = $state(0);
  let markdown = $state(true);
  let html = $state(false);
  let json = $state(false);
  let images = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const canRun = $derived(!!dest && (markdown || html || json));

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "blog-medium") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!canRun || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_blog_medium", {
        opts: {
          user: user.trim(),
          dest,
          drafts,
          limit,
          markdown,
          html,
          json,
          images,
          delay_ms: 1200,
          max_requests: 200,
        },
      });
      showToast("success", `${result.posts + result.drafts} ${$t("tools.medium.stories")}`);
      if (result.note) showToast("info", result.note);
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
          <div class="group-row-title">{$t("tools.medium.user")}</div>
          <div class="group-row-sub">{$t("tools.medium.user_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="text" placeholder={$t("tools.medium.user_ph")} bind:value={user} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.output_folder")}</div>
          <div class="group-row-sub mono">{dest || "—"}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.medium.formats")}</div></div>
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
          <div class="group-row-title">{$t("tools.medium.drafts")}</div>
          <div class="group-row-sub">{$t("tools.medium.drafts_hint")}</div>
        </div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={drafts} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.medium.limit")} {limit === 0 ? $t("tools.medium.limit_all") : limit}</div>
        </div>
        <div class="group-row-trailing"><input type="range" min="0" max="200" step="10" bind:value={limit} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.medium.images")}</div>
          <div class="group-row-sub">{$t("tools.medium.images_hint")}</div>
        </div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={images} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-sub">{$t("tools.medium.limits_note")}</div>
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !canRun} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.medium.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.posts} {$t("tools.medium.published")}{#if result.drafts > 0} · {result.drafts} {$t("tools.medium.drafts_done")}{/if}</div>
            <div class="group-row-sub">
              <span class="tag">{result.source === "session" ? $t("tools.medium.source_session") : $t("tools.medium.source_feed")}</span>
              {#if result.images > 0}· {result.images} {$t("tools.medium.images_done")}{/if}
              · {result.requests} {$t("tools.medium.requests")}
              · <span class="tag">{result.used_session ? $t("tools.medium.session_on") : $t("tools.medium.session_off")}</span>
            </div>
            {#if result.note}<div class="group-row-sub">{result.note}</div>{/if}
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result?.dest ?? "")}>{$t("tools.common.reveal")}</button></div>
        </div>
        {#each result.files.slice(0, 40) as f (f)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub mono">{baseName(f)}</div></div>
            <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => openPath(f)}>{$t("tools.common.open")}</button></div>
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
