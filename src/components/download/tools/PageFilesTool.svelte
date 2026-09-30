<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { formatBytes } from "$lib/stores/download-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type PageFile = { url: string; name: string; ext: string; kind: string; size: number | null };
  type Result = { page_url: string; title: string | null; files: PageFile[]; truncated: boolean };

  let url = $state("");
  let loading = $state(false);
  let sending = $state(false);
  let error = $state<string | null>(null);
  let result = $state<Result | null>(null);
  let picked = $state<Set<string>>(new Set());
  let kind = $state("all");

  const kinds = $derived(result ? ["all", ...new Set(result.files.map((f) => f.kind))] : []);
  const shown = $derived(result ? result.files.filter((f) => kind === "all" || f.kind === kind) : []);
  const pickedSize = $derived(
    result ? result.files.filter((f) => picked.has(f.url)).reduce((a, f) => a + (f.size ?? 0), 0) : 0,
  );

  const errText = (e: any) => {
    const msg = typeof e === "string" ? e : (e?.message ?? "");
    return msg ? translateBackendError(msg, $t) : $t("common.error");
  };

  async function scan() {
    const target = url.trim();
    if (!target || loading) return;
    loading = true;
    error = null;
    result = null;
    picked = new Set();
    kind = "all";
    try {
      result = await invoke<Result>("scan_page_files", { url: target });
    } catch (e) {
      error = errText(e);
    } finally {
      loading = false;
    }
  }

  function toggle(u: string) {
    const next = new Set(picked);
    if (next.has(u)) next.delete(u);
    else next.add(u);
    picked = next;
  }

  function pickShown(on: boolean) {
    const next = new Set(picked);
    for (const f of shown) {
      if (on) next.add(f.url);
      else next.delete(f.url);
    }
    picked = next;
  }

  async function download() {
    const outputDir = getSettings()?.download.default_output_dir ?? "";
    if (!outputDir) {
      showToast("error", $t("downloads.tools.page_files.no_folder"));
      return;
    }
    sending = true;
    let queued = 0;
    let failed = 0;
    for (const f of result?.files.filter((x) => picked.has(x.url)) ?? []) {
      try {
        await invoke("download_from_url", {
          url: f.url,
          outputDir,
          downloadMode: null,
          quality: null,
          formatId: null,
          referer: result?.page_url ?? null,
          cookieSlug: null,
          // A .torrent keeps its own engine; everything else is a plain file.
          engine: f.kind === "torrent" ? null : "direct_file",
        });
        queued++;
      } catch {
        failed++;
      }
    }
    sending = false;
    showToast(
      failed ? "info" : "success",
      failed
        ? $t("downloads.tools.page_files.queued_some", { queued, failed })
        : $t("downloads.tools.page_files.queued", { count: queued }),
    );
  }
</script>

<div class="pf" data-agent-readiness={loading || sending ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.page_files.lead")}</p>
  <form
    class="row"
    onsubmit={(e) => {
      e.preventDefault();
      scan();
    }}
  >
    <input
      type="url"
      bind:value={url}
      placeholder={$t("downloads.tools.page_files.placeholder")}
      aria-label={$t("downloads.tools.page_files.placeholder")}
      spellcheck="false"
      autocomplete="off"
    />
    <button class="button" type="submit" disabled={loading || !url.trim()}>
      {loading ? $t("downloads.tools.page_files.scanning") : $t("downloads.tools.page_files.scan")}
    </button>
  </form>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if result}
    <p class="dim">
      {#if result.title}<strong>{result.title}</strong> · {/if}
      {$t("downloads.tools.page_files.count", { count: result.files.length })}
      {#if result.truncated}· {$t("downloads.tools.page_files.truncated")}{/if}
    </p>
    {#if result.files.length === 0}
      <p class="dim">{$t("downloads.tools.page_files.none")}</p>
    {:else}
      <div class="filters" role="group" aria-label={$t("downloads.tools.page_files.filter")}>
        {#each kinds as k (k)}
          <button class="chip" class:on={kind === k} aria-pressed={kind === k} onclick={() => (kind = k)}>
            {$t(`downloads.tools.page_files.kind_${k}`)}
          </button>
        {/each}
        <span class="spacer"></span>
        <button class="link" onclick={() => pickShown(true)}>{$t("downloads.tools.page_files.select_all")}</button>
        <button class="link" onclick={() => pickShown(false)}>{$t("downloads.tools.page_files.select_none")}</button>
      </div>
      <ul class="list">
        {#each shown as f (f.url)}
          <li class="file">
            <label>
              <input type="checkbox" checked={picked.has(f.url)} onchange={() => toggle(f.url)} />
              <span class="name" title={f.url}>{f.name}</span>
            </label>
            <span class="size">{f.size ? formatBytes(f.size) : $t("downloads.tools.page_files.size_unknown")}</span>
          </li>
        {/each}
      </ul>
      <div class="row">
        <button class="button" onclick={download} disabled={sending || picked.size === 0}>
          {sending
            ? $t("downloads.tools.page_files.sending")
            : $t("downloads.tools.page_files.download", { count: picked.size, size: formatBytes(pickedSize) })}
        </button>
      </div>
    {/if}
  {/if}
</div>

<style>
  .pf {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .row input {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .row input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .filters {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
    align-items: center;
  }
  .spacer {
    flex: 1;
  }
  .chip {
    padding: 2px var(--space-2);
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text-dim);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .chip.on {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 360px;
    overflow-y: auto;
  }
  .file {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-size: var(--text-sm);
  }
  .file label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    flex: 1;
    cursor: pointer;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .size {
    flex-shrink: 0;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .link {
    border: none;
    background: none;
    color: var(--accent);
    font-size: var(--text-sm);
    cursor: pointer;
    padding: 2px var(--space-1);
    border-radius: var(--radius-full);
  }
  .link:hover {
    background: var(--accent-soft);
  }
</style>
