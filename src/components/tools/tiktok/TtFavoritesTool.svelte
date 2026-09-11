<script lang="ts">
  /** Índice dos favoritos e da mídia do perfil no TikTok, em CSV/JSON, com download opcional. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openUrl, pct, pickDir, reveal, baseName, type ToolProgress } from "$lib/tools/rt";

  type Entry = {
    id: string; author: string; author_name: string; description: string;
    sound: string; sound_author: string; sound_url: string;
    created: string; created_ts: number; duration: number; url: string;
    like_count: number; comment_count: number; view_count: number; save_count: number; local_path: string;
  };
  type Result = {
    source: string; user: string; entries: Entry[]; files: string[];
    downloaded: number; failed: number; dest: string; used_session: boolean; requests: number;
  };

  let user = $state("");
  let source = $state("posts");
  let dest = $state("");
  let indexFormat = $state("both");
  let limit = $state(200);
  let downloadMedia = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const needsSession = $derived(source === "favorites" || source === "liked");
  const ready = $derived(!!dest && user.trim().length > 0);

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "tt-favorites") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!ready || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_tt_favorites", {
        opts: {
          user: user.trim(),
          source,
          dest,
          index_format: indexFormat,
          limit: Number(limit) || 0,
          download_media: downloadMedia,
          delay_ms: 1200,
          cookies: null,
          account_slug: null,
        },
      });
      showToast("success", `${result.entries.length} ${$t("tools.ttfav.items")}`);
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
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ttfav.source")}</div><div class="group-row-sub">{needsSession ? $t("tools.ttfav.source_private") : $t("tools.ttfav.source_public")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={source}>
            <option value="posts">{$t("tools.ttfav.source_posts")}</option>
            <option value="favorites">{$t("tools.ttfav.source_favorites")}</option>
            <option value="liked">{$t("tools.ttfav.source_liked")}</option>
            <option value="collection">{$t("tools.ttfav.source_collection")}</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{source === "collection" ? $t("tools.ttfav.collection_url") : $t("tools.ttfav.user")}</div>
          <div class="group-row-sub">{source === "collection" ? $t("tools.ttfav.collection_hint") : $t("tools.ttfav.user_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="text" placeholder={source === "collection" ? "https://www.tiktok.com/@usuario/collection/…" : "@usuario"} bind:value={user} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.output_folder")}</div>
          <div class="group-row-sub mono">{dest || "—"}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ttfav.index_format")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={indexFormat}>
            <option value="csv">CSV</option>
            <option value="json">JSON</option>
            <option value="both">CSV + JSON</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ttfav.limit")}</div><div class="group-row-sub">{$t("tools.ttfav.limit_hint")}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="0" step="50" bind:value={limit} style:width="6em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ttfav.download_media")}</div><div class="group-row-sub">{$t("tools.ttfav.download_media_hint")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={downloadMedia} type="button" role="switch" aria-checked={downloadMedia} aria-label={$t("tools.ttfav.download_media")} onclick={() => (downloadMedia = !downloadMedia)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !ready} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.ttfav.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.entries.length} {$t("tools.ttfav.items")}</div>
            <div class="group-row-sub">
              <span class="tag" class:tag-success={result.used_session}>{result.used_session ? $t("tools.tt.session_on") : $t("tools.tt.session_off")}</span>
              @{result.user} · {result.downloaded} {$t("tools.ttfav.downloaded")}{result.failed ? ` · ${result.failed} ${$t("tools.ttfav.failed")}` : ""}
            </div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result?.dest ?? "")}>{$t("tools.common.reveal")}</button></div>
        </div>
        {#each result.files as f (f)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub mono">{baseName(f)}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(f)}>{$t("tools.common.reveal")}</button></div>
          </div>
        {/each}
      </div>
    </section>

    <section>
      <div class="group">
        {#each result.entries.slice(0, 80) as e (e.id)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">@{e.author} <span class="dim">{e.created}</span></div>
              <div class="group-row-sub">{e.description || "—"}</div>
              <div class="group-row-sub dim">{e.sound}{e.sound_author ? ` · ${e.sound_author}` : ""}</div>
            </div>
            <div class="group-row-trailing btn-row">
              {#if e.local_path}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(e.local_path)}>{$t("tools.common.reveal")}</button>{/if}
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(e.url)}>↗</button>
            </div>
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
