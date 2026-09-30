<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { getSettings } from "$lib/stores/settings-store.svelte";

  type Row = {
    url: string;
    ok: boolean;
    title: string | null;
    txt_path: string | null;
    json_path: string | null;
    error: string | null;
  };

  let text = $state("");
  let withJson = $state(true);
  let running = $state(false);
  let rows = $state<Row[]>([]);
  let error = $state<string | null>(null);

  const urls = $derived(
    text
      .split(/\s+/)
      .map((u) => u.trim())
      .filter((u) => /^https?:\/\//i.test(u)),
  );
  const saved = $derived(rows.filter((r) => r.ok).length);

  async function save() {
    if (!urls.length || running) return;
    const outputDir = getSettings()?.download.default_output_dir ?? "";
    if (!outputDir) {
      error = $t("downloads.tools.post_text.no_folder");
      return;
    }
    running = true;
    error = null;
    rows = [];
    try {
      rows = await invoke<Row[]>("save_post_text", { urls, outputDir, withJson });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : (e?.message ?? "");
      error = msg ? translateBackendError(msg, $t) : $t("common.error");
    } finally {
      running = false;
    }
  }
</script>

<div class="pt" data-agent-readiness={running ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.post_text.lead")}</p>
  <textarea
    bind:value={text}
    rows="4"
    placeholder={$t("downloads.tools.post_text.placeholder")}
    aria-label={$t("downloads.tools.post_text.placeholder")}
    spellcheck="false"
    disabled={running}
  ></textarea>
  <div class="bar">
    <label><input type="checkbox" bind:checked={withJson} disabled={running} /> {$t("downloads.tools.post_text.with_json")}</label>
    <button class="button" onclick={save} disabled={running || !urls.length}>
      {running ? $t("downloads.tools.post_text.saving") : $t("downloads.tools.post_text.save", { count: urls.length })}
    </button>
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if rows.length}
    <p class="dim">{$t("downloads.tools.post_text.summary", { saved, total: rows.length })}</p>
    <ul class="list">
      {#each rows as r (r.url)}
        <li class:fail={!r.ok}>
          <span class="name">{r.title ?? r.url}</span>
          {#if r.ok && r.txt_path}
            <button class="link" onclick={() => void invoke("reveal_file", { path: r.txt_path })}>{$t("downloads.tools.post_text.show")}</button>
          {:else}
            <span class="err">{r.error ? translateBackendError(r.error, $t) : $t("common.error")}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .pt {
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
  textarea {
    width: 100%;
    box-sizing: border-box;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
    font-family: inherit;
    resize: vertical;
  }
  textarea:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .bar label {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-size: var(--text-sm);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .err {
    color: var(--danger, #d33);
    font-size: var(--text-sm);
    flex-shrink: 0;
    max-width: 50%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .link {
    border: none;
    background: none;
    color: var(--accent);
    font-size: var(--text-sm);
    cursor: pointer;
    padding: 2px var(--space-1);
    border-radius: var(--radius-full);
    flex-shrink: 0;
  }
  .link:hover {
    background: var(--accent-soft);
  }
</style>
