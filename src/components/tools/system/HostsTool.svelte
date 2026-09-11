<script lang="ts">
  /** Bloquear anúncio e telemetria pelo arquivo hosts, só dentro do bloco do OmniGet. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtBytes, onToolProgress, pct, reveal, saveAs, type ToolProgress } from "$lib/tools/rt";

  type Backup = { path: string; bytes: number; name: string };
  type State = {
    path: string;
    exists: boolean;
    writable: boolean;
    bytes: number;
    user_lines: number;
    blocked: number;
    block_present: boolean;
    updated_at: string | null;
    user_preview: string[];
    backups: Backup[];
    sources: string[];
  };
  type SourceStat = { id: string; url: string | null; domains: number; ok: boolean; error: string | null };
  type ApplyResult = {
    path: string;
    dry_run: boolean;
    written: boolean;
    blocked: number;
    blocked_before: number;
    skipped_whitelist: number;
    bytes: number;
    backup: string | null;
    updated_at: string;
    sources: SourceStat[];
    needs_admin: boolean;
    staged: string | null;
    hint: string | null;
    error: string | null;
  };
  type RestoreResult = {
    path: string;
    written: boolean;
    removed_block: boolean;
    restored_from: string | null;
    blocked_before: number;
    needs_admin: boolean;
    hint: string | null;
    error: string | null;
  };

  const SOURCES = [
    { id: "stevenblack", label: "tools.hosts.src_stevenblack" },
    { id: "adaway", label: "tools.hosts.src_adaway" },
    { id: "microsoft", label: "tools.hosts.src_microsoft" },
  ];

  let info = $state<State | null>(null);
  let picked = $state<string[]>(["stevenblack", "microsoft"]);
  let whitelistText = $state("");
  let extraText = $state("");
  let ip = $state("0.0.0.0");
  let dryRun = $state(true);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<ApplyResult | null>(null);
  let backupPick = $state("");
  let unlisten: (() => void) | null = null;

  const whitelist = $derived(whitelistText.split(/[\s,;]+/).filter(Boolean));
  const extra = $derived(extraText.split(/[\s,;]+/).filter(Boolean));

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "sys-hosts") progress = p;
    });
    await refresh();
  });
  onDestroy(() => unlisten?.());

  function toggle(id: string) {
    picked = picked.includes(id) ? picked.filter((s) => s !== id) : [...picked, id];
  }

  async function refresh() {
    try {
      info = await invoke<State>("tool_hosts_read", { opts: {} });
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function copy(text: string) {
    await navigator.clipboard.writeText(text);
    showToast("success", $t("tools.common.copied") as string);
  }

  async function apply(outputPath: string | null) {
    if (busy || !picked.length) return;
    busy = true;
    result = null;
    progress = null;
    try {
      const r = await invoke<ApplyResult>("tool_hosts_apply", {
        opts: {
          sources: picked,
          extra,
          whitelist,
          ip,
          dry_run: outputPath ? false : dryRun,
          output_path: outputPath,
        },
      });
      result = r;
      if (r.error) showToast("error", r.error);
      else if (r.written) showToast("success", `${r.blocked} ${$t("tools.hosts.domains")}`);
      else showToast("info", `${r.blocked} ${$t("tools.hosts.domains")}`);
      await refresh();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function exportTo() {
    const dest = await saveAs("hosts");
    if (dest) await apply(dest);
  }

  async function undo(backup: string | null) {
    if (busy) return;
    busy = true;
    try {
      const r = await invoke<RestoreResult>("tool_hosts_restore", {
        opts: { backup, dry_run: false },
      });
      if (r.error) showToast("error", r.hint ?? r.error);
      else showToast("success", $t("tools.hosts.undone") as string);
      await refresh();
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
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.hosts.file")}</div>
          <div class="group-row-sub mono">{info?.path ?? "—"}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if info?.writable}<span class="tag tag-success">{$t("tools.hosts.writable")}</span>{:else}<span class="tag">{$t("tools.hosts.readonly")}</span>{/if}
          <button class="btn btn-ghost btn-sm" type="button" onclick={refresh}>{$t("tools.common.refresh")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">
            {info?.blocked ?? 0} {$t("tools.hosts.domains")}
            {#if info?.block_present}<span class="tag tag-success">{$t("tools.hosts.block_on")}</span>{/if}
          </div>
          <div class="group-row-sub">
            {info?.user_lines ?? 0} {$t("tools.hosts.user_lines")} · {fmtBytes(info?.bytes ?? 0)}
            {#if info?.updated_at}<span class="dim"> · {$t("tools.hosts.updated")} {info.updated_at}</span>{/if}
          </div>
        </div>
      </div>
      {#if info?.user_preview?.length}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.hosts.yours")}</div>
            <div class="group-row-sub mono pre">{info.user_preview.join("\n")}</div>
          </div>
        </div>
      {/if}
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.hosts.sources")}</div>
          <div class="group-row-sub">{$t("tools.hosts.sources_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#each SOURCES as s (s.id)}
            <button class="btn btn-sm {picked.includes(s.id) ? 'btn-primary' : 'btn-secondary'}" type="button" onclick={() => toggle(s.id)}>{$t(s.label)}</button>
          {/each}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.hosts.whitelist")}</div>
          <div class="group-row-sub">{$t("tools.hosts.whitelist_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" bind:value={whitelistText} placeholder="exemplo.com" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.hosts.extra")}</div>
          <div class="group-row-sub">{$t("tools.hosts.extra_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <input class="input" bind:value={extraText} placeholder="anuncio.exemplo.com" />
          <select class="input" bind:value={ip}><option value="0.0.0.0">0.0.0.0</option><option value="127.0.0.1">127.0.0.1</option></select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.hosts.dry_run")}</div>
          <div class="group-row-sub">{$t("tools.hosts.dry_run_hint")}</div>
        </div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={dryRun} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {:else}
            <div class="group-row-sub">{$t("tools.hosts.safety")}</div>
          {/if}
        </div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-secondary btn-sm" type="button" disabled={busy || !picked.length} onclick={exportTo}>{$t("tools.hosts.export")}</button>
          <button class="btn btn-primary" type="button" disabled={busy || !picked.length} onclick={() => apply(null)}>{busy ? $t("tools.common.working") : dryRun ? $t("tools.hosts.preview") : $t("tools.hosts.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">
              {result.blocked} {$t("tools.hosts.domains")}
              {#if result.dry_run}<span class="tag">{$t("tools.hosts.dry_tag")}</span>{:else if result.written}<span class="tag tag-success">{$t("tools.common.done")}</span>{/if}
            </div>
            <div class="group-row-sub">
              {$t("tools.hosts.was")} {result.blocked_before} · {fmtBytes(result.bytes)}
              {#if result.skipped_whitelist}<span class="dim"> · {result.skipped_whitelist} {$t("tools.hosts.skipped")}</span>{/if}
            </div>
          </div>
          {#if result.written}<div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result!.path)}>{$t("tools.common.reveal")}</button></div>{/if}
        </div>
        {#each result.sources as s (s.id)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{s.id}</div>
              <div class="group-row-sub">{#if s.ok}{s.domains} {$t("tools.hosts.domains")}{:else}<span class="tag tag-danger">{$t("tools.common.failed")}</span> {s.error}{/if}</div>
            </div>
          </div>
        {/each}
        {#if result.error}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title"><span class="tag tag-danger">{$t("tools.hosts.no_permission")}</span></div>
              <div class="group-row-sub">{result.error}</div>
              {#if result.hint}<div class="group-row-sub mono">{result.hint}</div>{/if}
            </div>
            <div class="group-row-trailing btn-row">
              {#if result.hint}<button class="btn btn-secondary btn-sm" type="button" onclick={() => copy(result!.hint!)}>{$t("tools.common.copy")}</button>{/if}
              <button class="btn btn-secondary btn-sm" type="button" onclick={exportTo}>{$t("tools.hosts.export")}</button>
            </div>
          </div>
        {/if}
      </div>
    </section>
  {/if}

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.hosts.undo")}</div>
          <div class="group-row-sub">{$t("tools.hosts.undo_hint")}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" disabled={busy || !info?.block_present} onclick={() => undo(null)}>{$t("tools.hosts.remove_block")}</button></div>
      </div>
      {#if info?.backups?.length}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.hosts.backups")}</div>
            <div class="group-row-sub">{info.backups.length} {$t("tools.hosts.backup_files")}</div>
          </div>
          <div class="group-row-trailing btn-row">
            <select class="input" bind:value={backupPick}>
              <option value="">{$t("tools.common.choose")}</option>
              {#each info.backups as b (b.path)}<option value={b.path}>{b.name}</option>{/each}
            </select>
            <button class="btn btn-secondary btn-sm" type="button" disabled={busy || !backupPick} onclick={() => undo(backupPick)}>{$t("tools.hosts.restore_backup")}</button>
          </div>
        </div>
      {/if}
    </div>
  </section>
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .pre { white-space: pre-wrap; max-height: 9rem; overflow-y: auto; }
</style>
