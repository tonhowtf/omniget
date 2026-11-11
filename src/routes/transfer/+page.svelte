<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { downloadDir } from "@tauri-apps/api/path";
  import { t } from "$lib/i18n";
  import * as api from "$lib/transfer/api";
  import {
    childPath,
    crumbs,
    draftOf,
    emptyForm,
    filterEntries,
    formatBytes,
    formOf,
    formReady,
    parentPath,
    progressPercent,
    secretOf,
    serverAddress,
    validName,
    type ServerForm,
  } from "$lib/transfer/helpers";
  import type { RemoteEntry, RemoteListing, TransferServer, TransferSnapshot } from "$lib/transfer/types";
  import type { DevtoolsError } from "$lib/devtools/types";
  import DevShell from "$components/devtools/DevShell.svelte";
  import ErrorNotice from "$components/devtools/ErrorNotice.svelte";

  type Job = {
    key: number;
    direction: "download" | "upload";
    status: TransferSnapshot["status"];
    bytes: number;
    total: number | null;
    remote: string;
    local: string;
    error: DevtoolsError | null;
    replace: (() => void) | null;
  };

  const MAX_JOBS_SHOWN = 20;
  const FILTER_FROM = 8;

  let servers = $state<TransferServer[] | null>(null);
  let selected = $state<string | null>(null);
  let form = $state<ServerForm | null>(null);
  let editing = $state(false);
  let saving = $state(false);
  let formError = $state<DevtoolsError | null>(null);
  let confirmRemove = $state(false);

  let listing = $state<RemoteListing | null>(null);
  let busy = $state(false);
  let error = $state<DevtoolsError | null>(null);
  let query = $state("");
  let saveDir = $state("");
  let newFolder = $state<string | null>(null);
  let renaming = $state<{ name: string; to: string } | null>(null);
  let pendingDelete = $state<RemoteEntry | null>(null);
  let deleting = $state(false);
  let jobs = $state<Job[]>([]);

  let alive = true;
  let loadSerial = 0;
  let jobSerial = 0;

  let current = $derived(servers?.find((server) => server.name === selected) ?? null);
  let shown = $derived(filterEntries(listing?.entries ?? [], query));
  let trail = $derived(listing ? crumbs(listing.path) : []);

  async function loadServers() {
    try {
      servers = await api.serversList();
    } catch (e) {
      servers = [];
      error = api.parseError(e);
    }
  }

  async function browse(path: string | null) {
    if (!selected) return;
    const serial = ++loadSerial;
    busy = true;
    error = null;
    newFolder = null;
    renaming = null;
    try {
      const result = await api.list({ server: selected }, path);
      if (serial !== loadSerial) return;
      listing = result;
      query = "";
    } catch (e) {
      if (serial !== loadSerial) return;
      error = api.parseError(e);
    } finally {
      if (serial === loadSerial) busy = false;
    }
  }

  function choose(name: string) {
    selected = name;
    form = null;
    listing = null;
    void browse(null);
  }

  function startAdd() {
    form = emptyForm();
    editing = false;
    formError = null;
    confirmRemove = false;
  }

  function startEdit(server: TransferServer) {
    form = formOf(server);
    editing = true;
    formError = null;
    confirmRemove = false;
  }

  async function saveServer() {
    if (!form || saving || !formReady(form)) return;
    saving = true;
    formError = null;
    try {
      const saved = await api.serverSave(draftOf(form), secretOf(form, editing));
      await loadServers();
      choose(saved.name);
    } catch (e) {
      formError = api.parseError(e);
    } finally {
      saving = false;
    }
  }

  async function removeServer() {
    if (!form || saving) return;
    const name = form.name;
    saving = true;
    try {
      await api.serverRemove(name);
      if (selected === name) {
        selected = null;
        listing = null;
        error = null;
      }
      form = null;
      await loadServers();
    } catch (e) {
      formError = api.parseError(e);
    } finally {
      saving = false;
      confirmRemove = false;
    }
  }

  async function browseKey() {
    const picked = await openDialog({ multiple: false, directory: false });
    if (typeof picked === "string" && form) form.keyPath = picked;
  }

  async function pickSaveDir(): Promise<string> {
    const picked = await openDialog({ multiple: false, directory: true });
    if (typeof picked === "string") saveDir = picked;
    return saveDir;
  }

  function dismiss(key: number) {
    jobs = jobs.filter((job) => job.key !== key);
  }

  async function run(direction: Job["direction"], remote: string, local: string, start: (overwrite: boolean) => Promise<TransferSnapshot>, overwrite = false) {
    const key = ++jobSerial;
    const server = selected;
    jobs = [{ key, direction, status: "running" as const, bytes: 0, total: null, remote, local, error: null, replace: null }, ...jobs].slice(0, MAX_JOBS_SHOWN);
    const patch = (change: Partial<Job>) => {
      jobs = jobs.map((job) => (job.key === key ? { ...job, ...change } : job));
    };
    try {
      let snapshot = await start(overwrite);
      while (alive && snapshot.status === "running") {
        patch({ bytes: snapshot.bytes, total: snapshot.total });
        snapshot = await api.status(snapshot.transferId, 2);
      }
      patch({ status: snapshot.status, bytes: snapshot.bytes, total: snapshot.total, remote: snapshot.remote, local: snapshot.local });
      if (direction === "upload" && listing && selected === server) void browse(listing.path);
    } catch (e) {
      const failure = api.parseError(e);
      const replace =
        failure.code === "ALREADY_EXISTS" && !overwrite
          ? () => {
              dismiss(key);
              void run(direction, remote, local, start, true);
            }
          : null;
      patch({ status: "failed", error: failure, replace });
    }
  }

  async function download(entry: RemoteEntry) {
    if (!selected || !listing) return;
    const folder = saveDir || (await pickSaveDir());
    if (!folder) return;
    const target = { server: selected };
    const remote = childPath(listing.path, entry.name);
    void run("download", remote, folder, (overwrite) => api.get(target, remote, folder, overwrite));
  }

  async function upload() {
    if (!selected || !listing) return;
    const picked = await openDialog({ multiple: true, directory: false });
    const files = Array.isArray(picked) ? picked : picked ? [picked] : [];
    const target = { server: selected };
    const folder = `${listing.path === "/" ? "" : listing.path}/`;
    for (const file of files) {
      await run("upload", folder, file, (overwrite) => api.put(target, file, folder, overwrite));
    }
  }

  async function manage(work: () => Promise<unknown>) {
    if (!listing) return;
    const folder = listing.path;
    busy = true;
    error = null;
    try {
      await work();
      await browse(folder);
    } catch (e) {
      error = api.parseError(e);
      busy = false;
    }
  }

  function createFolder() {
    if (!selected || !listing || newFolder === null || !validName(newFolder)) return;
    const target = { server: selected };
    const path = childPath(listing.path, newFolder.trim());
    void manage(() => api.mkdir(target, path));
  }

  function applyRename() {
    if (!selected || !listing || !renaming || !validName(renaming.to) || renaming.to.trim() === renaming.name) return;
    const target = { server: selected };
    const path = childPath(listing.path, renaming.name);
    const to = renaming.to.trim();
    void manage(() => api.rename(target, path, to));
  }

  async function askDelete(entry: RemoteEntry) {
    if (!selected || !listing) return;
    error = null;
    try {
      await api.remove({ server: selected }, childPath(listing.path, entry.name), false);
      pendingDelete = entry;
    } catch (e) {
      error = api.parseError(e);
    }
  }

  async function confirmDelete() {
    if (!selected || !listing || !pendingDelete || deleting) return;
    const target = { server: selected };
    const path = childPath(listing.path, pendingDelete.name);
    deleting = true;
    await manage(() => api.remove(target, path, true));
    deleting = false;
    pendingDelete = null;
  }

  async function trustNewKey() {
    if (!current) return;
    try {
      await api.hostForget(current.host, current.port);
      await browse(listing?.path ?? null);
    } catch (e) {
      error = api.parseError(e);
    }
  }

  function when(modified: string | null): string {
    if (!modified) return "";
    const date = new Date(modified);
    return Number.isNaN(date.getTime()) ? "" : date.toLocaleString();
  }

  function keydown(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    if (pendingDelete && !deleting) pendingDelete = null;
    else if (renaming) renaming = null;
    else if (newFolder !== null) newFolder = null;
  }

  onMount(() => {
    void loadServers();
    void downloadDir()
      .then((dir) => {
        if (!saveDir) saveDir = dir;
      })
      .catch(() => {});
  });

  onDestroy(() => {
    alive = false;
  });
</script>

<svelte:window onkeydown={keydown} />

<DevShell icon="transfer" title={$t("transfer.title") as string} subtitle={$t("transfer.subtitle") as string} {busy}>
  <div class="tr-grid">
    <aside class="dv-panel tr-servers" aria-label={$t("transfer.servers") as string}>
      <h2 class="dv-title">{$t("transfer.servers")}</h2>
      {#if servers === null}
        <p class="dv-meta">{$t("common.loading")}</p>
      {:else if servers.length === 0}
        <p class="dv-meta">{$t("transfer.no_servers")}</p>
      {:else}
        <ul>
          {#each servers as server (server.name)}
            <li>
              <button type="button" class="tr-server" class:active={selected === server.name} aria-current={selected === server.name ? "true" : undefined} onclick={() => choose(server.name)}>
                <b>{server.name}</b>
                <span class="dv-mono">{serverAddress(server)}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
      <button type="button" class="btn btn-secondary btn-sm" onclick={startAdd}>{$t("transfer.add_server")}</button>
    </aside>

    <div class="dv-stack">
      {#if form}
        <form
          class="dv-panel"
          onsubmit={(e) => {
            e.preventDefault();
            void saveServer();
          }}
        >
          <h2 class="dv-title">{editing ? $t("transfer.edit_server", { name: form.name }) : $t("transfer.new_server")}</h2>
          <div class="dv-row">
            <label class="dv-field wide">
              {$t("transfer.name")}
              <input class="input" bind:value={form.name} disabled={editing} maxlength="64" placeholder={$t("transfer.name_placeholder") as string} autocomplete="off" />
            </label>
            <label class="dv-field">
              {$t("transfer.protocol")}
              <select class="input" bind:value={form.protocol}>
                <option value="ftp">{$t("transfer.protocol_ftp")}</option>
                <option value="ftps">{$t("transfer.protocol_ftps")}</option>
                <option value="sftp">{$t("transfer.protocol_sftp")}</option>
              </select>
            </label>
          </div>
          <div class="dv-row">
            <label class="dv-field wide">
              {$t("transfer.host")}
              <input class="input dv-mono" bind:value={form.host} placeholder="192.168.0.42" spellcheck="false" autocomplete="off" autocapitalize="off" />
            </label>
            <label class="dv-field fixed">
              {$t("transfer.port")}
              <input class="input dv-mono" bind:value={form.port} inputmode="numeric" placeholder={form.protocol === "sftp" ? "22" : "21"} autocomplete="off" />
            </label>
          </div>
          <div class="dv-row">
            <label class="dv-field wide">
              {$t("transfer.user")}
              <input class="input dv-mono" bind:value={form.user} placeholder={form.protocol === "sftp" ? "" : ($t("transfer.user_anonymous") as string)} spellcheck="false" autocomplete="off" autocapitalize="off" />
            </label>
            <label class="dv-field wide">
              {form.protocol === "sftp" && form.keyPath.trim() ? $t("transfer.passphrase") : $t("transfer.password")}
              <input class="input" type="password" bind:value={form.secret} placeholder={editing ? ($t("transfer.secret_keep") as string) : ""} autocomplete="off" />
            </label>
          </div>
          {#if form.protocol === "sftp"}
            <div class="dv-row">
              <label class="dv-field wide">
                {$t("transfer.key_file")}
                <input class="input dv-mono" bind:value={form.keyPath} placeholder="~/.ssh/id_ed25519" spellcheck="false" autocomplete="off" />
              </label>
              <button type="button" class="btn btn-secondary" onclick={browseKey}>{$t("transfer.choose")}</button>
            </div>
          {/if}
          <label class="dv-field">
            {$t("transfer.start_path")}
            <input class="input dv-mono" bind:value={form.startPath} placeholder={$t("transfer.start_path_hint") as string} spellcheck="false" autocomplete="off" />
          </label>
          <p class="dv-meta">{$t("transfer.secret_note")}</p>
          {#if form.protocol === "ftp"}
            <p class="dv-warn">{$t("transfer.insecure_note")}</p>
          {/if}
          {#if formError}
            <ErrorNotice error={formError} />
          {/if}
          <div class="dv-bar">
            <div class="dv-bar-group">
              <button type="submit" class="btn btn-primary" disabled={saving || !formReady(form)}>
                {#if saving}<span class="spinner" aria-hidden="true"></span>{/if}
                {$t("transfer.save")}
              </button>
              <button type="button" class="btn btn-secondary" disabled={saving} onclick={() => (form = null)}>{$t("common.cancel")}</button>
            </div>
            {#if editing}
              <div class="dv-bar-group">
                {#if confirmRemove}
                  <span class="dv-meta">{$t("transfer.remove_server_confirm", { name: form.name })}</span>
                  <button type="button" class="btn btn-destructive btn-sm" disabled={saving} onclick={removeServer}>{$t("transfer.remove_server")}</button>
                  <button type="button" class="btn btn-secondary btn-sm" onclick={() => (confirmRemove = false)}>{$t("common.cancel")}</button>
                {:else}
                  <button type="button" class="btn btn-ghost btn-sm" onclick={() => (confirmRemove = true)}>{$t("transfer.remove_server")}</button>
                {/if}
              </div>
            {/if}
          </div>
        </form>
      {:else if !current}
        {#if error}
          <ErrorNotice {error} />
        {:else}
          <p class="dv-empty">{servers?.length ? $t("transfer.pick_server") : $t("transfer.no_servers")}</p>
        {/if}
      {:else}
        <div class="dv-panel" aria-busy={busy}>
          <div class="dv-bar">
            <nav class="tr-crumbs" aria-label={$t("transfer.path") as string}>
              {#if listing}
                {#each trail as crumb, i (crumb.path)}
                  {#if i > 1}<span class="tr-sep" aria-hidden="true">/</span>{/if}
                  <button type="button" class="tr-crumb dv-mono" aria-current={i === trail.length - 1 ? "page" : undefined} disabled={busy} onclick={() => browse(crumb.path)}>{crumb.label}</button>
                {/each}
              {:else}
                <span class="dv-meta">{busy ? $t("transfer.connecting", { name: current.name }) : serverAddress(current)}</span>
              {/if}
            </nav>
            <div class="dv-bar-group">
              {#if busy}<span class="spinner" aria-hidden="true"></span>{/if}
              <button type="button" class="btn btn-secondary btn-sm" disabled={busy || !listing || listing.path === "/"} onclick={() => listing && browse(parentPath(listing.path))}>{$t("transfer.up")}</button>
              <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => browse(listing?.path ?? null)}>{$t("transfer.refresh")}</button>
              <button type="button" class="btn btn-secondary btn-sm" disabled={busy || !listing} onclick={() => (newFolder = "")}>{$t("transfer.new_folder")}</button>
              <button type="button" class="btn btn-primary btn-sm" disabled={busy || !listing} onclick={upload}>{$t("transfer.upload")}</button>
              <button type="button" class="btn btn-ghost btn-sm" onclick={() => current && startEdit(current)}>{$t("transfer.edit")}</button>
            </div>
          </div>

          {#if newFolder !== null}
            <form
              class="dv-row"
              onsubmit={(e) => {
                e.preventDefault();
                createFolder();
              }}
            >
              <label class="dv-field wide">
                {$t("transfer.folder_name")}
                <!-- svelte-ignore a11y_autofocus -->
                <input class="input" bind:value={newFolder} autocomplete="off" autofocus />
              </label>
              <button type="submit" class="btn btn-primary" disabled={busy || !validName(newFolder)}>{$t("transfer.create")}</button>
              <button type="button" class="btn btn-secondary" onclick={() => (newFolder = null)}>{$t("common.cancel")}</button>
            </form>
          {/if}

          {#if error}
            <ErrorNotice {error}>
              {#if error.code === "HOST_KEY_CHANGED"}
                <p class="dv-meta">{$t("transfer.trust_new_key_hint")}</p>
                <button type="button" class="btn btn-destructive btn-sm" onclick={trustNewKey}>{$t("transfer.trust_new_key")}</button>
              {:else if error.code === "AUTH_FAILED"}
                <button type="button" class="btn btn-secondary btn-sm" onclick={() => current && startEdit(current)}>{$t("transfer.edit")}</button>
              {/if}
            </ErrorNotice>
          {/if}

          {#if listing}
            {#if listing.entries.length > FILTER_FROM}
              <label class="dv-field">
                {$t("transfer.filter")}
                <input class="input" type="search" bind:value={query} autocomplete="off" spellcheck="false" />
              </label>
            {/if}
            {#if listing.entries.length === 0}
              <p class="dv-empty">{$t("transfer.empty_folder")}</p>
            {:else if shown.length === 0}
              <p class="dv-empty">{$t("transfer.no_match", { query })}</p>
            {:else}
              <div class="dv-scroll">
                <table class="dv-table">
                  <thead>
                    <tr>
                      <th>{$t("transfer.col_name")}</th>
                      <th class="num">{$t("transfer.col_size")}</th>
                      <th class="tr-when">{$t("transfer.col_modified")}</th>
                      <th></th>
                    </tr>
                  </thead>
                  <tbody>
                    {#each shown as entry (entry.name)}
                      <tr>
                        <td class="tr-name">
                          {#if renaming?.name === entry.name}
                            <form
                              class="tr-rename"
                              onsubmit={(e) => {
                                e.preventDefault();
                                applyRename();
                              }}
                            >
                              <!-- svelte-ignore a11y_autofocus -->
                              <input class="input" aria-label={$t("transfer.new_name") as string} bind:value={renaming.to} autocomplete="off" autofocus />
                              <button type="submit" class="btn btn-primary btn-sm" disabled={busy || !validName(renaming.to) || renaming.to.trim() === entry.name}>{$t("transfer.save")}</button>
                              <button type="button" class="btn btn-secondary btn-sm" onclick={() => (renaming = null)}>{$t("common.cancel")}</button>
                            </form>
                          {:else if entry.kind === "dir"}
                            <button type="button" class="tr-open" disabled={busy} onclick={() => listing && browse(childPath(listing.path, entry.name))}>
                              <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor" aria-hidden="true"><path d="M1.5 3.5A1.5 1.5 0 0 1 3 2h3.2l1.6 1.8H13A1.5 1.5 0 0 1 14.5 5.3v6.2A1.5 1.5 0 0 1 13 13H3a1.5 1.5 0 0 1-1.5-1.5z" /></svg>
                              <span>{entry.name}</span>
                              <span class="sr-only">{$t("transfer.kind_dir")}</span>
                            </button>
                          {:else}
                            <span class="tr-file">
                              <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" aria-hidden="true"><path d="M4 1.8h5l3 3v9.4H4z M9 1.8v3h3" /></svg>
                              <span>{entry.name}</span>
                              {#if entry.kind !== "file"}<span class="tag tag-info">{entry.kind === "link" ? $t("transfer.kind_link") : $t("transfer.kind_other")}</span>{/if}
                            </span>
                          {/if}
                        </td>
                        <td class="num dim dv-mono">{formatBytes(entry.size)}</td>
                        <td class="dim tr-when">{when(entry.modified)}</td>
                        <td class="num tr-actions">
                          {#if entry.kind === "file"}
                            <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => download(entry)}>{$t("transfer.download")}</button>
                          {/if}
                          <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={() => (renaming = { name: entry.name, to: entry.name })}>{$t("transfer.rename")}</button>
                          <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={() => askDelete(entry)}>{$t("transfer.delete")}</button>
                        </td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            {/if}
            {#if listing.truncated}
              <p class="dv-warn">{$t("transfer.truncated", { shown: listing.entries.length, total: listing.total })}</p>
            {/if}
            {#if listing.skipped > 0}
              <p class="dv-warn">{$t("transfer.skipped", { count: listing.skipped })}</p>
            {/if}
          {/if}
          {#if current.protocol === "ftp"}
            <p class="dv-meta">{$t("transfer.insecure_note")}</p>
          {/if}
        </div>
      {/if}

      <div class="dv-panel">
        <div class="dv-bar">
          <h2 class="dv-title">{$t("transfer.transfers")}</h2>
          <div class="dv-bar-group tr-save">
            <span class="dv-meta">{$t("transfer.save_to")}</span>
            <span class="dv-mono tr-save-path" title={saveDir}>{saveDir}</span>
            <button type="button" class="btn btn-secondary btn-sm" onclick={pickSaveDir}>{$t("transfer.choose")}</button>
          </div>
        </div>
        {#if jobs.length === 0}
          <p class="dv-meta">{$t("transfer.transfers_empty")}</p>
        {:else}
          <ul class="tr-jobs">
            {#each jobs as job (job.key)}
              {@const percent = progressPercent(job)}
              <li>
                <div class="tr-job-top">
                  <span class="tag tag-{job.status === 'failed' ? 'danger' : job.status === 'done' ? 'success' : 'info'}">
                    {job.status === "failed" ? $t("transfer.failed") : job.status === "done" ? $t("transfer.done") : job.direction === "download" ? $t("transfer.downloading") : $t("transfer.uploading")}
                  </span>
                  <span class="dv-mono tr-job-name" title={job.direction === "download" ? job.remote : job.local}>{job.direction === "download" ? job.remote : job.local}</span>
                  <span class="dv-meta">
                    {job.total !== null ? $t("transfer.progress", { done: formatBytes(job.bytes), total: formatBytes(job.total) }) : $t("transfer.progress_unknown", { done: formatBytes(job.bytes) })}
                  </span>
                  {#if job.status === "done" && job.direction === "download"}
                    <button type="button" class="btn btn-secondary btn-sm" onclick={() => void api.reveal(job.local).catch(() => {})}>{$t("transfer.show_in_folder")}</button>
                  {/if}
                  {#if job.status !== "running"}
                    <button type="button" class="btn btn-ghost btn-sm tr-x" aria-label={$t("transfer.dismiss") as string} title={$t("transfer.dismiss") as string} onclick={() => dismiss(job.key)}>
                      <svg viewBox="0 0 12 12" width="10" height="10" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="m3 3 6 6M9 3 3 9" /></svg>
                    </button>
                  {/if}
                </div>
                {#if job.status === "running"}
                  {#if percent === null}
                    <progress class="tr-progress" aria-label={job.remote}></progress>
                  {:else}
                    <progress class="tr-progress" max="100" value={percent} aria-label={job.remote}></progress>
                  {/if}
                {/if}
                {#if job.error}
                  <ErrorNotice error={job.error}>
                    {#if job.replace}
                      <button type="button" class="btn btn-secondary btn-sm" onclick={job.replace}>{$t("transfer.replace")}</button>
                    {/if}
                  </ErrorNotice>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    </div>
  </div>
</DevShell>

{#if pendingDelete}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="dialog-backdrop" onclick={(e) => e.target === e.currentTarget && !deleting && (pendingDelete = null)}>
    <div class="dialog-card" role="alertdialog" aria-modal="true" aria-labelledby="tr-delete-title" aria-describedby="tr-delete-body">
      <h2 class="dialog-title" id="tr-delete-title">{$t("transfer.delete_title", { name: pendingDelete.name })}</h2>
      <div class="dialog-body" id="tr-delete-body">
        <p>{pendingDelete.kind === "dir" ? $t("transfer.delete_dir_body") : $t("transfer.delete_file_body")}</p>
      </div>
      <div class="dialog-actions">
        <!-- svelte-ignore a11y_autofocus -->
        <button type="button" class="btn btn-secondary" disabled={deleting} onclick={() => (pendingDelete = null)} autofocus>{$t("common.cancel")}</button>
        <button type="button" class="btn btn-destructive" disabled={deleting} onclick={confirmDelete}>
          {#if deleting}<span class="spinner" aria-hidden="true"></span>{/if}
          {$t("transfer.delete")}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .tr-grid {
    display: grid;
    grid-template-columns: 240px minmax(0, 1fr);
    align-items: start;
    gap: var(--space-4);
  }

  .tr-servers ul,
  .tr-jobs {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .tr-jobs {
    gap: var(--space-3);
  }

  .tr-server {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: var(--space-2);
    border: 0;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }

  .tr-server span {
    overflow: hidden;
    color: var(--text-dim);
    font-size: var(--text-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (hover: hover) {
    .tr-server:hover:not(.active) {
      background: var(--fill-1);
    }
  }

  .tr-server:active:not(.active) {
    background: var(--fill-2);
  }

  .tr-server.active {
    background: var(--accent-soft);
  }

  .tr-server:focus-visible,
  .tr-crumb:focus-visible,
  .tr-open:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .tr-crumbs {
    display: flex;
    flex: 1 1 200px;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
    min-width: 0;
  }

  .tr-sep {
    color: var(--text-dim);
  }

  .tr-crumb,
  .tr-open {
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }

  .tr-crumb {
    padding: 2px var(--space-1);
    font-size: var(--text-sm);
  }

  .tr-crumb[aria-current="page"] {
    font-weight: 700;
  }

  @media (hover: hover) {
    .tr-crumb:hover:not(:disabled),
    .tr-open:hover:not(:disabled) {
      background: var(--fill-1);
    }
  }

  .tr-open,
  .tr-file {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    max-width: 100%;
    padding: 2px var(--space-1);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
    text-align: left;
  }

  .tr-open {
    font-weight: 600;
  }

  .tr-open svg {
    flex-shrink: 0;
    color: var(--accent);
  }

  .tr-file svg {
    flex-shrink: 0;
    color: var(--text-dim);
  }

  .tr-name {
    width: 100%;
    user-select: text;
  }

  .tr-rename {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .tr-rename .input {
    flex: 1;
    min-width: 0;
  }

  .tr-when,
  .tr-actions {
    white-space: nowrap;
  }

  .tr-save {
    min-width: 0;
  }

  .tr-save-path {
    max-width: 280px;
    overflow: hidden;
    color: var(--text);
    font-size: var(--text-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
  }

  .tr-job-top {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-1);
  }

  .tr-job-name {
    flex: 1 1 160px;
    min-width: 0;
    overflow: hidden;
    font-size: var(--text-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tr-x {
    width: var(--control-h);
    padding: 0;
    justify-content: center;
  }

  .tr-progress {
    display: block;
    width: 100%;
    height: 6px;
    accent-color: var(--accent);
  }

  .dialog-body p {
    margin: 0;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  @media (max-width: 860px) {
    .tr-grid {
      grid-template-columns: minmax(0, 1fr);
    }

    .tr-when {
      display: none;
    }
  }
</style>
