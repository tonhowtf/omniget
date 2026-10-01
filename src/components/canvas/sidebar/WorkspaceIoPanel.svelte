<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";

  interface PreviewNode {
    floorId: string;
    id: string;
    kind: string;
    title?: string | null;
  }
  interface ImportPreview {
    name: string;
    exportedAt: string;
    workingDirectory?: string | null;
    floors: Array<{ id: string; name: string; nodes: number }>;
    nodes: PreviewNode[];
    cords: number;
    groups: number;
    notes: number;
    chats: number;
    roles: string[];
    routines: number;
    omitted: Array<{ path: string; reason: string }>;
    conflicts: string[];
  }
  interface BackupInfo {
    id: string;
    createdAt: string;
    reason: string;
    sizeBytes: number;
    summary: { floors: number; nodes: number; notes: number; cords: number };
  }
  interface RestoreDiff {
    lines: string[];
    identical: boolean;
  }

  type Mode = { kind: "import"; path: string } | { kind: "backups"; workspaceId: string; name: string };

  let {
    mode,
    onclose,
    onimported,
    onrestored,
  }: {
    mode: Mode;
    onclose: () => void;
    onimported?: (workspaceId: string) => void;
    onrestored?: (workspaceId: string) => void;
  } = $props();

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) });

  let busy = $state(false);
  let error = $state<string | null>(null);
  let preview = $state<ImportPreview | null>(null);
  let importName = $state("");
  let importDir = $state("");
  let backups = $state<BackupInfo[]>([]);
  let chosen = $state<string | null>(null);
  let diff = $state<RestoreDiff | null>(null);

  $effect(() => {
    const m = mode;
    error = null;
    if (m.kind === "import") void loadPreview(m.path);
    else void loadBackups(m.workspaceId);
  });

  async function loadPreview(path: string) {
    busy = true;
    try {
      preview = await invoke<ImportPreview>("canvas_workspace_import_preview", { path });
      importName = preview.name;
      importDir = preview.workingDirectory ?? "";
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function doImport() {
    if (mode.kind !== "import") return;
    busy = true;
    try {
      const r = await invoke<{ workspaceId: string }>("canvas_workspace_import", {
        path: mode.path,
        name: importName.trim() || null,
        dir: importDir.trim() || null,
      });
      onimported?.(r.workspaceId);
      onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function chooseDir() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ directory: true, multiple: false, defaultPath: importDir || undefined });
      if (typeof picked === "string") importDir = picked;
    } catch {
    }
  }

  async function loadBackups(workspaceId: string) {
    busy = true;
    try {
      backups = await invoke<BackupInfo[]>("canvas_workspace_backups", { workspaceId });
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function backupNow() {
    if (mode.kind !== "backups") return;
    busy = true;
    try {
      await invoke("canvas_workspace_backup_now", { workspaceId: mode.workspaceId });
      backups = await invoke<BackupInfo[]>("canvas_workspace_backups", { workspaceId: mode.workspaceId });
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function pick(id: string) {
    if (mode.kind !== "backups") return;
    chosen = id;
    diff = null;
    try {
      diff = await invoke<RestoreDiff>("canvas_workspace_backup_diff", { workspaceId: mode.workspaceId, backupId: id });
    } catch (e) {
      error = String(e);
    }
  }

  async function restore() {
    if (mode.kind !== "backups" || !chosen) return;
    busy = true;
    try {
      await invoke("canvas_workspace_backup_restore", { workspaceId: mode.workspaceId, backupId: chosen });
      onrestored?.(mode.workspaceId);
      onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function when(iso: string): string {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
  }

  function reasonLabel(r: string): string {
    if (r === "hourly") return tr("canvas.workspaceIo.reasonHourly", "Automatic");
    if (r === "manual") return tr("canvas.workspaceIo.reasonManual", "Manual");
    if (r.startsWith("before-")) return tr("canvas.workspaceIo.reasonBefore", "Before {{what}}", { what: r.slice(7).replace(/-/g, " ") });
    return r;
  }

  function size(n: number): string {
    return n > 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="overlay" data-canvas-chrome onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="panel" role="dialog" aria-modal="true">
    {#if mode.kind === "import"}
      <h3>{tr("canvas.workspaceIo.importTitle", "Import workspace")}</h3>
      {#if preview}
        <p class="muted">{tr("canvas.workspaceIo.exportedAt", "Exported {{when}}", { when: when(preview.exportedAt) })}</p>
        <label>
          <span>{tr("canvas.workspaceIo.name", "Name")}</span>
          <input bind:value={importName} />
        </label>
        <label>
          <span>{tr("canvas.workspaceIo.folder", "Folder")}</span>
          <div class="row">
            <input bind:value={importDir} placeholder={tr("canvas.workspaceIo.noFolder", "No folder")} />
            <button type="button" onclick={chooseDir}>{tr("canvas.workspaceIo.choose", "Choose…")}</button>
          </div>
        </label>
        <p>
          {tr("canvas.workspaceIo.contents", "{{nodes}} items on {{floors}} floors, {{cords}} cords, {{notes}} notes, {{chats}} chats, {{routines}} routines", {
            nodes: preview.nodes.length,
            floors: preview.floors.length,
            cords: preview.cords,
            notes: preview.notes,
            chats: preview.chats,
            routines: preview.routines,
          })}
        </p>
        <ul class="nodes">
          {#each preview.nodes.slice(0, 40) as n (n.id)}
            <li><span class="kind">{n.kind}</span> {n.title ?? ""}</li>
          {/each}
          {#if preview.nodes.length > 40}<li class="muted">+{preview.nodes.length - 40}</li>{/if}
        </ul>
        {#if preview.roles.length}
          <p class="muted">{tr("canvas.workspaceIo.roles", "Roles: {{list}}", { list: preview.roles.join(", ") })}</p>
        {/if}
        {#each preview.conflicts as c (c)}<p class="warn">{c}</p>{/each}
        {#if preview.omitted.length}
          <details>
            <summary>{tr("canvas.workspaceIo.leftOut", "Left out of the file ({{count}})", { count: preview.omitted.length })}</summary>
            <ul>{#each preview.omitted as o, i (i)}<li>{o.path}: {o.reason}</li>{/each}</ul>
          </details>
        {/if}
      {/if}
      <div class="actions">
        <button type="button" onclick={onclose}>{tr("canvas.workspaceIo.cancel", "Cancel")}</button>
        <button type="button" class="primary" disabled={busy || !preview} onclick={doImport}>
          {tr("canvas.workspaceIo.import", "Import")}
        </button>
      </div>
    {:else}
      <h3>{tr("canvas.workspaceIo.backupsTitle", "Backups of {{name}}", { name: mode.name })}</h3>
      <p class="muted">
        {tr("canvas.workspaceIo.backupsHint", "Taken every hour when something changed, and before deleting, landing, importing or restoring.")}
      </p>
      <ul class="backups">
        {#each backups as b (b.id)}
          <li>
            <button type="button" class:on={chosen === b.id} onclick={() => void pick(b.id)}>
              <span>{when(b.createdAt)}</span>
              <span class="muted">{reasonLabel(b.reason)} · {b.summary.nodes} · {size(b.sizeBytes)}</span>
            </button>
          </li>
        {:else}
          <li class="muted">{busy ? "…" : tr("canvas.workspaceIo.noBackups", "No backups yet")}</li>
        {/each}
      </ul>
      {#if diff}
        <div class="diff">
          <strong>{tr("canvas.workspaceIo.restoreWill", "Restoring this backup:")}</strong>
          <ul>{#each diff.lines as l (l)}<li>{l}</li>{/each}</ul>
        </div>
      {/if}
      <div class="actions">
        <button type="button" disabled={busy} onclick={backupNow}>{tr("canvas.workspaceIo.backupNow", "Back up now")}</button>
        <span class="grow"></span>
        <button type="button" onclick={onclose}>{tr("canvas.workspaceIo.close", "Close")}</button>
        <button type="button" class="primary" disabled={busy || !chosen || !diff || diff.identical} onclick={restore}>
          {tr("canvas.workspaceIo.restore", "Restore")}
        </button>
      </div>
    {/if}
    {#if error}<p class="error">{error}</p>{/if}
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgb(0 0 0 / 0.3);
  }
  .panel {
    width: min(460px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
    overflow: auto;
    padding: 16px;
    border-radius: 12px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.35);
    color: var(--text);
    font-size: 13px;
    user-select: text;
  }
  h3 {
    margin: 0 0 8px;
    font-size: 15px;
  }
  p {
    margin: 6px 0;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 8px 0;
  }
  label > span {
    font-size: 11px;
    color: var(--text-muted);
  }
  input {
    flex: 1;
    padding: 6px 8px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
    font: inherit;
  }
  .row {
    display: flex;
    gap: 6px;
  }
  button {
    padding: 6px 10px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent, #fff);
  }
  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 12px;
  }
  .grow {
    flex: 1;
  }
  .muted {
    color: var(--text-muted);
  }
  .warn {
    color: var(--warning, #d9a441);
  }
  .error {
    color: var(--danger, #d96a62);
  }
  ul {
    margin: 4px 0;
    padding-left: 18px;
  }
  .nodes {
    max-height: 160px;
    overflow: auto;
  }
  .kind {
    font-size: 11px;
    color: var(--text-muted);
  }
  .backups {
    list-style: none;
    padding: 0;
    max-height: 240px;
    overflow: auto;
  }
  .backups button {
    width: 100%;
    display: flex;
    justify-content: space-between;
    gap: 8px;
    border: none;
    text-align: left;
  }
  .backups button.on,
  .backups button:hover {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .diff {
    margin-top: 8px;
    padding: 8px;
    border-radius: 8px;
    border: 1px solid var(--border);
  }
</style>
