<script lang="ts">
  /**
   * Backup de board / seção / perfil (estudo 67): a resposta ao "Mass-saving
   * your pins" do Reddit sem linha de comando. Originais, vídeos, pastas por
   * seção, pins.csv/pins.json, galeria offline e sincronização incremental.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openPath, openUrl, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";
  import { defaultDownload, defaultFilters, fmtCount, loadCookies, type BackupOut, type Inspect } from "$lib/tools/pinterest";
  import PinCookies from "./PinCookies.svelte";
  import PinDownloadOptions from "./PinDownloadOptions.svelte";
  import PinFilters from "./PinFilters.svelte";

  let { mode = "board" }: { mode?: "board" | "profile" } = $props();

  let url = $state("");
  let cookies = $state(loadCookies());
  let busy = $state<string | null>(null);
  let info = $state<Inspect | null>(null);
  let opts = $state(defaultDownload());
  let filters = $state({ ...defaultFilters(), skip_promoted: true, ai_level: 0 });
  let limit = $state(0);
  let metadata = $state(true);
  let gallery = $state(true);
  let includeCreated = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<BackupOut | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => { unlisten = await onToolProgress((p) => { if (p.id === "pinterest:backup") progress = p; }); });
  onDestroy(() => unlisten?.());

  async function inspect() {
    if (!url.trim() || busy) return;
    busy = "inspect"; info = null; result = null;
    try {
      info = await invoke<Inspect>("tool_pin_inspect", { url, cookies: cookies || null });
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }

  async function run() {
    if (!url.trim() || busy) return;
    if (!opts.dest) { const d = await pickDir(); if (!d) return; opts.dest = d; }
    busy = "run"; result = null; progress = null;
    try {
      result = await invoke<BackupOut>("tool_pin_backup", {
        opts: { url: info?.resolved_url ?? url, download: opts, cookies: cookies || null, limit, filters, metadata, gallery, include_created: includeCreated },
      });
      showToast(result.failed.length ? "info" : "success", `${result.downloaded} ${$t("tools.pinterest.downloaded")} · ${result.files} ${$t("tools.pinterest.files")}`);
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }

  let stageText = $derived.by(() => {
    if (!progress) return "";
    const map: Record<string, string> = { list: "stage_list", download: "stage_download", board: "stage_board" };
    const k = map[progress.stage];
    const base = k ? ($t(`tools.pinterest.${k}`) as string) : progress.stage;
    const n = progress.total ? `${progress.done}/${progress.total}` : String(progress.done);
    return `${base} ${n}${progress.message ? ` · ${progress.message}` : ""}`;
  });
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="text" bind:value={url} placeholder={mode === "profile" ? $t("tools.pinterest.profile_placeholder") : $t("tools.pinterest.board_placeholder")} onkeydown={(e) => e.key === "Enter" && inspect()} /></div>
        <div class="group-row-trailing"><button class="btn btn-secondary" type="button" disabled={busy !== null || !url.trim()} onclick={inspect}>{busy === "inspect" ? $t("tools.common.working") : $t("tools.pinterest.inspect")}</button></div>
      </div>
      <PinCookies bind:value={cookies} />
    </div>
  </section>

  {#if info}
    <section>
      <div class="group">
        {#if info.board}
          <div class="group-row">
            {#if info.board.cover}<img class="cover" src={info.board.cover} alt="" />{/if}
            <div class="group-row-content">
              <div class="group-row-title strong">{info.board.name}{#if info.section} · {info.section.title}{/if}</div>
              {#if info.board.description}<div class="group-row-sub">{info.board.description}</div>{/if}
              <div class="group-row-sub">
                {fmtCount(info.section ? info.section.pin_count : info.board.pin_count)} {$t("tools.pinterest.pins")}
                {#if !info.section && info.board.section_count} · {info.board.section_count} {$t("tools.pinterest.sections")}{/if}
                · {fmtCount(info.board.follower_count)} {$t("tools.pinterest.followers")}
                {#if info.board.owner?.username} · {info.board.owner.name ?? info.board.owner.username}{/if}
                {#if info.board.privacy !== "public"} · <span class="tag tag-warning">{$t("tools.pinterest.secret")}</span>{/if}
              </div>
              {#if info.sections.length && !info.section}
                <div class="chips">{#each info.sections as s (s.id)}<span class="tag">{s.title} · {s.pin_count}</span>{/each}</div>
              {/if}
            </div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(info!.board!.url)}>↗</button></div>
          </div>
        {:else if info.user}
          <div class="group-row">
            {#if info.user.avatar}<img class="avatar" src={info.user.avatar} alt="" />{/if}
            <div class="group-row-content">
              <div class="group-row-title strong">{info.user.name || info.user.username} <span class="dim">@{info.user.username}</span></div>
              {#if info.user.about}<div class="group-row-sub">{info.user.about}</div>{/if}
              <div class="group-row-sub">{fmtCount(info.user.pin_count)} {$t("tools.pinterest.pins")} · {info.user.board_count} {$t("tools.pinterest.boards")} · {fmtCount(info.user.follower_count)} {$t("tools.pinterest.followers")} · {fmtCount(info.user.following_count)} {$t("tools.pinterest.following")}{#if info.user.website} · {info.user.website}{/if}</div>
              {#if info.boards.length}
                <div class="chips">{#each info.boards.slice(0, 40) as b (b.id)}<span class="tag">{b.name} · {b.pin_count}</span>{/each}{#if info.boards.length > 40}<span class="tag">+{info.boards.length - 40}</span>{/if}</div>
              {/if}
            </div>
          </div>
        {:else}
          <div class="group-row"><div class="group-row-sub">{info.target.kind === "search" ? `${$t("tools.pinterest.search")}: ${info.target.query}` : info.resolved_url}</div></div>
        {/if}
      </div>
    </section>
  {/if}

  <section>
    <span class="group-label">{$t("tools.pinterest.options")}</span>
    <div class="group">
      <PinDownloadOptions bind:opts />
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.limit")}</div><div class="group-row-sub">{$t("tools.pinterest.limit_hint")}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="0" step="50" bind:value={limit} style:width="7em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.metadata")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={metadata} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.gallery")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={gallery} /></div>
      </div>
      {#if mode === "profile"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.include_created")}</div></div>
          <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={includeCreated} /></div>
        </div>
      {/if}
    </div>
  </section>

  <section>
    <span class="group-label">{$t("tools.pinterest.filters")}</span>
    <div class="group">
      <PinFilters bind:filters />
      <div class="group-row">
        <div class="group-row-content">
          {#if busy === "run"}<div class="group-row-sub">{stageText}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy !== null || !url.trim()} onclick={run}>{busy === "run" ? $t("tools.common.working") : $t("tools.pinterest.backup")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <span class="group-label">{result.title}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.downloaded} {$t("tools.pinterest.downloaded")} · {result.skipped} {$t("tools.pinterest.skipped")} · {result.failed.length} {$t("tools.pinterest.failed")} · {result.files} {$t("tools.pinterest.files")}{#if result.hidden} · {result.hidden} {$t("tools.pinterest.hidden")}{/if}{#if result.boards > 1} · {result.boards} {$t("tools.pinterest.boards_done")}{/if}</div>
            <div class="group-row-sub mono">{result.dest}</div>
          </div>
          <div class="group-row-trailing btn-row">
            {#if result.html_path}<button class="btn btn-secondary btn-sm" type="button" onclick={() => openPath(result!.html_path!)}>{$t("tools.pinterest.open_gallery")}</button>{/if}
            {#if result.csv_path}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result!.csv_path!)}>{$t("tools.pinterest.csv")}</button>{/if}
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.dest)}>{$t("tools.common.reveal")}</button>
          </div>
        </div>
        {#each result.failed.slice(0, 30) as f (f.id)}
          <div class="group-row"><div class="group-row-sub mono">{f.id} · {f.error}</div></div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .strong { font-weight: 600; }
  .dim { color: var(--text-dim); font-weight: 400; }
  .cover { width: 72px; height: 72px; object-fit: cover; border-radius: var(--radius-md); flex-shrink: 0; }
  .avatar { width: 56px; height: 56px; object-fit: cover; border-radius: 50%; flex-shrink: 0; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
</style>
