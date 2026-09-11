<script lang="ts">
  /**
   * Etiquetas de música: lê a pasta, aponta o que está errado e corrige em
   * lote. Nada é gravado sem o usuário ver o diff antes — o botão de aplicar
   * só liga depois de uma prévia.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, fmtSecs, onToolProgress, pct, pickFile, pickFiles, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Tags = {
    title: string | null; artist: string | null; album_artist: string | null; album: string | null;
    year: string | null; track: number | null; track_total: number | null; disc: number | null;
    disc_total: number | null; genre: string | null; comment: string | null;
    track_raw: string | null; disc_raw: string | null;
  };
  type Issue = { code: string; field: string | null; severity: string; detail: string; fixable: boolean };
  type Cover = { count: number; bytes: number; mime: string | null; front_count: number; duplicated: boolean };
  type Track = {
    path: string; file_name: string; format: string; tag_type: string | null; primary_tag_type: string;
    bytes: number; duration_secs: number; bitrate_kbps: number | null; sample_rate: number | null;
    channels: number | null; tags: Tags; cover: Cover; issues: Issue[]; error: string | null;
  };
  type ScanResult = { scanned: number; failed: number; with_issues: number; issue_counts: Record<string, number>; tracks: Track[] };
  type Diff = { field: string; from: string | null; to: string | null; reason: string };
  type CoverDiff = { action: string; from_bytes: number; to_bytes: number };
  type FileChange = { path: string; file_name: string; diffs: Diff[]; cover: CoverDiff | null; unsupported: string[]; written: boolean; backup: string | null; error: string | null };
  type EditResult = { dry_run: boolean; changed: number; written: number; failed: number; files: FileChange[] };

  const SET_FIELDS = ["artist", "album_artist", "album", "year", "genre", "comment"] as const;
  const FILTERS_AUDIO = [{ name: "Audio", extensions: ["mp3", "m4a", "m4b", "flac", "ogg", "oga", "opus", "wav", "aiff", "aif", "aac", "ape", "wv", "mpc"] }];
  const FILTERS_IMAGES = [{ name: "Images", extensions: ["jpg", "jpeg", "png"] }];

  let dir = $state("");
  let extraFiles = $state<string[]>([]);
  let recursive = $state(true);

  let setValues = $state<Record<string, string>>({ artist: "", album_artist: "", album: "", year: "", genre: "", comment: "" });
  let clearFields = $state<string[]>([]);
  let fixMojibake = $state(false);
  let splitPair = $state(false);
  let swapTitleArtist = $state(false);
  let albumArtistFromArtist = $state(false);
  let renumber = $state(false);
  let setTrackTotal = $state(true);
  let coverPath = $state("");
  let removeCover = $state(false);
  let dedupeCover = $state(false);
  let backup = $state(true);

  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let scanResult = $state<ScanResult | null>(null);
  let preview = $state<EditResult | null>(null);
  let unlisten: (() => void) | null = null;

  const tracks = $derived(scanResult?.tracks ?? []);
  const issueCodes = $derived(Object.entries(scanResult?.issue_counts ?? {}).sort((a, b) => b[1] - a[1]));
  const previewChanges = $derived((preview?.files ?? []).filter((f) => f.diffs.length || f.cover || f.error));

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "audio-tag") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  function label(field: string): string {
    return $t(`tools.atag.f_${field}`);
  }

  function issueLabel(code: string): string {
    return $t(`tools.atag.issue_${code}`);
  }

  function toggleClear(field: string) {
    clearFields = clearFields.includes(field) ? clearFields.filter((f) => f !== field) : [...clearFields, field];
  }

  function editOptions(dryRun: boolean) {
    const set: Record<string, string> = {};
    for (const f of SET_FIELDS) {
      const v = setValues[f]?.trim();
      if (v) set[f] = v;
    }
    return {
      files: tracks.filter((tr) => !tr.error).map((tr) => tr.path),
      set,
      clear: clearFields,
      sort_by_name: true,
      renumber,
      renumber_start: 1,
      set_track_total: setTrackTotal,
      album_artist_from_artist: albumArtistFromArtist,
      fix_mojibake: fixMojibake,
      split_track_slash: splitPair,
      swap_title_artist: swapTitleArtist,
      cover_path: coverPath || null,
      remove_cover: removeCover,
      dedupe_cover: dedupeCover,
      backup,
      dry_run: dryRun,
    };
  }

  async function runScan() {
    if (busy || (!dir && !extraFiles.length)) return;
    busy = true;
    preview = null;
    scanResult = null;
    progress = null;
    try {
      scanResult = await invoke<ScanResult>("tool_audio_tag_scan", {
        opts: { dirs: dir ? [dir] : [], files: extraFiles, recursive, max_files: 2000 },
      });
      showToast("success", `${scanResult.scanned} ${$t("tools.common.files")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function runPreview() {
    if (busy || !tracks.length) return;
    busy = true;
    progress = null;
    try {
      preview = await invoke<EditResult>("tool_audio_tag_write", { opts: editOptions(true) });
      if (!preview.changed) showToast("info", $t("tools.atag.no_changes"));
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function runApply() {
    if (busy || !preview || !preview.changed) return;
    busy = true;
    progress = null;
    try {
      const done = await invoke<EditResult>("tool_audio_tag_write", { opts: editOptions(false) });
      showToast(done.failed ? "info" : "success", `${done.written} ${$t("tools.atag.written")}`);
      preview = null;
      await runScan();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.atag.folder")}</div><div class="group-row-sub mono">{dir || "—"}</div></div>
        <div class="group-row-trailing btn-row">
          {#if dir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (dir = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dir = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.atag.recursive")}</div><div class="group-row-sub">{extraFiles.length ? `${extraFiles.length} ${$t("tools.common.files")}` : $t("tools.atag.recursive_sub")}</div></div>
        <div class="group-row-trailing btn-row">
          <input type="checkbox" bind:checked={recursive} />
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { extraFiles = [...extraFiles, ...(await pickFiles(FILTERS_AUDIO))]; }}>{$t("tools.common.add")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || (!dir && !extraFiles.length)} onclick={runScan}>{busy ? $t("tools.common.working") : $t("tools.atag.scan")}</button></div>
      </div>
    </div>
  </section>

  {#if scanResult}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{scanResult.scanned} {$t("tools.common.files")}</div>
            <div class="group-row-sub">
              {#if scanResult.with_issues}{scanResult.with_issues} {$t("tools.atag.with_issues")}{:else}{$t("tools.atag.no_issues")}{/if}
              {#if scanResult.failed} · {scanResult.failed} {$t("tools.common.failed")}{/if}
            </div>
          </div>
        </div>
        {#if issueCodes.length}
          <div class="group-row">
            <div class="group-row-content chips">
              {#each issueCodes as [code, count] (code)}<span class="tag">{issueLabel(code)} {count}</span>{/each}
            </div>
          </div>
        {/if}
      </div>
    </section>

    <section>
      <div class="group">
        {#each tracks.slice(0, 200) as tr (tr.path)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{tr.tags.title || baseName(tr.path)}</div>
              <div class="group-row-sub">
                {tr.tags.artist || "—"}{#if tr.tags.album} · {tr.tags.album}{/if}
                <span class="dim"> · {tr.format} · {fmtSecs(tr.duration_secs)}{#if tr.bitrate_kbps} · {tr.bitrate_kbps} kbps{/if}{#if tr.tags.track} · #{tr.tags.track_raw ?? tr.tags.track}{/if}{#if tr.cover.count} · {$t("tools.atag.cover")} {fmtBytes(tr.cover.bytes)}{/if}</span>
              </div>
              {#if tr.error}
                <div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {tr.error}</div>
              {:else if tr.issues.length}
                <div class="group-row-sub chips">
                  {#each tr.issues as issue, i (i)}<span class="tag" title={issue.detail}>{issueLabel(issue.code)}{#if issue.field} · {label(issue.field)}{/if}</span>{/each}
                </div>
              {/if}
            </div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(tr.path)}>↗</button></div>
          </div>
        {/each}
      </div>
    </section>

    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.atag.batch")}</div><div class="group-row-sub">{$t("tools.atag.batch_sub")}</div></div>
        </div>
        {#each SET_FIELDS as f (f)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{label(f)}</div>{#if clearFields.includes(f)}<div class="group-row-sub"><span class="tag tag-danger">{$t("tools.atag.will_clear")}</span></div>{/if}</div>
            <div class="group-row-trailing btn-row">
              <input class="input" type="text" placeholder={$t("tools.atag.keep")} bind:value={setValues[f]} disabled={clearFields.includes(f)} />
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => toggleClear(f)}>{$t("tools.atag.clear")}</button>
            </div>
          </div>
        {/each}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.atag.fix_mojibake")}</div><div class="group-row-sub">{$t("tools.atag.fix_mojibake_sub")}</div></div>
          <div class="group-row-trailing"><input type="checkbox" bind:checked={fixMojibake} /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.atag.split_pair")}</div><div class="group-row-sub">{$t("tools.atag.split_pair_sub")}</div></div>
          <div class="group-row-trailing"><input type="checkbox" bind:checked={splitPair} /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.atag.swap")}</div><div class="group-row-sub">{$t("tools.atag.swap_sub")}</div></div>
          <div class="group-row-trailing"><input type="checkbox" bind:checked={swapTitleArtist} /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.atag.album_artist_from_artist")}</div></div>
          <div class="group-row-trailing"><input type="checkbox" bind:checked={albumArtistFromArtist} /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.atag.renumber")}</div><div class="group-row-sub">{$t("tools.atag.renumber_sub")}</div></div>
          <div class="group-row-trailing btn-row"><input type="checkbox" bind:checked={renumber} />{#if renumber}<label class="dim">{$t("tools.atag.set_track_total")} <input type="checkbox" bind:checked={setTrackTotal} /></label>{/if}</div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.atag.set_cover")}</div><div class="group-row-sub mono">{coverPath ? baseName(coverPath) : "—"}</div></div>
          <div class="group-row-trailing btn-row">
            {#if coverPath}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (coverPath = "")}>×</button>{/if}
            <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(FILTERS_IMAGES); if (f) coverPath = f; }}>{$t("tools.common.choose")}</button>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.atag.dedupe_cover")}</div></div>
          <div class="group-row-trailing"><input type="checkbox" bind:checked={dedupeCover} /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.atag.remove_cover")}</div></div>
          <div class="group-row-trailing"><input type="checkbox" bind:checked={removeCover} /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.atag.backup")}</div><div class="group-row-sub">{$t("tools.atag.backup_sub")}</div></div>
          <div class="group-row-trailing"><input type="checkbox" bind:checked={backup} /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-sub">{$t("tools.atag.preview_note")}</div></div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary" type="button" disabled={busy || !tracks.length} onclick={runPreview}>{$t("tools.atag.preview")}</button>
            <button class="btn btn-primary" type="button" disabled={busy || !preview?.changed} onclick={runApply}>{$t("tools.atag.apply")}</button>
          </div>
        </div>
      </div>
    </section>
  {/if}

  {#if preview}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{preview.changed} {$t("tools.atag.changes")}</div>
            <div class="group-row-sub">{$t("tools.atag.dry_run_note")}</div>
          </div>
        </div>
        {#each previewChanges.slice(0, 200) as change (change.path)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{change.file_name}</div>
              {#if change.error}
                <div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {change.error}</div>
              {:else}
                {#each change.diffs as d (d.field)}
                  <div class="group-row-sub diff"><span class="field">{label(d.field)}</span> <span class="old">{d.from ?? "—"}</span> → <span class="new">{d.to ?? "—"}</span></div>
                {/each}
                {#if change.cover}
                  <div class="group-row-sub diff"><span class="field">{$t("tools.atag.cover")}</span> {$t(`tools.atag.cover_${change.cover.action}`)} <span class="dim">{fmtBytes(change.cover.from_bytes)} → {fmtBytes(change.cover.to_bytes)}</span></div>
                {/if}
                {#if change.unsupported.length}
                  <div class="group-row-sub"><span class="tag tag-danger">{$t("tools.atag.unsupported")}</span> {change.unsupported.map(label).join(", ")}</div>
                {/if}
              {/if}
            </div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .chips { display: flex; flex-wrap: wrap; gap: var(--space-1); }
  .diff { display: flex; flex-wrap: wrap; align-items: baseline; gap: var(--space-1); }
  .diff .field { color: var(--text-dim); min-width: 7em; }
  .diff .old { text-decoration: line-through; color: var(--text-dim); }
  .diff .new { font-weight: 600; }
</style>
