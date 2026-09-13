<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import PageHero from "$lib/study-components/PageHero.svelte";

  type NotetypeSummary = {
    id: number;
    name: string;
    kind: string;
    field_count: number;
    template_count: number;
  };

  type DeckSummary = {
    id: number;
    name: string;
    filtered: boolean;
  };

  type ApkgSummary = {
    notetypes_added: number;
    decks_added: number;
    notes_added: number;
    cards_added: number;
    revlog_added: number;
    skipped_existing_notes: number;
    media_added: number;
  };

  type JsonSummary = {
    notetypes_added: number;
    decks_added: number;
    deck_configs_added: number;
    notes_added: number;
    cards_added: number;
    revlog_added: number;
    skipped_existing_notes: number;
  };

  type CsvSummary = {
    imported: number;
    skipped: number;
    errors: string[];
  };

  type Result =
    | { kind: "apkg"; data: ApkgSummary }
    | { kind: "json"; data: JsonSummary }
    | { kind: "csv"; data: CsvSummary };

  let notetypes = $state<NotetypeSummary[]>([]);
  let decks = $state<DeckSummary[]>([]);
  let csvNotetypeId = $state<number | null>(null);
  let csvDeckId = $state<number | null>(null);
  let csvHasHeader = $state(true);
  let csvDelimiter = $state<"" | "\t" | "," | ";">("");

  let busy = $state(false);
  let error = $state("");
  let result = $state<Result | null>(null);
  let lastSourcePath = $state("");

  type ExportKind = "apkg" | "colpkg" | "json" | "csv";
  let exporting = $state<ExportKind | null>(null);
  let exportNotetypeId = $state<number | null>(null);
  let exportDelimiter = $state<"" | "\t" | "," | ";">("");
  let exportToast = $state<{ kind: "ok" | "err"; msg: string } | null>(null);

  function showExportToast(kind: "ok" | "err", msg: string) {
    exportToast = { kind, msg };
    setTimeout(() => (exportToast = null), 3000);
  }

  async function refreshLookups() {
    try {
      const [nts, ds] = await Promise.all([
        pluginInvoke<NotetypeSummary[]>("study", "study:anki:notetypes:list"),
        pluginInvoke<DeckSummary[]>("study", "study:anki:decks:list"),
      ]);
      notetypes = nts;
      decks = ds.filter((d) => !d.filtered);
      if (csvNotetypeId === null && notetypes.length > 0) {
        csvNotetypeId = notetypes[0].id;
      }
      if (exportNotetypeId === null && notetypes.length > 0) {
        exportNotetypeId = notetypes[0].id;
      }
      if (csvDeckId === null) {
        const def = decks.find((d) => d.id === 1) ?? decks[0];
        if (def) csvDeckId = def.id;
      }
    } catch (e) {
      console.error("lookup failed", e);
    }
  }

  onMount(async () => {
    try {
      await pluginInvoke("study", "study:anki:storage:open");
    } catch (e) {
      console.error("open storage failed", e);
    }
    await refreshLookups();
  });

  function detectKind(path: string): "apkg" | "colpkg" | "json" | "csv" | null {
    const lower = path.toLowerCase();
    if (lower.endsWith(".colpkg")) return "colpkg";
    if (lower.endsWith(".apkg")) return "apkg";
    if (lower.endsWith(".json")) return "json";
    if (lower.endsWith(".csv") || lower.endsWith(".tsv") || lower.endsWith(".txt"))
      return "csv";
    return null;
  }

  async function pickAndImport() {
    error = "";
    result = null;
    try {
      const dialog = await import("@tauri-apps/plugin-dialog");
      const picked = await dialog.open({
        multiple: false,
        filters: [
          {
            name: "Anki collections / notes",
            extensions: ["apkg", "colpkg", "json", "csv", "tsv", "txt"],
          },
        ],
      });
      if (typeof picked !== "string" || !picked) return;
      lastSourcePath = picked;
      const kind = detectKind(picked);
      if (!kind) {
        error = `Formato não reconhecido: ${picked}`;
        return;
      }
      busy = true;
      if (kind === "apkg") {
        const data = await pluginInvoke<ApkgSummary>(
          "study",
          "study:anki:import:apkg",
          { sourcePath: picked },
        );
        result = { kind: "apkg", data };
      } else if (kind === "colpkg") {
        const data = await pluginInvoke<ApkgSummary>(
          "study",
          "study:anki:import:colpkg",
          { sourcePath: picked },
        );
        result = { kind: "apkg", data };
      } else if (kind === "json") {
        const data = await pluginInvoke<JsonSummary>(
          "study",
          "study:anki:import:json",
          { sourcePath: picked },
        );
        result = { kind: "json", data };
      } else {
        if (csvNotetypeId === null || csvDeckId === null) {
          error = "Selecione um modelo e um deck antes de importar CSV.";
          return;
        }
        const delim = csvDelimiter === "" ? null : csvDelimiter;
        const data = await pluginInvoke<CsvSummary>(
          "study",
          "study:anki:import:csv_notes",
          {
            notetypeId: csvNotetypeId,
            deckId: csvDeckId,
            sourcePath: picked,
            delimiter: delim,
            hasHeader: csvHasHeader,
          },
        );
        result = { kind: "csv", data };
      }
      await refreshLookups();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function pickAndExport(kind: ExportKind) {
    if (exporting) return;
    exporting = kind;
    try {
      const dialog = await import("@tauri-apps/plugin-dialog");
      const ext =
        kind === "apkg" ? "apkg"
        : kind === "colpkg" ? "colpkg"
        : kind === "json" ? "json"
        : "csv";
      const defaultName = `omniget-${kind}-${new Date().toISOString().slice(0, 10)}.${ext}`;
      const target = await dialog.save({
        defaultPath: defaultName,
        filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
      });
      if (typeof target !== "string" || !target) {
        exporting = null;
        return;
      }
      if (kind === "apkg") {
        await pluginInvoke("study", "study:anki:export:apkg", { targetPath: target });
      } else if (kind === "colpkg") {
        await pluginInvoke("study", "study:anki:export:colpkg", { targetPath: target });
      } else if (kind === "json") {
        await pluginInvoke("study", "study:anki:export:json", { targetPath: target });
      } else {
        if (exportNotetypeId == null) {
          showExportToast("err", "Selecione um modelo para exportar CSV");
          return;
        }
        const delim = exportDelimiter === "" ? null : exportDelimiter;
        await pluginInvoke("study", "study:anki:export:csv_notes", {
          notetypeId: exportNotetypeId,
          targetPath: target,
          delimiter: delim,
        });
      }
      showExportToast("ok", `Exportado · ${target.split(/[\\/]/).pop()}`);
    } catch (e) {
      showExportToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      exporting = null;
    }
  }

  function summaryRows(r: Result): { label: string; value: number | string }[] {
    if (r.kind === "apkg") {
      return [
        { label: $t("study.anki.import.notes_added"), value: r.data.notes_added },
        { label: $t("study.anki.import.cards_added"), value: r.data.cards_added },
        { label: $t("study.anki.import.decks_new"), value: r.data.decks_added },
        { label: $t("study.anki.import.notetypes_new"), value: r.data.notetypes_added },
        { label: $t("study.anki.import.media_copied"), value: r.data.media_added },
        { label: $t("study.anki.import.revlog_imported"), value: r.data.revlog_added },
        { label: $t("study.anki.import.notes_skipped"), value: r.data.skipped_existing_notes },
      ];
    }
    if (r.kind === "json") {
      return [
        { label: "Notes adicionadas", value: r.data.notes_added },
        { label: "Cards adicionados", value: r.data.cards_added },
        { label: "Decks novos", value: r.data.decks_added },
        { label: $t("study.anki.import.deck_configs_new"), value: r.data.deck_configs_added },
        { label: "Modelos novos", value: r.data.notetypes_added },
        { label: "Revlog importado", value: r.data.revlog_added },
        { label: $t("study.anki.import.notes_skipped"), value: r.data.skipped_existing_notes },
      ];
    }
    return [
      { label: $t("study.anki.import.rows_imported"), value: r.data.imported },
      { label: $t("study.anki.import.rows_skipped"), value: r.data.skipped },
    ];
  }
</script>

<section class="study-page">
  <PageHero title={$t("study.anki.import.title")} subtitle={$t("study.anki.import.subtitle")} />

  <div class="format-grid">
    <article class="format-card">
      <h3>.apkg / .colpkg</h3>
      <p>{$t("study.anki.import.fmt_apkg")}</p>
      <small class="muted">{$t("study.anki.import.fmt_apkg_conflicts")}</small>
    </article>
    <article class="format-card">
      <h3>.json</h3>
      <p>{$t("study.anki.import.fmt_json")}</p>
    </article>
    <article class="format-card csv-card">
      <h3>.csv / .tsv</h3>
      <p>{$t("study.anki.import.fmt_csv")}</p>
      <div class="csv-options">
        <label>
          <span>{$t("study.anki.revlog.th_notetype")}</span>
          <select bind:value={csvNotetypeId} disabled={busy}>
            {#each notetypes as nt (nt.id)}
              <option value={nt.id}>{nt.name}</option>
            {/each}
          </select>
        </label>
        <label>
          <span>{$t("study.anki.sidebar.decks")}</span>
          <select bind:value={csvDeckId} disabled={busy}>
            {#each decks as d (d.id)}
              <option value={d.id}>{d.name}</option>
            {/each}
          </select>
        </label>
        <label>
          <span>{$t("study.anki.import.delimiter")}</span>
          <select bind:value={csvDelimiter} disabled={busy}>
            <option value="">Auto</option>
            <option value={"\t"}>Tab</option>
            <option value=",">{$t("study.anki.import.comma")}</option>
            <option value=";">{$t("study.anki.import.semicolon")}</option>
          </select>
        </label>
        <label class="checkbox">
          <input type="checkbox" bind:checked={csvHasHeader} disabled={busy} />
          <span>{$t("study.anki.import.first_row_header")}</span>
        </label>
      </div>
    </article>
  </div>

  <div class="cta-row">
    <button type="button" class="btn-primary" onclick={pickAndImport} disabled={busy}>
      {busy ? $t("study.anki.import.importing") : $t("study.anki.import.pick_and_import")}
    </button>
    {#if lastSourcePath && !busy}
      <span class="last-path">{$t("study.anki.import.last")}: {lastSourcePath}</span>
    {/if}
  </div>

  {#if error}
    <p class="error">{error}</p>
  {/if}

  <section class="export-section">
    <h2 class="section-heading">{$t("study.anki.import.export")}</h2>
    <p class="section-lede">
      {$t("study.anki.import.export_lede")}
    </p>

    {#if exportToast}
      <div class="toast" class:err={exportToast.kind === "err"} role="status">
        {exportToast.msg}
      </div>
    {/if}

    <div class="export-grid">
      <article class="export-card">
        <h3>.apkg</h3>
        <p>{$t("study.anki.import.export_apkg_desc")}</p>
        <button
          type="button"
          class="btn-secondary"
          onclick={() => pickAndExport("apkg")}
          disabled={exporting !== null || busy}
        >
          {exporting === "apkg" ? $t("study.anki.import.exporting") : $t("study.anki.import.export_apkg")}
        </button>
      </article>

      <article class="export-card">
        <h3>.colpkg</h3>
        <p>{$t("study.anki.import.export_colpkg_desc")}</p>
        <button
          type="button"
          class="btn-secondary"
          onclick={() => pickAndExport("colpkg")}
          disabled={exporting !== null || busy}
        >
          {exporting === "colpkg" ? $t("study.anki.import.exporting") : $t("study.anki.import.export_colpkg")}
        </button>
      </article>

      <article class="export-card">
        <h3>.json</h3>
        <p>{$t("study.anki.import.export_json_desc")}</p>
        <button
          type="button"
          class="btn-secondary"
          onclick={() => pickAndExport("json")}
          disabled={exporting !== null || busy}
        >
          {exporting === "json" ? $t("study.anki.import.exporting") : $t("study.anki.import.export_json")}
        </button>
      </article>

      <article class="export-card csv-card">
        <h3>.csv (notes)</h3>
        <p>{$t("study.anki.import.export_csv_desc")}</p>
        <div class="export-options">
          <label>
            <span>Modelo</span>
            <select bind:value={exportNotetypeId} disabled={exporting !== null}>
              {#each notetypes as nt (nt.id)}
                <option value={nt.id}>{nt.name}</option>
              {/each}
            </select>
          </label>
          <label>
            <span>Delimitador</span>
            <select bind:value={exportDelimiter} disabled={exporting !== null}>
              <option value="">Auto</option>
              <option value={"\t"}>Tab</option>
              <option value=",">Vírgula</option>
              <option value=";">Ponto-e-vírgula</option>
            </select>
          </label>
        </div>
        <button
          type="button"
          class="btn-secondary"
          onclick={() => pickAndExport("csv")}
          disabled={exporting !== null || busy || exportNotetypeId == null}
        >
          {exporting === "csv" ? $t("study.anki.import.exporting") : $t("study.anki.import.export_csv")}
        </button>
      </article>
    </div>
  </section>

  {#if result}
    <section class="card result-card">
      <header class="card-head">
        <h2>{$t("study.anki.import.import_done")}</h2>
        <span class="kind-badge">{result.kind.toUpperCase()}</span>
      </header>
      <ul class="summary-list">
        {#each summaryRows(result) as row (row.label)}
          <li>
            <span class="row-label">{row.label}</span>
            <span class="row-value">{row.value}</span>
          </li>
        {/each}
      </ul>
      {#if result.kind === "csv" && result.data.errors.length > 0}
        <details class="errors">
          <summary>{$t("study.anki.import.error_rows", { n: result.data.errors.length })}</summary>
          <ul>
            {#each result.data.errors.slice(0, 20) as err (err)}
              <li>{err}</li>
            {/each}
            {#if result.data.errors.length > 20}
              <li class="muted">{$t("study.anki.import.more_n", { n: result.data.errors.length - 20 })}</li>
            {/if}
          </ul>
        </details>
      {/if}
      <footer class="card-foot">
        <a class="back-link" href="/study/anki">Ver no painel →</a>
      </footer>
    </section>
  {/if}
</section>

<style>
  .study-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
    width: 100%;
    max-width: 900px;
    margin-inline: auto;
  }

  .format-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: var(--padding);
  }
  .format-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: calc(var(--padding) * 1.5);
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
  }
  .format-card h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
    font-family: var(--font-mono, monospace);
  }
  .format-card p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    line-height: 1.5;
  }
  .csv-card {
    grid-column: 1 / -1;
  }
  .csv-options {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 10px;
    margin-top: 10px;
  }
  .csv-options label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
    color: var(--tertiary);
  }
  .csv-options label.checkbox {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    color: var(--secondary);
    font-size: 12px;
  }
  .csv-options select {
    background: var(--input-bg);
    border: none;
    color: var(--secondary);
    padding: 6px 8px;
    border-radius: var(--border-radius);
    font-size: 13px;
  }
  .csv-options select:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: -1px;
  }

  .cta-row {
    display: flex;
    align-items: center;
    gap: var(--padding);
    flex-wrap: wrap;
  }
  .btn-primary {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 10px 24px;
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
    border: 0;
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    transition: filter 150ms ease;
  }
  .btn-primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .btn-primary:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }
  .btn-primary:disabled {
    background: color-mix(in oklab, var(--input-border) 80%, transparent);
    color: var(--tertiary);
    cursor: not-allowed;
  }
  .last-path {
    font-size: 11px;
    color: var(--tertiary);
    font-family: var(--font-mono, monospace);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 60%;
  }

  .error {
    color: var(--error);
    font-size: 13px;
    padding: 8px 12px;
    background: color-mix(in oklab, var(--error) 8%, transparent);
    border: 1px solid color-mix(in oklab, var(--error) 25%, var(--input-border));
    border-radius: var(--border-radius);
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding: calc(var(--padding) * 2);
    border: none;
    border-radius: var(--border-radius);
    background: var(--button-elevated);
  }
  .card-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--padding);
  }
  .card-head h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 500;
    color: var(--secondary);
  }
  .kind-badge {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.8px;
    padding: 2px 8px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--accent) 18%, transparent);
    color: var(--accent);
    font-family: var(--font-mono, monospace);
  }
  .summary-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 8px;
  }
  .summary-list li {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    padding: 8px 12px;
    background: var(--bg);
    border: none;
    border-radius: var(--border-radius);
  }
  .row-label {
    color: var(--tertiary);
    font-size: 12px;
  }
  .row-value {
    color: var(--accent);
    font-family: var(--font-mono, monospace);
    font-variant-numeric: tabular-nums;
    font-weight: 500;
  }
  .errors summary {
    cursor: pointer;
    font-size: 13px;
    color: var(--secondary);
    padding: 6px 0;
  }
  .errors ul {
    margin: 6px 0 0;
    padding-left: 20px;
    font-size: 12px;
    color: var(--tertiary);
    line-height: 1.5;
  }
  .muted {
    color: var(--tertiary);
    font-size: 11px;
  }
  .card-foot {
    display: flex;
    justify-content: flex-end;
    padding-top: var(--padding);
    border-top: none;
  }
  .back-link {
    color: var(--accent);
    text-decoration: none;
    font-size: 13px;
    padding: 4px 10px;
    border-radius: var(--border-radius);
    transition: background 150ms ease;
  }
  .back-link:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .back-link:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }

  .export-section {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding-top: calc(var(--padding) * 1.5);
    border-top: none;
  }
  .section-heading {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--secondary);
  }
  .section-lede {
    margin: 0;
    color: var(--tertiary);
    font-size: 13px;
    line-height: 1.5;
  }

  .export-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: var(--padding);
  }
  .export-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: calc(var(--padding) * 1.25);
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
  }
  .export-card h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
    font-family: var(--font-mono, monospace);
  }
  .export-card p {
    margin: 0;
    flex: 1;
    font-size: 12px;
    color: var(--text);
    line-height: 1.5;
  }
  .export-card.csv-card {
    grid-column: 1 / -1;
  }
  .export-options {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 8px;
  }
  .export-options label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
    color: var(--tertiary);
  }
  .export-options select {
    background: var(--input-bg);
    border: none;
    color: var(--secondary);
    padding: 6px 8px;
    border-radius: var(--border-radius);
    font-size: 13px;
  }

  .btn-secondary {
    align-self: flex-start;
    padding: 7px 14px;
    background: transparent;
    border: none;
    border-radius: var(--border-radius);
    color: var(--text);
    font-family: inherit;
    font-size: 12px;
    cursor: pointer;
    transition: background 120ms ease, border-color 120ms ease;
  }
  .btn-secondary:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border-color: var(--accent);
  }
  .btn-secondary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .toast {
    padding: 8px 12px;
    border-radius: var(--border-radius);
    background: color-mix(in oklab, var(--accent) 14%, var(--surface));
    color: var(--text);
    font-size: 12px;
    border: 1px solid color-mix(in oklab, var(--accent) 30%, transparent);
  }
  .toast.err {
    background: color-mix(in oklab, var(--error, var(--accent)) 14%, var(--surface));
    border-color: color-mix(in oklab, var(--error, var(--accent)) 30%, transparent);
  }

  @media (prefers-reduced-motion: reduce) {
    .btn-primary,
    .btn-secondary,
    .back-link {
      transition: none;
    }
  }
</style>
