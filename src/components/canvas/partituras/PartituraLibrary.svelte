<script lang="ts">
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { errorText } from "$lib/canvas/floors/api";
  import {
    PARTITURA_DRAG_TYPE,
    addToLibrary,
    deletePartitura,
    editPartitura,
    exportPartituraFile,
    listPartituras,
    readPartituraFile,
    savePartitura,
    type ImportResult,
    type LibraryPartitura,
    type PartituraSummary,
    type Permissions,
  } from "$lib/canvas/partituras/api";
  import ImportPreview from "./ImportPreview.svelte";

  interface Props {
    workspaceId: string;
    floorId: string;
    selection: { nodeIds: string[]; drawingIds?: string[] };
    center: { x: number; y: number };
    onClose: () => void;
    onImported: (r: ImportResult) => void;
  }
  let { workspaceId, floorId, selection, center, onClose, onImported }: Props = $props();

  let items = $state<PartituraSummary[]>([]);
  let query = $state("");
  let scope = $state<"all" | "global" | "workspace">("all");
  let error = $state("");
  let reviewing = $state<{ source: { id?: string; partitura?: LibraryPartitura }; at: { x: number; y: number } } | null>(null);

  let saving = $state(false);
  let name = $state("");
  let summary = $state("");
  let saveScope = $state<"workspace" | "global">("workspace");
  let includeCommands = $state(true);
  let blankNotes = $state(false);
  let includeChat = $state(false);
  let updateExisting = $state("");
  let setupCmd = $state("");
  let runCmd = $state("");
  let teardownCmd = $state("");
  let maxUsd = $state<number | null>(null);
  let permMode = $state<Permissions["mode"] | "">("");

  async function refresh() {
    try {
      const sc = scope === "workspace" ? workspaceId : scope;
      let list = await listPartituras(sc, query);
      if (scope === "workspace") list = list.filter((p) => p.scopeWorkspaceId === workspaceId);
      items = list;
    } catch (e) {
      error = errorText(e);
    }
  }

  $effect(() => {
    void query;
    void scope;
    void workspaceId;
    refresh();
  });

  async function saveSelection() {
    if (!name.trim() && !updateExisting) return;
    error = "";
    try {
      const setup = setupCmd || runCmd || teardownCmd ? { setup: setupCmd || undefined, run: runCmd || undefined, teardown: teardownCmd || undefined } : undefined;
      await savePartitura(workspaceId, floorId, {
        selection,
        options: { name: name.trim() || "Partitura", summary: summary.trim() || undefined, includeStartupCommands: includeCommands, blankNotes },
        scope: saveScope === "global" ? "global" : workspaceId,
        updateExisting: updateExisting || undefined,
        includeChat,
        setup,
        budget: maxUsd != null && maxUsd > 0 ? { maxUsd } : undefined,
        permissions: permMode ? { mode: permMode } : undefined,
      });
      saving = false;
      name = summary = setupCmd = runCmd = teardownCmd = updateExisting = "";
      maxUsd = null;
      permMode = "";
      refresh();
    } catch (e) {
      error = errorText(e);
    }
  }

  const updating = $derived(items.find((p) => p.id === updateExisting));
  const lockedCommands = $derived(!!updating?.privacy?.noStartupCommands);
  const lockedNotes = $derived(!!updating?.privacy?.blankNotes);
  $effect(() => {
    if (lockedCommands) includeCommands = false;
    if (lockedNotes) blankNotes = true;
  });

  async function undoUpdate(p: PartituraSummary) {
    error = "";
    try {
      await editPartitura(p.id, { restore: "previous" });
      refresh();
    } catch (e) {
      error = errorText(e);
    }
  }

  async function remove(p: PartituraSummary) {
    const msg = $t("canvas.partituras.confirmDelete", { default: "Delete '{{name}}' from the library? Canvases that use it are not touched.", name: p.name }) as string;
    if (!(await confirm(msg))) return;
    try {
      await deletePartitura(p.id);
      refresh();
    } catch (e) {
      error = errorText(e);
    }
  }

  async function exportOne(ids: string[], suggested: string) {
    const path = await saveDialog({ defaultPath: `${suggested}.json`, filters: [{ name: "Partitura", extensions: ["json"] }] });
    if (!path) return;
    try {
      await exportPartituraFile(ids, path);
    } catch (e) {
      error = errorText(e);
    }
  }

  async function importFiles() {
    const picked = await openDialog({ multiple: true, filters: [{ name: "Partitura", extensions: ["json"] }] });
    const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
    if (!paths.length) return;
    error = "";
    try {
      const all: LibraryPartitura[] = [];
      for (const p of paths) all.push(...(await readPartituraFile(p)));
      if (all.length === 1) {
        await addToLibrary(all);
        reviewing = { source: { partitura: all[0] }, at: center };
      } else {
        const msg = $t("canvas.partituras.confirmBulk", { default: "Add {{count}} partituras to your library? Commands are reviewed when you add one to a canvas.", count: all.length }) as string;
        if (!(await confirm(msg))) return;
        const r = await addToLibrary(all);
        if (r.skipped.length) error = $t("canvas.partituras.skipped", { default: "{{count}} already in the library", count: r.skipped.length }) as string;
      }
      refresh();
    } catch (e) {
      error = errorText(e);
    }
  }

  function dragStart(e: DragEvent, p: PartituraSummary) {
    e.dataTransfer?.setData(PARTITURA_DRAG_TYPE, p.id);
    e.dataTransfer?.setData("text/plain", p.name);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "copy";
  }

  function viewBox(p: PartituraSummary): string {
    const w = Math.max(p.bounds.w, 1);
    const h = Math.max(p.bounds.h, 1);
    const pad = Math.max(w, h) * 0.04;
    return `${-pad} ${-pad} ${w + pad * 2} ${h + pad * 2}`;
  }

  function center2(p: PartituraSummary, id: string): [number, number] {
    const s = p.shapes.find((x) => x.id === id);
    return s ? [s.x + s.w / 2, s.y + s.h / 2] : [0, 0];
  }

  const KIND_FILL: Record<string, string> = {
    terminal: "var(--text)",
    agent: "var(--accent)",
    note: "#f5a524",
    portal: "#3e8ef7",
  };
</script>

<div class="lib" data-canvas-chrome role="dialog" aria-label={$t("canvas.partituras.title", { default: "Partituras" })}>
  <header>
    <h3>{$t("canvas.partituras.title", { default: "Partituras" })}</h3>
    <div class="actions">
      <button type="button" onclick={importFiles}>{$t("canvas.partituras.import", { default: "Import…" })}</button>
      {#if items.length > 1}
        <button type="button" onclick={() => exportOne(items.map((i) => i.id), "partituras")}>{$t("canvas.partituras.exportAll", { default: "Export all…" })}</button>
      {/if}
      <button type="button" class="icon" onclick={onClose} aria-label={$t("canvas.partituras.close", { default: "Close" })}>✕</button>
    </div>
  </header>
  <input class="search" bind:value={query} placeholder={$t("canvas.partituras.search", { default: "Search by name, role or workspace" })} />
  <div class="seg" role="radiogroup">
    {#each [["all", "canvas.partituras.scopeAll", "All"], ["global", "canvas.partituras.scopeGlobal", "Global"], ["workspace", "canvas.partituras.scopeWorkspace", "This workspace"]] as [v, key, label] (v)}
      <button type="button" class:on={scope === v} onclick={() => (scope = v as typeof scope)}>{$t(key, { default: label })}</button>
    {/each}
  </div>

  {#if saving}
    <div class="form">
      {#if items.length}
        <label>
          <span>{$t("canvas.partituras.updateExisting", { default: "Update existing (keeps its name and identity)" })}</span>
          <select bind:value={updateExisting}>
            <option value="">{$t("canvas.partituras.createNew", { default: "Create a new one" })}</option>
            {#each items as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
          </select>
        </label>
      {/if}
      {#if !updateExisting}
        <label><span>{$t("canvas.partituras.name", { default: "Name" })}</span><input bind:value={name} /></label>
        <label><span>{$t("canvas.partituras.summary", { default: "Description" })}</span><input bind:value={summary} /></label>
        <label>
          <span>{$t("canvas.partituras.scope", { default: "Scope" })}</span>
          <select bind:value={saveScope}>
            <option value="workspace">{$t("canvas.partituras.scopeWorkspace", { default: "This workspace" })}</option>
            <option value="global">{$t("canvas.partituras.scopeGlobal", { default: "Global" })}</option>
          </select>
        </label>
      {/if}
      <label class="check"><input type="checkbox" bind:checked={includeCommands} disabled={lockedCommands} />{$t("canvas.partituras.includeCommands", { default: "Include startup commands" })}</label>
      <label class="check"><input type="checkbox" bind:checked={blankNotes} disabled={lockedNotes} />{$t("canvas.partituras.blankNotes", { default: "Send notes blank" })}</label>
      <label class="check"><input type="checkbox" bind:checked={includeChat} />{$t("canvas.partituras.includeChat", { default: "Include chat history" })}</label>
      {#if lockedCommands || lockedNotes}
        <p class="muted">{$t("canvas.partituras.privacyKept", { default: "Kept from when it was saved: what was left out stays out." })}</p>
      {/if}
      <details>
        <summary>{$t("canvas.partituras.extras", { default: "Hooks, budget and permissions" })}</summary>
        <label><span>{$t("canvas.floors.hooks.setup", { default: "Setup" })}</span><input class="mono" bind:value={setupCmd} placeholder="pnpm install" /></label>
        <label><span>{$t("canvas.floors.hooks.run", { default: "Run" })}</span><input class="mono" bind:value={runCmd} placeholder="pnpm dev --port $PORT" /></label>
        <label><span>{$t("canvas.floors.hooks.teardown", { default: "Teardown" })}</span><input class="mono" bind:value={teardownCmd} /></label>
        <label><span>{$t("canvas.partituras.maxUsd", { default: "Max spend" })} (US$)</span><input type="number" min="0" step="0.5" bind:value={maxUsd} /></label>
        <label>
          <span>{$t("canvas.partituras.permMode", { default: "Permission mode" })}</span>
          <select bind:value={permMode}>
            <option value="">—</option>
            <option value="supervised">supervised</option>
            <option value="acceptEdits">acceptEdits</option>
            <option value="auto">auto</option>
            <option value="full">full</option>
          </select>
        </label>
      </details>
      <div class="row">
        <button type="button" onclick={() => (saving = false)}>{$t("canvas.partituras.cancel", { default: "Cancel" })}</button>
        <button type="button" class="primary" disabled={!selection.nodeIds.length || (!name.trim() && !updateExisting)} onclick={saveSelection}>
          {$t("canvas.partituras.save", { default: "Save" })}
        </button>
      </div>
    </div>
  {:else}
    <button type="button" class="primary wide" disabled={!selection.nodeIds.length} onclick={() => (saving = true)}>
      {$t("canvas.partituras.fromSelection", { default: "New partitura from selection" })}
    </button>
  {/if}
  {#if error}<p class="error">{error}</p>{/if}

  <div class="grid">
    {#each items as p (p.id)}
      <div
        class="card"
        role="button"
        tabindex="0"
        draggable="true"
        style:--pc={p.color ?? "var(--accent)"}
        ondragstart={(e) => dragStart(e, p)}
        ondblclick={() => (reviewing = { source: { id: p.id }, at: center })}
        onkeydown={(e) => e.key === "Enter" && (reviewing = { source: { id: p.id }, at: center })}
        title={p.summary ?? p.name}
      >
        <svg viewBox={viewBox(p)} preserveAspectRatio="xMidYMid meet" aria-hidden="true">
          {#each p.cords as [a, b], i (i)}
            {@const pa = center2(p, a)}
            {@const pb = center2(p, b)}
            <line x1={pa[0]} y1={pa[1]} x2={pb[0]} y2={pb[1]} stroke="var(--text-muted)" stroke-width={Math.max(p.bounds.w, p.bounds.h) / 150} />
          {/each}
          {#each p.shapes as s (s.id)}
            <rect x={s.x} y={s.y} width={s.w} height={s.h} rx={Math.min(s.w, s.h) / 12} fill={s.color ?? KIND_FILL[s.kind] ?? "var(--text-muted)"} fill-opacity="0.35" />
          {/each}
        </svg>
        <div class="meta">
          <strong>{p.name}</strong>
          <span class="muted">
            {p.terminalCount} {$t("canvas.partituras.terminals", { default: "terminals" })}
            {#if p.workspaceName}· {p.workspaceName}{:else}· {$t("canvas.partituras.scopeGlobal", { default: "Global" })}{/if}
          </span>
          {#if p.roleNames.length}
            <div class="chips">{#each p.roleNames as r (r)}<span class="chip">{r}</span>{/each}</div>
          {/if}
        </div>
        <div class="cardActions">
          <button type="button" onclick={() => (reviewing = { source: { id: p.id }, at: center })}>{$t("canvas.partituras.addToCanvas", { default: "Add to canvas" })}</button>
          {#if p.versions}
            <button type="button" class="icon" title={$t("canvas.partituras.undoUpdate", { default: "Undo the last update" })} onclick={() => undoUpdate(p)}>↶</button>
          {/if}
          <button type="button" class="icon" title={$t("canvas.partituras.saveTo", { default: "Save to…" })} onclick={() => exportOne([p.id], p.name)}>⤓</button>
          <button type="button" class="icon" title={$t("canvas.partituras.delete", { default: "Delete…" })} onclick={() => remove(p)}>✕</button>
        </div>
      </div>
    {:else}
      <p class="muted">{$t("canvas.partituras.empty", { default: "No partituras yet. Select nodes on the canvas and save them as one." })}</p>
    {/each}
  </div>
</div>

{#if reviewing}
  <ImportPreview
    {workspaceId}
    {floorId}
    source={reviewing.source}
    at={reviewing.at}
    onClose={() => (reviewing = null)}
    onImported={(r) => {
      reviewing = null;
      onImported(r);
    }}
  />
{/if}

<style>
  .lib {
    width: min(420px, calc(100vw - 32px));
    max-height: 75vh;
    overflow: auto;
    background: var(--popup-bg, var(--surface));
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: 0 12px 36px rgba(0, 0, 0, 0.3);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 13px;
  }
  header,
  .actions,
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    justify-content: space-between;
  }
  h3 {
    margin: 0;
    font-size: 15px;
  }
  .search,
  input:not([type="checkbox"]),
  select {
    font: inherit;
    color: var(--text);
    background: var(--input-bg, transparent);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 5px 8px;
    min-width: 0;
  }
  .seg {
    display: flex;
    gap: 4px;
  }
  .seg .on {
    border-color: var(--accent);
    color: var(--accent);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  label > span {
    color: var(--text-muted);
    font-size: 12px;
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  details {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: 8px;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border: 1px solid var(--border);
    border-top: 3px solid var(--pc);
    border-radius: 10px;
    cursor: grab;
    outline: none;
  }
  .card:focus-visible {
    box-shadow: 0 0 0 2px var(--accent);
  }
  svg {
    width: 100%;
    height: 80px;
    background: var(--input-bg, rgba(0, 0, 0, 0.12));
    border-radius: 6px;
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .meta strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 3px;
  }
  .chip {
    font-size: 11px;
    padding: 1px 6px;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--text-muted);
  }
  .cardActions {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .cardActions button:first-child {
    flex: 1;
  }
  .muted {
    color: var(--text-muted);
    font-size: 12px;
    margin: 0;
  }
  .error {
    color: var(--danger);
    margin: 0;
  }
  .mono {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
  }
  button {
    font: inherit;
    color: inherit;
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 4px 10px;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .icon {
    border: none;
    padding: 2px 6px;
  }
  .primary {
    background: var(--accent);
    color: var(--on-accent, #fff);
    border-color: var(--accent);
  }
  .wide {
    width: 100%;
  }
</style>
