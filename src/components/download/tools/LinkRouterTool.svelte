<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Routed = { url: string; engine: string; choices: string[] };
  type Report = { links: Routed[]; duplicates: number; ignored: number; truncated: boolean };
  type Row = Routed & { pick: string; state: "idle" | "queued" | "failed"; error: string | null };

  let text = $state("");
  let checking = $state(false);
  let sending = $state(false);
  let error = $state<string | null>(null);
  let report = $state<Report | null>(null);
  let rows = $state<Row[]>([]);

  const pending = $derived(rows.filter((r) => r.state !== "queued" && r.engine));

  const errText = (e: any) => {
    const msg = typeof e === "string" ? e : (e?.message ?? "");
    return msg ? translateBackendError(msg, $t) : $t("common.error");
  };

  function engineLabel(name: string): string {
    switch (name) {
      case "generic":
        return $t("downloads.tools.link_router.engine_ytdlp");
      case "gallery":
        return $t("downloads.tools.link_router.engine_gallery");
      case "direct_file":
        return $t("downloads.tools.link_router.engine_direct");
      case "magnet":
        return $t("downloads.tools.link_router.engine_torrent");
      case "p2p":
        return $t("downloads.tools.link_router.engine_p2p");
      case "":
        return $t("downloads.tools.link_router.engine_none");
      default:
        return $t("downloads.tools.link_router.engine_own", { name: name.charAt(0).toUpperCase() + name.slice(1) });
    }
  }

  async function check() {
    if (!text.trim() || checking) return;
    checking = true;
    error = null;
    report = null;
    rows = [];
    try {
      const r = await invoke<Report>("route_links", { text });
      report = r;
      rows = r.links.map((l) => ({ ...l, pick: "auto", state: "idle", error: null }));
    } catch (e) {
      error = errText(e);
    } finally {
      checking = false;
    }
  }

  async function openFile() {
    const selected = await open({
      title: $t("downloads.tools.link_router.open_file"),
      filters: [{ name: "Text", extensions: ["txt", "list", "csv"] }],
      multiple: false,
    });
    if (!selected || typeof selected !== "string") return;
    try {
      const urls = await invoke<string[]>("parse_batch_file", { path: selected });
      if (urls.length === 0) {
        showToast("info", $t("downloads.tools.link_router.file_empty"));
        return;
      }
      text = urls.join("\n");
      await check();
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function downloadAll() {
    const outputDir = getSettings()?.download.default_output_dir ?? "";
    if (!outputDir) {
      showToast("error", $t("downloads.tools.link_router.no_folder"));
      return;
    }
    sending = true;
    let queued = 0;
    // One at a time: the queue decides how many run together.
    for (const row of pending) {
      try {
        await invoke("download_from_url", {
          url: row.url,
          outputDir,
          downloadMode: null,
          quality: null,
          formatId: null,
          referer: null,
          cookieSlug: null,
          engine: row.pick === "auto" ? null : row.pick,
        });
        row.state = "queued";
        row.error = null;
        queued++;
      } catch (e) {
        row.state = "failed";
        row.error = errText(e);
      }
    }
    sending = false;
    const failed = rows.filter((r) => r.state === "failed").length;
    showToast(
      failed ? "info" : "success",
      failed
        ? $t("downloads.tools.link_router.queued_some", { queued, failed })
        : $t("downloads.tools.link_router.queued_all", { count: queued }),
    );
  }
</script>

<div class="lr" data-agent-readiness={checking || sending ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.link_router.lead")}</p>
  <textarea
    bind:value={text}
    rows="6"
    placeholder={$t("downloads.tools.link_router.placeholder")}
    aria-label={$t("downloads.tools.link_router.placeholder")}
    spellcheck="false"
  ></textarea>
  <div class="row">
    <button class="button" onclick={check} disabled={checking || !text.trim()}>
      {checking ? $t("downloads.tools.link_router.checking") : $t("downloads.tools.link_router.check")}
    </button>
    <button class="link" onclick={openFile}>{$t("downloads.tools.link_router.open_file")}</button>
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if report}
    <p class="dim">
      {$t("downloads.tools.link_router.summary", { count: rows.length, duplicates: report.duplicates, ignored: report.ignored })}
      {#if report.truncated}· {$t("downloads.tools.link_router.truncated")}{/if}
    </p>
    {#if rows.length}
      <ul class="list">
        {#each rows as row (row.url)}
          <li class="link-row" data-engine={row.engine} data-state={row.state}>
            <div class="info">
              <span class="url" title={row.url}>{row.url}</span>
              {#if row.error}
                <span class="error small">{row.error}</span>
              {/if}
            </div>
            {#if row.state === "queued"}
              <span class="badge on">{$t("downloads.tools.link_router.queued")}</span>
            {:else if row.choices.length}
              <select bind:value={row.pick} aria-label={$t("downloads.tools.link_router.engine")}>
                <option value="auto">{$t("downloads.tools.link_router.auto", { engine: engineLabel(row.engine) })}</option>
                {#each row.choices.filter((c) => c !== row.engine) as c (c)}
                  <option value={c}>{engineLabel(c)}</option>
                {/each}
              </select>
            {:else}
              <span class="badge">{engineLabel(row.engine)}</span>
            {/if}
          </li>
        {/each}
      </ul>
      <div class="row">
        <button class="button" onclick={downloadAll} disabled={sending || pending.length === 0}>
          {sending ? $t("downloads.tools.link_router.sending") : $t("downloads.tools.link_router.download_all", { count: pending.length })}
        </button>
      </div>
    {/if}
  {/if}
</div>

<style>
  .lr {
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
  .small {
    font-size: var(--text-xs, 11px);
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-family: var(--font-mono, monospace);
    font-size: var(--text-sm);
    resize: vertical;
  }
  textarea:focus-visible,
  select:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .link-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .url {
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  select {
    flex-shrink: 0;
    max-width: 50%;
    padding: 2px var(--space-2);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .badge {
    flex-shrink: 0;
    font-size: 11px;
    padding: 1px 6px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .badge.on {
    background: var(--accent-soft);
    color: var(--accent);
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
