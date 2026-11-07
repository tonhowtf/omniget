<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/components/api";
  import { errorKey } from "$lib/components/errors";
  import { applyVars, buildVars, cssSnippet, flattenDtcg, hunks, lineDiff, suggestMapping, type DiffHunk, type FlatToken, type MappingRow } from "$lib/components/tokens";
  import type { ProjectInfo } from "$lib/components/types";
  import StatusIcon from "$components/converter/StatusIcon.svelte";
  import CopyButton from "./CopyButton.svelte";
  import DiffView from "./DiffView.svelte";

  let { project, onchooseproject }: { project: ProjectInfo | null; onchooseproject: () => void } = $props();

  type Preview = { rel: string; before: string; after: string; hunks: DiffHunk[]; count: number; skipped: string[] };
  type WriteState = { state: "idle" } | { state: "reading" } | { state: "writing" } | { state: "done"; path: string; count: number } | { state: "error"; code: string };

  let source = $state("");
  let fileName = $state<string | null>(null);
  let parseError = $state<string | null>(null);
  let tokens = $state<FlatToken[]>([]);
  let unresolved = $state<string[]>([]);
  let duplicates = $state<string[]>([]);
  let rows = $state<MappingRow[]>([]);
  let showUnmapped = $state(false);
  let cssRel = $state("");
  let preview = $state<Preview | null>(null);
  let write = $state<WriteState>({ state: "idle" });
  let fileInput = $state<HTMLInputElement | null>(null);

  let loaded = $derived(tokens.length > 0);
  let light = $derived(buildVars(rows, tokens, "light"));
  let dark = $derived(buildVars(rows, tokens, "dark"));
  let snippet = $derived(cssSnippet(light, dark));
  let mappedCount = $derived(rows.filter((r) => r.light || r.dark).length);
  let visibleRows = $derived(showUnmapped ? rows : rows.filter((r) => r.light || r.dark));
  let valueOf = $derived(new Map(tokens.map((tk) => [tk.path, tk.value])));
  let colorTokens = $derived(tokens.filter((tk) => tk.type !== "dimension" && tk.type !== "fontFamily" && tk.type !== "number"));

  let cssFor: string | null = null;

  $effect(() => {
    const path = project?.path ?? null;
    if (path === cssFor) return;
    cssFor = path;
    cssRel = project?.css_path ?? "";
    preview = null;
    write = { state: "idle" };
  });

  function read(text: string) {
    parseError = null;
    preview = null;
    write = { state: "idle" };
    let doc: unknown;
    try {
      doc = JSON.parse(text);
    } catch {
      parseError = "tokens_json";
      return;
    }
    const result = flattenDtcg(doc);
    if (result.tokens.length === 0) {
      parseError = "tokens_empty";
      return;
    }
    tokens = result.tokens;
    unresolved = result.unresolved;
    duplicates = result.duplicates;
    rows = suggestMapping(result.tokens);
    showUnmapped = rows.every((r) => !r.light && !r.dark);
  }

  async function pickFile(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    if (file.size > 5 * 1024 * 1024) {
      parseError = "too_large";
      return;
    }
    fileName = file.name;
    source = await file.text();
    read(source);
  }

  function reset() {
    tokens = [];
    rows = [];
    unresolved = [];
    duplicates = [];
    source = "";
    fileName = null;
    preview = null;
    write = { state: "idle" };
  }

  function isColor(value: string | undefined): boolean {
    return !!value && /^(#|rgb|hsl|oklch|oklab|lab|lch|color\()/i.test(value);
  }

  async function makePreview() {
    if (!project || !cssRel.trim()) return;
    write = { state: "reading" };
    preview = null;
    try {
      const rel = cssRel.trim();
      const before = await api.readCss(project.path, rel);
      const step = applyVars(before, ":root", light);
      const final = applyVars(step.css, ".dark", dark);
      const count = step.changes.length + final.changes.length;
      const skipped = [...new Set([...step.skipped, ...final.skipped])];
      preview = { rel, before, after: final.css, hunks: hunks(lineDiff(before, final.css)), count, skipped };
      write = { state: "idle" };
    } catch (e) {
      write = { state: "error", code: api.errorCode(e).code };
    }
  }

  async function confirmWrite() {
    if (!project || !preview || preview.count === 0) return;
    write = { state: "writing" };
    try {
      const path = await api.writeCss(project.path, preview.rel, preview.before, preview.after);
      write = { state: "done", path, count: preview.count };
      preview = null;
    } catch (e) {
      write = { state: "error", code: api.errorCode(e).code };
    }
  }
</script>

<div class="cr-tok">
  {#if !loaded}
    <div class="cr-tok-empty">
      <h2 class="cr-tok-title">{$t("components.tokens_empty_title")}</h2>
      <p class="cr-tok-body">{$t("components.tokens_empty_body")}</p>
      <div class="cr-tok-actions">
        <button type="button" class="btn btn-primary" onclick={() => fileInput?.click()}>{$t("components.tokens_choose_file")}</button>
        <input bind:this={fileInput} class="cr-hidden" type="file" accept=".json,application/json" onchange={pickFile} tabindex="-1" aria-hidden="true" />
      </div>
      <label class="cr-tok-label" for="cr-tok-paste">{$t("components.tokens_paste_label")}</label>
      <textarea id="cr-tok-paste" class="input cr-tok-paste" rows="6" spellcheck="false" bind:value={source} placeholder={'{ "color": { "primary": { "$type": "color", "$value": "#3366ff" } } }'}></textarea>
      <div class="cr-tok-actions">
        <button type="button" class="btn btn-secondary" disabled={!source.trim()} onclick={() => read(source)}>{$t("components.tokens_read")}</button>
      </div>
      {#if parseError}
        <p class="cr-tok-error" role="alert"><StatusIcon name="failed" size={14} /> {$t(errorKey(parseError), { detail: "" })}</p>
      {/if}
    </div>
  {:else}
    <div class="cr-tok-summary">
      <StatusIcon name={unresolved.length || duplicates.length ? "warning" : "done"} size={16} />
      <p>
        {$t("components.tokens_loaded", { count: tokens.length, mapped: mappedCount })}
        {#if fileName}<span class="cr-mono">· {fileName}</span>{/if}
        {#if unresolved.length}
          <br /><span class="cr-dim">{$t("components.tokens_unresolved", { count: unresolved.length, list: unresolved.slice(0, 4).join(", ") })}</span>
        {/if}
        {#if duplicates.length}
          <br /><span class="cr-dim">{$t("components.tokens_duplicates", { count: duplicates.length, list: duplicates.slice(0, 4).join(", ") })}</span>
        {/if}
      </p>
      <button type="button" class="btn btn-ghost btn-sm" onclick={reset}>{$t("components.tokens_other_file")}</button>
    </div>

    <section class="cr-tok-section" aria-labelledby="cr-map-title">
      <div class="cr-tok-section-head">
        <h3 id="cr-map-title" class="cr-tok-h3">{$t("components.mapping_title")}</h3>
        <label class="cr-check">
          <input type="checkbox" class="checkbox" bind:checked={showUnmapped} />
          <span>{$t("components.mapping_show_all")}</span>
        </label>
      </div>
      <p class="cr-dim cr-small">{$t("components.mapping_hint")}</p>
      {#if visibleRows.length === 0}
        <p class="cr-dim cr-small">{$t("components.mapping_none")}</p>
      {:else}
        <table class="cr-map">
          <thead>
            <tr>
              <th scope="col">{$t("components.mapping_variable")}</th>
              <th scope="col">{$t("components.mapping_light")}</th>
              <th scope="col">{$t("components.mapping_dark")}</th>
            </tr>
          </thead>
          <tbody>
            {#each visibleRows as row (row.name)}
              <tr>
                <th scope="row" class="cr-mono">--{row.name}</th>
                {#each ["light", "dark"] as const as mode (mode)}
                  {@const path = row[mode]}
                  <td>
                    <div class="cr-map-cell">
                      <span class="cr-swatch" class:empty={!isColor(path ? valueOf.get(path) : undefined)} style:background={path && isColor(valueOf.get(path)) ? valueOf.get(path) : undefined} aria-hidden="true"></span>
                      <select class="input cr-map-select" bind:value={row[mode]} aria-label={$t(mode === "light" ? "components.mapping_light_for" : "components.mapping_dark_for", { name: row.name })}>
                        <option value={null}>{$t("components.mapping_skip")}</option>
                        {#each row.name === "radius" ? tokens : colorTokens as tk (tk.path)}
                          <option value={tk.path}>{tk.path}</option>
                        {/each}
                      </select>
                    </div>
                  </td>
                {/each}
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

    <section class="cr-tok-section" aria-labelledby="cr-css-title">
      <div class="cr-tok-section-head">
        <h3 id="cr-css-title" class="cr-tok-h3">{$t("components.css_title")}</h3>
        {#if snippet}<CopyButton text={snippet} label={$t("components.copy_css") as string} />{/if}
      </div>
      {#if snippet}
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <pre class="cr-snippet" tabindex="0">{snippet}</pre>
      {:else}
        <p class="cr-dim cr-small">{$t("components.css_empty")}</p>
      {/if}
    </section>

    <section class="cr-tok-section cr-tok-write" aria-labelledby="cr-write-title">
      <h3 id="cr-write-title" class="cr-tok-h3">{$t("components.write_title")}</h3>
      {#if !project}
        <p class="cr-dim cr-small">{$t("components.write_need_project")}</p>
        <div class="cr-tok-actions">
          <button type="button" class="btn btn-secondary" onclick={onchooseproject}>{$t("components.choose_project")}</button>
        </div>
      {:else}
        <label class="cr-tok-label" for="cr-css-path">{$t("components.write_path_label", { project: project.name })}</label>
        <div class="cr-tok-row">
          <input id="cr-css-path" class="input cr-mono" spellcheck="false" placeholder="app/globals.css" bind:value={cssRel} oninput={() => (preview = null)} />
          <button type="button" class="btn btn-secondary" disabled={!snippet || !cssRel.trim() || write.state === "reading" || write.state === "writing"} onclick={makePreview}>
            {write.state === "reading" ? $t("components.write_reading") : $t("components.write_preview")}
          </button>
        </div>
        {#if !snippet}<p class="cr-dim cr-small">{$t("components.write_need_mapping")}</p>{/if}
        {#if !project.css_path}<p class="cr-dim cr-small">{$t("components.write_no_css_path")}</p>{/if}

        {#if preview}
          {#if preview.skipped.length}
            <p class="cr-tok-warn" role="status"><StatusIcon name="warning" size={14} /> {$t("components.write_skipped_hsl", { count: preview.skipped.length, list: preview.skipped.map((n) => `--${n}`).join(", ") })}</p>
          {/if}
          {#if preview.count === 0}
            <p class="cr-tok-ok" role="status"><StatusIcon name="done" size={14} /> {$t("components.write_nothing")}</p>
          {:else}
            <p class="cr-small">{$t("components.write_review", { count: preview.count, path: preview.rel })}</p>
            <DiffView hunks={preview.hunks} path={preview.rel} />
            <div class="cr-tok-actions">
              <button type="button" class="btn btn-primary" onclick={confirmWrite}>{$t("components.write_confirm", { count: preview.count, path: preview.rel })}</button>
              <button type="button" class="btn btn-ghost" onclick={() => (preview = null)}>{$t("components.write_cancel")}</button>
            </div>
          {/if}
        {/if}

        {#if write.state === "writing"}
          <p class="cr-small" role="status"><StatusIcon name="running" size={14} /> {$t("components.write_writing")}</p>
        {:else if write.state === "done"}
          <p class="cr-tok-ok" role="status"><StatusIcon name="done" size={14} /> {$t("components.write_done", { count: write.count, path: write.path })}</p>
        {:else if write.state === "error"}
          <p class="cr-tok-error" role="alert"><StatusIcon name="failed" size={14} /> {$t(errorKey(write.code), { detail: "" })}</p>
        {/if}
      {/if}
    </section>
  {/if}
</div>

<style>
  .cr-tok {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .cr-tok-empty {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-6) var(--space-5);
    border-radius: var(--radius-xl);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cr-tok-title {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .cr-tok-body {
    margin: 0;
    max-width: 560px;
    font-size: var(--text-base);
    color: var(--text-dim);
  }

  .cr-tok-label {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }

  .cr-tok-paste {
    width: 100%;
    height: auto;
    min-height: 120px;
    padding: var(--space-2) var(--space-3);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    resize: vertical;
  }

  .cr-tok-actions,
  .cr-tok-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .cr-tok-row .input {
    flex: 1;
    min-width: 200px;
  }

  .cr-hidden {
    display: none;
  }

  .cr-tok-error,
  .cr-tok-warn,
  .cr-tok-ok {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .cr-tok-summary {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cr-tok-summary p {
    flex: 1;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .cr-tok-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .cr-tok-write {
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cr-tok-section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .cr-tok-h3 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .cr-check {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .cr-dim {
    color: var(--text-dim);
  }

  .cr-small {
    margin: 0;
    font-size: var(--text-sm);
  }

  .cr-mono {
    font-family: var(--font-mono);
  }

  .cr-map {
    width: 100%;
    border-collapse: collapse;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-size: var(--text-sm);
  }

  .cr-map th,
  .cr-map td {
    padding: var(--space-1) var(--space-3);
    text-align: left;
    border-bottom: var(--hairline) solid var(--content-border);
  }

  .cr-map thead th {
    height: var(--row-compact);
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-dim);
  }

  .cr-map tbody th {
    font-weight: 500;
    color: var(--text);
    white-space: nowrap;
  }

  .cr-map tbody tr:last-child th,
  .cr-map tbody tr:last-child td {
    border-bottom: 0;
  }

  .cr-map-cell {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .cr-map-select {
    flex: 1;
    min-width: 0;
    max-width: 280px;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  .cr-swatch {
    flex-shrink: 0;
    width: 16px;
    height: 16px;
    border-radius: var(--radius-xs);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cr-swatch.empty {
    background: repeating-linear-gradient(45deg, var(--fill-2) 0 3px, transparent 3px 6px);
  }

  .cr-snippet {
    max-height: 280px;
    margin: 0;
    padding: var(--space-3);
    overflow: auto;
    border-radius: var(--radius-md);
    background: var(--fill-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.6;
    color: var(--text);
  }

  .cr-snippet:focus-visible {
    outline: var(--focus-ring);
  }
</style>
