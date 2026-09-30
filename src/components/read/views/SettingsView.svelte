<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { addLibraryFolder, bookById, library, refreshShelves, refreshStatus } from "$lib/read/library-store.svelte";
  import { confirm, go, tr } from "$lib/read/ui-store.svelte";
  import type { Catalog, Shelf, SmartRule, SmartRules } from "$lib/read/types";
  import ReadIcon from "../ReadIcon.svelte";

  let { tab = "library" }: { tab?: string } = $props();
  let current = $state(untrack(() => tab));
  let lib = $derived(library());

  const TABS = ["library", "shelves", "sources", "updates", "sync", "devices", "notes", "import"];

  // organize
  let orgTpl = $state("{author}/<{series}/><{index} - >{title}");
  let orgPlan = $state<{ id: number; from: string; to: string }[] | null>(null);
  async function organize(dry: boolean) {
    const ids = lib.books.filter((b) => !b.chapter_id).map((b) => b.id);
    try {
      orgPlan = await invoke("read_organize", { ids, root: dlDir, template: orgTpl, dryRun: dry });
      if (!dry) showToast("success", tr("read.settings.organized", { count: orgPlan?.length ?? 0 }));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  // library
  let dupes = $state<number[][] | null>(null);
  let dlDir = $state("");

  // sources
  let catalogs = $state<Catalog[]>([]);
  let newCat = $state({ kind: "opds", title: "", url: "", username: "", password: "" });
  let repoUrl = $state("");
  let health = $state<Record<number, string>>({});
  type Ext = { pkgName: string; name: string; lang: string; versionName: string; isInstalled: boolean; hasUpdate: boolean; isNsfw: boolean; iconUrl: string };
  let exts = $state<Ext[] | null>(null);
  let extQ = $state("");
  let mihonRepo = $state("");
  async function loadExts() {
    try {
      exts = await invoke<Ext[]>("read_suwayomi_extensions");
    } catch (e) {
      showToast("error", String(e));
    }
  }
  async function extAction(pkg: string, action: string) {
    await wrap(invoke("read_suwayomi_extension", { pkg, action }));
    await loadExts();
  }

  // updates
  let upd = $state({ interval_h: 12, delete_after_read: -1, download_ahead: 0 });

  // sync
  let kosync = $state({ enabled: false, server: "https://sync.koreader.rocks", username: "", password: "", device: "OmniGet" });
  let share = $state({ enabled: false, port: 8580, username: "omniget", password: "" });

  // trackers
  let trackers = $state<{ anilist: string | null; mangaupdates: boolean }>({ anilist: null, mangaupdates: false });
  let aniToken = $state("");
  let muUser = $state("");
  let muPass = $state("");
  async function loadTrackers() {
    trackers = await invoke<typeof trackers>("read_tracker_status").catch(() => trackers);
  }

  // devices
  let mail = $state({ host: "", port: 587, security: "starttls", username: "", password: "", from: "", to: "" });

  // notes
  let notesDir = $state("");

  // shelves
  let editing = $state<{ id: number | null; name: string; smart: boolean; rules: SmartRules } | null>(null);

  onMount(async () => {
    dlDir = lib.status?.download_dir ?? "";
    share = { ...(lib.status?.share ?? share) };
    const [c, u, k, m, n] = await Promise.all([
      invoke<Catalog[]>("read_catalogs").catch(() => []),
      invoke<typeof upd>("read_update_settings", { set: null }).catch(() => upd),
      invoke<typeof kosync>("read_kosync_config", { set: null, register: false }).catch(() => kosync),
      invoke<typeof mail>("read_mail_config", { set: null }).catch(() => mail),
      invoke<string>("read_notes_dir", { dir: null }).catch(() => ""),
    ]);
    catalogs = c;
    upd = u;
    kosync = k;
    mail = { ...mail, ...m };
    notesDir = n;
    void loadTrackers();
  });

  async function wrap(p: Promise<unknown>, ok?: string) {
    try {
      await p;
      if (ok) showToast("success", ok);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function addRoot() {
    const d = await open({ directory: true });
    if (typeof d === "string") await wrap(addLibraryFolder(d), tr("read.settings.folder_added"));
  }

  async function removeRoot(p: string) {
    confirm(tr("read.settings.remove_folder"), tr("read.settings.remove_folder_body"), tr("read.remove"), async () => {
      await invoke("read_roots_remove", { path: p, forgetBooks: true });
      await refreshStatus();
    });
  }

  async function pickDl() {
    const d = await open({ directory: true });
    if (typeof d !== "string") return;
    dlDir = await invoke<string>("read_set_download_dir", { dir: d });
    await refreshStatus();
  }

  async function addCatalog() {
    try {
      await invoke("read_catalog_add", { ...newCat });
      catalogs = await invoke<Catalog[]>("read_catalogs");
      newCat = { kind: newCat.kind, title: "", url: "", username: "", password: "" };
      showToast("success", tr("read.sources.added"));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function installRepo() {
    try {
      const names = await invoke<string[]>("read_repo_install", { url: repoUrl });
      catalogs = await invoke<Catalog[]>("read_catalogs");
      repoUrl = "";
      showToast("success", tr("read.sources.installed", { list: names.join(", ") }));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function test(c: Catalog) {
    health[c.id] = "…";
    try {
      const r = await invoke<{ ok: boolean; list: string | null; details: string | null; chapters: string | null; pages: string | null; sample: string | null }>("read_source_test", { source: `decl:${c.id}` });
      health[c.id] = r.ok ? `✓ ${r.sample ?? ""}` : `✗ ${r.list ?? r.details ?? r.chapters ?? r.pages ?? ""}`;
    } catch (e) {
      health[c.id] = `✗ ${e}`;
    }
  }

  // ---------- shelves ----------
  const FIELDS = ["title", "author", "series", "tag", "subject", "status", "format", "language", "publisher", "rating", "progress", "favorite", "added_days", "read_days", "pages", "size_mb", "read_hours", "source", "path"];
  const NUMERIC = new Set(["rating", "progress", "added_days", "read_days", "pages", "size_mb", "read_hours"]);
  const TEXT_OPS = ["contains", "not_contains", "is", "is_not", "starts", "empty", "not_empty"];
  const NUM_OPS = ["gte", "lte", "gt", "lt", "eq", "ne"];

  function newShelf(smart: boolean) {
    editing = { id: null, name: "", smart, rules: { matching: "all", rules: smart ? [{ field: "status", op: "is", value: "reading" }] : [], sort: "", limit: null } };
  }
  function editShelf(s: Shelf) {
    editing = { id: s.id, name: s.name, smart: !!s.smart, rules: s.smart ? JSON.parse(JSON.stringify(s.smart)) : { matching: "all", rules: [], sort: "", limit: null } };
  }
  async function saveShelf() {
    if (!editing?.name.trim()) return;
    await wrap(invoke("read_shelf_save", { id: editing.id, name: editing.name.trim(), icon: null, smart: editing.smart ? editing.rules : null }));
    editing = null;
    await refreshShelves();
  }
  async function moveShelf(i: number, d: number) {
    const ids = lib.shelves.map((s) => s.id);
    const j = i + d;
    if (j < 0 || j >= ids.length) return;
    [ids[i], ids[j]] = [ids[j], ids[i]];
    await invoke("read_shelf_reorder", { ids });
    await refreshShelves();
  }

  // ---------- backup ----------
  async function exportBackup() {
    const p = await save({ defaultPath: `omniget-read-${new Date().toISOString().slice(0, 10)}.json` });
    if (!p) return;
    await wrap(invoke("read_backup_export", { path: p }), tr("read.import.exported"));
  }
  async function importBackup() {
    const p = await open({ multiple: false, filters: [{ name: "JSON", extensions: ["json"] }] });
    if (typeof p !== "string") return;
    try {
      const msg = await invoke<string>("read_backup_import", { path: p });
      showToast("success", msg);
    } catch (e) {
      showToast("error", String(e));
    }
  }
  let mihon = $state<{ imported: number; unmatched: [string, string][] } | null>(null);
  let mihonBusy = $state(false);
  async function importMihon() {
    const p = await open({ multiple: false, filters: [{ name: "Mihon / Tachiyomi", extensions: ["tachibk", "gz", "proto"] }] });
    if (typeof p !== "string") return;
    mihonBusy = true;
    try {
      mihon = await invoke("read_import_mihon", { path: p });
    } catch (e) {
      showToast("error", String(e));
    } finally {
      mihonBusy = false;
    }
  }

  let shareUrl = $derived(lib.status?.lan_ip ? `http://${lib.status.lan_ip}:${lib.status.share_port ?? share.port}` : "");
</script>

<div class="r-h"><h1>{$t("read.nav.settings")}</h1></div>
<div class="r-toolbar">
  {#each TABS as x (x)}
    <button class="r-chip" class:on={current === x} onclick={() => (current = x)}>{$t(`read.settings.tab_${x}`)}</button>
  {/each}
</div>

{#if current === "library"}
  <div class="r-panel">
    <h3>{$t("read.settings.folders")}</h3>
    <p class="r-muted">{$t("read.settings.folders_hint")}</p>
    <ul class="r-list">
      {#each lib.status?.roots ?? [] as r (r)}
        <li class="r-li"><ReadIcon name="folder" size={15} /><span class="grow mono">{r}</span>
          <button class="r-icon-btn" onclick={() => removeRoot(r)} aria-label={$t("read.remove") as string}><ReadIcon name="trash" size={14} /></button></li>
      {/each}
    </ul>
    <div class="r-line">
      <button class="btn btn-primary" onclick={addRoot}><ReadIcon name="plus" size={14} /> {$t("read.settings.add_folder")}</button>
      <button class="btn" onclick={() => wrap(invoke("read_scan", { full: false }))} disabled={!!lib.scan}><ReadIcon name="refresh" size={14} spin={!!lib.scan} /> {$t("read.settings.scan")}</button>
      <button class="btn" onclick={() => wrap(invoke("read_scan", { full: true }))} disabled={!!lib.scan}>{$t("read.settings.full_scan")}</button>
      {#if lib.scan}<button class="btn" onclick={() => invoke("read_scan_cancel")}>{$t("read.cancel")}</button>{/if}
    </div>
  </div>
  <div class="r-panel">
    <h3>{$t("read.settings.download_dir")}</h3>
    <div class="r-line"><span class="grow mono">{dlDir}</span><button class="btn" onclick={pickDl}>{$t("read.settings.change")}</button></div>
  </div>
  <div class="r-panel">
    <h3>{$t("read.settings.organize")}</h3>
    <p class="r-muted">{$t("read.settings.organize_hint")}</p>
    <div class="r-line">
      <input class="input grow mono" bind:value={orgTpl} />
      <button class="btn" onclick={() => organize(true)}>{$t("read.settings.preview")}</button>
      <button class="btn btn-primary" onclick={() => confirm(tr("read.settings.organize"), tr("read.settings.organize_confirm"), tr("read.settings.organize"), () => organize(false), false)} disabled={!orgPlan?.length}>{$t("read.settings.organize")}</button>
    </div>
    {#if orgPlan}
      <p class="r-muted">{$t("read.settings.organize_count", { count: orgPlan.length })}</p>
      <ul class="plan">{#each orgPlan.slice(0, 40) as m (m.id)}<li class="mono">{m.to.replace(dlDir, "…")}</li>{/each}</ul>
    {/if}
  </div>
  <div class="r-panel">
    <h3>{$t("read.settings.duplicates")}</h3>
    <button class="btn" onclick={async () => (dupes = await invoke<number[][]>("read_duplicates"))}>{$t("read.settings.find_duplicates")}</button>
    {#if dupes}
      {#if !dupes.length}<p class="r-muted">{$t("read.settings.no_duplicates")}</p>{/if}
      {#each dupes as g, i (i)}
        <div class="dup">
          {#each g as id (id)}
            {@const b = bookById(id)}
            {#if b}<button class="r-chip" onclick={() => go({ kind: "book", id })}>{b.title} · {b.format} · {b.path.split(/[/\\]/).slice(-2, -1)[0]}</button>{/if}
          {/each}
        </div>
      {/each}
    {/if}
  </div>
{:else if current === "shelves"}
  <div class="r-panel">
    <div class="r-line">
      <button class="btn" onclick={() => newShelf(false)}><ReadIcon name="shelf" size={14} /> {$t("read.shelf.new")}</button>
      <button class="btn" onclick={() => newShelf(true)}><ReadIcon name="wand" size={14} /> {$t("read.shelf.new_smart")}</button>
    </div>
    <ul class="r-list">
      {#each lib.shelves as s, i (s.id)}
        <li class="r-li">
          <ReadIcon name={s.smart ? "wand" : "shelf"} size={15} />
          <span class="grow">{s.name} <span class="r-muted">({s.book_ids.length})</span></span>
          <button class="r-icon-btn" onclick={() => moveShelf(i, -1)} aria-label="up"><ReadIcon name="up" size={14} /></button>
          <button class="r-icon-btn" onclick={() => moveShelf(i, 1)} aria-label="down"><ReadIcon name="down" size={14} /></button>
          <button class="r-icon-btn" onclick={() => editShelf(s)} aria-label={$t("read.shelf.edit") as string}><ReadIcon name="edit" size={14} /></button>
          <button class="r-icon-btn" onclick={() => confirm(tr("read.shelf.delete"), s.name, tr("read.delete"), async () => { await invoke("read_shelf_delete", { id: s.id }); await refreshShelves(); })} aria-label={$t("read.delete") as string}><ReadIcon name="trash" size={14} /></button>
        </li>
      {/each}
    </ul>
  </div>
  {#if editing}
    <form class="r-panel" onsubmit={(e) => { e.preventDefault(); void saveShelf(); }}>
      <label class="r-field">{$t("read.shelf.name")}<input class="input" bind:value={editing.name} /></label>
      {#if editing.smart}
        <div class="r-line" style="margin-top:10px">
          {$t("read.shelf.match")}
          <select class="input sm" bind:value={editing.rules.matching}>
            <option value="all">{$t("read.shelf.all")}</option>
            <option value="any">{$t("read.shelf.any")}</option>
          </select>
        </div>
        {#each editing.rules.rules as r, i (i)}
          <div class="rule">
            <select class="input sm" bind:value={r.field}>
              {#each FIELDS as f (f)}<option value={f}>{$t(`read.shelf.field_${f}`)}</option>{/each}
            </select>
            <select class="input sm" bind:value={r.op}>
              {#each NUMERIC.has(r.field) ? NUM_OPS : r.field === "favorite" ? ["is"] : TEXT_OPS as o (o)}<option value={o}>{$t(`read.shelf.op_${o}`)}</option>{/each}
            </select>
            {#if r.field === "favorite"}
              <select class="input sm" bind:value={r.value}><option value={true}>✓</option><option value={false}>✗</option></select>
            {:else if !["empty", "not_empty"].includes(r.op)}
              <input class="input sm grow" bind:value={r.value} />
            {/if}
            <button type="button" class="r-icon-btn" onclick={() => editing!.rules.rules.splice(i, 1)} aria-label={$t("read.remove") as string}><ReadIcon name="close" size={14} /></button>
          </div>
        {/each}
        <button type="button" class="r-chip" onclick={() => editing!.rules.rules.push({ field: "title", op: "contains", value: "" } as SmartRule)}><ReadIcon name="plus" size={12} /> {$t("read.shelf.add_rule")}</button>
        <div class="r-line" style="margin-top:10px">
          <label class="r-field">{$t("read.library.sort")}
            <select class="input sm" bind:value={editing.rules.sort}>
              <option value="">{$t("read.library.sort_title")}</option>
              <option value="added">{$t("read.library.sort_added")}</option>
              <option value="read">{$t("read.library.sort_read")}</option>
              <option value="rating">{$t("read.library.sort_rating")}</option>
              <option value="random">{$t("read.shelf.random")}</option>
            </select>
          </label>
          <label class="r-field">{$t("read.shelf.limit")}<input class="input sm" type="number" min="0" bind:value={editing.rules.limit} /></label>
        </div>
      {/if}
      <div class="dialog-actions">
        <button type="button" class="btn" onclick={() => (editing = null)}>{$t("read.cancel")}</button>
        <button type="submit" class="btn btn-primary">{$t("read.save")}</button>
      </div>
    </form>
  {/if}
{:else if current === "sources"}
  <div class="r-panel">
    <h3>{$t("read.sources.builtin")}</h3>
    <p class="r-muted">{$t("read.sources.builtin_hint")}</p>
  </div>
  <div class="r-panel">
    <h3>{$t("read.sources.catalogs")}</h3>
    <p class="r-muted">{$t("read.sources.catalogs_hint")}</p>
    <ul class="r-list">
      {#each catalogs.filter((c) => c.kind !== "declarative") as c (c.id)}
        <li class="r-li">
          <ReadIcon name={c.kind === "suwayomi" ? "server" : "rss"} size={15} />
          <div class="grow"><b>{c.title}</b> <span class="r-badge">{c.kind}</span><br /><span class="r-muted mono">{c.url}</span></div>
          <label class="r-line"><input type="checkbox" checked={c.enabled} onchange={(e) => invoke("read_catalog_toggle", { id: c.id, enabled: (e.target as HTMLInputElement).checked })} /> {$t("read.sources.enabled")}</label>
          <button class="r-icon-btn" onclick={async () => { await invoke("read_catalog_remove", { id: c.id }); catalogs = catalogs.filter((x) => x.id !== c.id); }} aria-label={$t("read.remove") as string}><ReadIcon name="trash" size={14} /></button>
        </li>
      {/each}
    </ul>
    <form class="r-fields" onsubmit={(e) => { e.preventDefault(); void addCatalog(); }}>
      <label class="r-field">{$t("read.sources.kind")}
        <select class="input" bind:value={newCat.kind}>
          <option value="opds">OPDS (Calibre-Web, Komga, Kavita, Stump…)</option>
          <option value="suwayomi">Suwayomi-Server</option>
        </select>
      </label>
      <label class="r-field">URL<input class="input" bind:value={newCat.url} placeholder={newCat.kind === "suwayomi" ? "http://127.0.0.1:4567" : "https://…/opds"} /></label>
      <label class="r-field">{$t("read.sources.title")}<input class="input" bind:value={newCat.title} /></label>
      <label class="r-field">{$t("read.sources.user")}<input class="input" bind:value={newCat.username} autocomplete="off" /></label>
      <label class="r-field">{$t("read.sources.password")}<input class="input" type="password" bind:value={newCat.password} autocomplete="off" /></label>
      <div class="r-field end"><button class="btn btn-primary" type="submit" disabled={!newCat.url.trim()}>{$t("read.sources.add")}</button></div>
    </form>
    {#if newCat.kind === "suwayomi"}<p class="r-muted">{$t("read.sources.suwayomi_hint")}</p>{/if}
  </div>
  {#if catalogs.some((c) => c.kind === "suwayomi" && c.enabled)}
    <div class="r-panel">
      <h3>{$t("read.sources.suwayomi_ext")}</h3>
      <form class="r-line" onsubmit={(e) => { e.preventDefault(); void wrap(invoke("read_suwayomi_add_repo", { url: mihonRepo }).then(loadExts), tr("read.sources.added")); mihonRepo = ""; }}>
        <input class="input grow" bind:value={mihonRepo} placeholder={$t("read.sources.mihon_repo") as string} />
        <button class="btn" type="submit" disabled={!mihonRepo.trim()}>{$t("read.sources.add")}</button>
        <button class="btn" type="button" onclick={loadExts}><ReadIcon name="refresh" size={14} /> {$t("read.sources.list_ext")}</button>
      </form>
      {#if exts}
        <input class="input" style="margin:8px 0" bind:value={extQ} placeholder={$t("read.notes.search") as string} />
        <ul class="r-list ext">
          {#each exts.filter((x) => !extQ || x.name.toLowerCase().includes(extQ.toLowerCase())).slice(0, 200) as x (x.pkgName)}
            <li class="r-li">
              <img src={x.iconUrl} alt="" width="22" height="22" />
              <span class="grow">{x.name} <span class="r-badge">{x.lang}</span> <span class="r-muted">{x.versionName}</span>{#if x.isNsfw} <span class="r-badge">18+</span>{/if}</span>
              {#if x.isInstalled}
                {#if x.hasUpdate}<button class="btn" onclick={() => extAction(x.pkgName, "update")}>{$t("read.sources.update")}</button>{/if}
                <button class="btn" onclick={() => extAction(x.pkgName, "uninstall")}>{$t("read.remove")}</button>
              {:else}
                <button class="btn" onclick={() => extAction(x.pkgName, "install")}>{$t("read.sources.install")}</button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
  <div class="r-panel">
    <h3>{$t("read.sources.repos")}</h3>
    <p class="r-muted">{$t("read.sources.repos_hint")}</p>
    <form class="r-line" onsubmit={(e) => { e.preventDefault(); void installRepo(); }}>
      <input class="input grow" bind:value={repoUrl} placeholder="https://…/omniget-sources.json" />
      <button class="btn btn-primary" type="submit" disabled={!repoUrl.trim()}>{$t("read.sources.install")}</button>
    </form>
    <ul class="r-list">
      {#each catalogs.filter((c) => c.kind === "declarative") as c (c.id)}
        <li class="r-li">
          <ReadIcon name="layers" size={15} />
          <div class="grow"><b>{c.title}</b> <span class="r-badge">{c.config?.source?.lang ?? ""}</span><br /><span class="r-muted mono">{c.url}</span>
            {#if health[c.id]}<br /><span class="r-muted">{health[c.id]}</span>{/if}</div>
          <button class="btn" onclick={() => test(c)}>{$t("read.sources.test")}</button>
          <label class="r-line"><input type="checkbox" checked={c.enabled} onchange={(e) => invoke("read_catalog_toggle", { id: c.id, enabled: (e.target as HTMLInputElement).checked })} /></label>
          <button class="r-icon-btn" onclick={async () => { await invoke("read_catalog_remove", { id: c.id }); catalogs = catalogs.filter((x) => x.id !== c.id); }} aria-label={$t("read.remove") as string}><ReadIcon name="trash" size={14} /></button>
        </li>
      {/each}
    </ul>
    <p class="r-muted warn"><ReadIcon name="alert" size={13} /> {$t("read.sources.legal")}</p>
  </div>
{:else if current === "updates"}
  <form class="r-panel" onsubmit={(e) => { e.preventDefault(); void wrap(invoke("read_update_settings", { set: upd }), tr("read.saved")); }}>
    <div class="r-fields">
      <label class="r-field">{$t("read.settings.update_interval")}
        <select class="input" bind:value={upd.interval_h}>
          {#each [0, 1, 3, 6, 12, 24, 48, 168] as h (h)}<option value={h}>{h ? $t("read.settings.every_hours", { hours: h }) : $t("read.settings.never")}</option>{/each}
        </select>
      </label>
      <label class="r-field">{$t("read.settings.delete_after_read")}
        <select class="input" bind:value={upd.delete_after_read}>
          <option value={-1}>{$t("read.settings.never")}</option>
          {#each [0, 1, 2, 5, 10] as n (n)}<option value={n}>{$t("read.settings.keep_behind", { count: n })}</option>{/each}
        </select>
      </label>
      <label class="r-field">{$t("read.settings.download_ahead")}
        <select class="input" bind:value={upd.download_ahead}>
          {#each [0, 1, 2, 3, 5, 10] as n (n)}<option value={n}>{n ? n : $t("read.settings.off")}</option>{/each}
        </select>
      </label>
    </div>
    <div class="dialog-actions"><button class="btn btn-primary" type="submit">{$t("read.save")}</button></div>
  </form>
{:else if current === "sync"}
  <form class="r-panel" onsubmit={async (e) => { e.preventDefault(); try { kosync = await invoke("read_kosync_config", { set: kosync, register: false }); showToast("success", tr("read.saved")); } catch (err) { showToast("error", String(err)); } }}>
    <h3>{$t("read.sync.kosync")}</h3>
    <p class="r-muted">{$t("read.sync.kosync_hint")}</p>
    <label class="r-line"><input type="checkbox" bind:checked={kosync.enabled} /> {$t("read.sources.enabled")}</label>
    <div class="r-fields">
      <label class="r-field">{$t("read.sync.server")}<input class="input" bind:value={kosync.server} /></label>
      <label class="r-field">{$t("read.sources.user")}<input class="input" bind:value={kosync.username} autocomplete="off" /></label>
      <label class="r-field">{$t("read.sources.password")}<input class="input" type="password" bind:value={kosync.password} autocomplete="off" /></label>
      <label class="r-field">{$t("read.sync.device")}<input class="input" bind:value={kosync.device} /></label>
    </div>
    <div class="dialog-actions">
      <button type="button" class="btn" onclick={async () => { try { kosync = await invoke("read_kosync_config", { set: kosync, register: true }); showToast("success", tr("read.sync.registered")); } catch (err) { showToast("error", String(err)); } }}>{$t("read.sync.register")}</button>
      <button class="btn btn-primary" type="submit">{$t("read.save")}</button>
    </div>
  </form>
  <div class="r-panel">
    <h3>{$t("read.track.trackers")}</h3>
    <p class="r-muted">{$t("read.track.trackers_hint")}</p>
    <div class="r-line">
      <b>AniList</b>
      {#if trackers.anilist}
        <span class="grow">{$t("read.track.logged_as", { name: trackers.anilist })}</span>
        <button class="btn" onclick={async () => { await invoke("read_tracker_login", { service: "anilist", token: "" }); await loadTrackers(); }}>{$t("read.track.logout")}</button>
      {:else}
        <input class="input grow" type="password" bind:value={aniToken} placeholder={$t("read.track.anilist_token") as string} />
        <button class="btn" onclick={() => wrap(invoke("read_tracker_login", { service: "anilist", token: aniToken }).then(loadTrackers), tr("read.saved"))}>{$t("read.track.login")}</button>
      {/if}
    </div>
    <div class="r-line" style="margin-top:8px">
      <b>MangaUpdates</b>
      {#if trackers.mangaupdates}
        <span class="grow">{$t("read.track.connected")}</span>
        <button class="btn" onclick={async () => { await invoke("read_tracker_login", { service: "mangaupdates", user: "" }); await loadTrackers(); }}>{$t("read.track.logout")}</button>
      {:else}
        <input class="input" bind:value={muUser} placeholder={$t("read.sources.user") as string} />
        <input class="input" type="password" bind:value={muPass} placeholder={$t("read.sources.password") as string} />
        <button class="btn" onclick={() => wrap(invoke("read_tracker_login", { service: "mangaupdates", user: muUser, password: muPass }).then(loadTrackers), tr("read.saved"))}>{$t("read.track.login")}</button>
      {/if}
    </div>
  </div>
  <form class="r-panel" onsubmit={async (e) => { e.preventDefault(); try { await invoke("read_share_set", { cfg: share }); await refreshStatus(); showToast("success", tr("read.saved")); } catch (err) { showToast("error", String(err)); } }}>
    <h3>{$t("read.sync.share")}</h3>
    <p class="r-muted">{$t("read.sync.share_hint")}</p>
    <label class="r-line"><input type="checkbox" bind:checked={share.enabled} /> {$t("read.sync.share_on")}</label>
    <div class="r-fields">
      <label class="r-field">{$t("read.sync.port")}<input class="input" type="number" bind:value={share.port} /></label>
      <label class="r-field">{$t("read.sources.user")}<input class="input" bind:value={share.username} autocomplete="off" /></label>
      <label class="r-field">{$t("read.sources.password")}<input class="input" type="password" bind:value={share.password} autocomplete="off" /></label>
    </div>
    {#if lib.status?.share_running && shareUrl}
      <div class="urls">
        <span>OPDS: <code>{shareUrl}/opds</code></span>
        <span>KOReader sync: <code>{shareUrl}</code></span>
      </div>
    {/if}
    <div class="dialog-actions"><button class="btn btn-primary" type="submit">{$t("read.save")}</button></div>
  </form>
{:else if current === "devices"}
  <form class="r-panel" onsubmit={async (e) => { e.preventDefault(); try { mail = { ...mail, ...(await invoke<typeof mail>("read_mail_config", { set: mail })) }; showToast("success", tr("read.saved")); } catch (err) { showToast("error", String(err)); } }}>
    <h3>{$t("read.tools.email")}</h3>
    <p class="r-muted">{$t("read.settings.mail_hint")}</p>
    <div class="r-fields">
      <label class="r-field">SMTP<input class="input" bind:value={mail.host} placeholder="smtp.gmail.com" /></label>
      <label class="r-field">{$t("read.sync.port")}<input class="input" type="number" bind:value={mail.port} /></label>
      <label class="r-field">{$t("read.settings.security")}
        <select class="input" bind:value={mail.security}><option value="starttls">STARTTLS</option><option value="tls">TLS</option><option value="none">{$t("read.settings.none")}</option></select>
      </label>
      <label class="r-field">{$t("read.sources.user")}<input class="input" bind:value={mail.username} autocomplete="off" /></label>
      <label class="r-field">{$t("read.sources.password")}<input class="input" type="password" bind:value={mail.password} autocomplete="off" /></label>
      <label class="r-field">{$t("read.settings.from")}<input class="input" bind:value={mail.from} /></label>
      <label class="r-field">{$t("read.settings.to_default")}<input class="input" bind:value={mail.to} placeholder="you@kindle.com" /></label>
    </div>
    <div class="dialog-actions"><button class="btn btn-primary" type="submit">{$t("read.save")}</button></div>
  </form>
{:else if current === "notes"}
  <div class="r-panel">
    <h3>{$t("read.notes.live_export")}</h3>
    <p class="r-muted">{$t("read.notes.live_hint")}</p>
    <div class="r-line">
      <span class="grow mono">{notesDir || $t("read.settings.off")}</span>
      <button class="btn" onclick={async () => { const d = await open({ directory: true }); if (typeof d === "string") notesDir = await invoke<string>("read_notes_dir", { dir: d }); }}>{$t("read.settings.change")}</button>
      {#if notesDir}<button class="btn" onclick={async () => (notesDir = await invoke<string>("read_notes_dir", { dir: "" }))}>{$t("read.settings.turn_off")}</button>{/if}
    </div>
  </div>
{:else if current === "import"}
  <div class="r-panel">
    <h3>{$t("read.import.backup")}</h3>
    <p class="r-muted">{$t("read.import.backup_hint")}</p>
    <div class="r-line">
      <button class="btn" onclick={exportBackup}><ReadIcon name="upload" size={14} /> {$t("read.import.export")}</button>
      <button class="btn" onclick={importBackup}><ReadIcon name="download" size={14} /> {$t("read.import.restore")}</button>
    </div>
  </div>
  <div class="r-panel">
    <h3>{$t("read.import.mihon")}</h3>
    <p class="r-muted">{$t("read.import.mihon_hint")}</p>
    <button class="btn" onclick={importMihon} disabled={mihonBusy}><ReadIcon name="download" size={14} spin={false} /> {mihonBusy ? $t("read.import.importing") : $t("read.import.pick_mihon")}</button>
    {#if mihon}
      <p>{$t("read.import.mihon_done", { count: mihon.imported })}</p>
      {#if mihon.unmatched.length}
        <details><summary>{$t("read.import.unmatched", { count: mihon.unmatched.length })}</summary>
          <ul>{#each mihon.unmatched as [title, src], i (i)}<li>{title} <span class="r-muted">{src}</span></li>{/each}</ul>
        </details>
      {/if}
    {/if}
  </div>
  <div class="r-panel">
    <h3>{$t("read.import.files")}</h3>
    <p class="r-muted">{$t("read.drop_hint")}</p>
    <button class="btn" onclick={async () => { const f = await open({ multiple: true }); const paths = Array.isArray(f) ? f : typeof f === "string" ? [f] : []; if (paths.length) { const rows = await invoke<unknown[]>("read_import_paths", { paths }); showToast("success", tr("read.import.added", { count: rows.length })); } }}><ReadIcon name="plus" size={14} /> {$t("read.import.pick_files")}</button>
  </div>
{/if}

<style>
  h3 {
    margin: 0 0 6px;
    font-size: var(--text-base);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    word-break: break-all;
  }
  .plan {
    max-height: 220px;
    overflow-y: auto;
    margin: 6px 0 0;
    padding-left: 18px;
  }
  .dup {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 8px;
    padding-top: 8px;
    border-top: var(--hairline) solid var(--content-border);
  }
  .rule {
    display: flex;
    gap: 6px;
    align-items: center;
    margin: 6px 0;
  }
  .sm {
    height: 30px;
    padding: 0 8px;
  }
  .end {
    justify-content: flex-end;
  }
  .urls {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 10px 0;
    font-size: var(--text-sm);
  }
  .ext {
    max-height: 420px;
    overflow-y: auto;
  }
  .ext img {
    border-radius: 4px;
  }
  .warn {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    margin-top: 10px;
  }
</style>
