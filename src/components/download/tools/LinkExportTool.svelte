<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { save } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluralKey } from "$lib/home/plural";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Entry = { url: string; title: string; success: boolean; completed_at: number };
  type Which = "failed" | "done" | "all";
  type Since = "day" | "week" | "ever";

  let history = $state<Entry[]>([]);
  let loaded = $state(false);
  let which = $state<Which>("failed");
  let since = $state<Since>("ever");

  onMount(async () => {
    try {
      history = await invoke<Entry[]>("get_download_history");
    } catch {
      history = [];
    }
    loaded = true;
  });

  const ms = (ts: number) => (ts < 1e12 ? ts * 1000 : ts);

  const chosen = $derived.by(() => {
    const cut = since === "day" ? Date.now() - 864e5 : since === "week" ? Date.now() - 7 * 864e5 : 0;
    const seen = new Set<string>();
    return history.filter((h) => {
      if (which === "failed" && h.success) return false;
      if (which === "done" && !h.success) return false;
      if (cut && ms(h.completed_at) < cut) return false;
      if (seen.has(h.url)) return false;
      seen.add(h.url);
      return true;
    });
  });

  async function copy() {
    try {
      await navigator.clipboard.writeText(chosen.map((h) => h.url).join("\n") + "\n");
      showToast("success", $t("downloads.tools.link_export.copied"));
    } catch {
      showToast("error", $t("common.error"));
    }
  }

  async function saveFile() {
    const path = await save({
      defaultPath: `omniget-${which}-links.txt`,
      filters: [{ name: "Text", extensions: ["txt"] }],
    });
    if (!path) return;
    try {
      const n = await invoke<number>("export_link_list", {
        path,
        header: `OmniGet, ${which} downloads, ${new Date().toISOString().slice(0, 10)}`,
        urls: chosen.map((h) => h.url),
      });
      showToast("success", $t(pluralKey("downloads.tools.link_export.saved", n), { count: n }));
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : $t("common.error"));
    }
  }
</script>

<div class="le" data-agent-readiness={loaded ? "ready" : "loading"}>
  <p class="lead">{$t("downloads.tools.link_export.lead")}</p>
  <div class="row">
    <select bind:value={which} aria-label={$t("downloads.tools.link_export.which")}>
      <option value="failed">{$t("downloads.tools.link_export.failed")}</option>
      <option value="done">{$t("downloads.tools.link_export.done")}</option>
      <option value="all">{$t("downloads.tools.link_export.all")}</option>
    </select>
    <select bind:value={since} aria-label={$t("downloads.tools.link_export.since")}>
      <option value="day">{$t("downloads.tools.link_export.day")}</option>
      <option value="week">{$t("downloads.tools.link_export.week")}</option>
      <option value="ever">{$t("downloads.tools.link_export.ever")}</option>
    </select>
  </div>
  <p class="count">{$t(pluralKey("downloads.tools.link_export.count", chosen.length), { count: chosen.length })}</p>
  {#if chosen.length}
    <ul class="list">
      {#each chosen.slice(0, 8) as h (h.url)}
        <li title={h.url}>{h.title || h.url}</li>
      {/each}
      {#if chosen.length > 8}<li class="dim">…</li>{/if}
    </ul>
  {/if}
  <div class="row">
    <button class="button" onclick={copy} disabled={!chosen.length}>{$t("downloads.tools.link_export.copy")}</button>
    <button class="button primary" onclick={saveFile} disabled={!chosen.length}>{$t("downloads.tools.link_export.save")}</button>
  </div>
  <p class="dim small">{$t("downloads.tools.link_export.note")}</p>
</div>

<style>
  .le {
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
  .row {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  select {
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .count {
    margin: 0;
    font-weight: 600;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    font-size: var(--text-sm);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .list li {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
