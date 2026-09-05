<script lang="ts">
  /** ⚠️ Só Windows. Limpar registro (estudo 10, Kudu): órfãos com backup .reg antes. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openPath, pct, type ToolProgress } from "$lib/tools/rt";

  type Orphan = { id: string; category: string; key: string; value: string | null; name: string; data: string; reason: string };
  type Result = { removed: number; backups: string[]; failed: string[] };
  const CATS = ["uninstall", "app-paths", "run", "mui-cache", "shared-dlls"];

  let items = $state<Orphan[]>([]);
  let selected = $state<Set<string>>(new Set());
  let busy = $state<"scan" | "fix" | null>(null);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let scanned = $state(false);
  let backupsDir = $state<string | null>(null);

  let unlisten: (() => void) | null = null;
  onMount(async () => {
    unlisten = await onToolProgress((p) => { if (p.id === "winreg") progress = p; });
    backupsDir = await invoke<string | null>("tool_registry_backups_dir");
    await scan();
  });
  onDestroy(() => unlisten?.());

  async function scan() {
    busy = "scan"; result = null;
    try { items = await invoke<Orphan[]>("tool_registry_scan"); selected = new Set(items.map((i) => i.id)); scanned = true; }
    catch (e) { showToast("error", errText(e)); } finally { busy = null; progress = null; }
  }
  function toggle(id: string) { const s = new Set(selected); if (s.has(id)) s.delete(id); else s.add(id); selected = s; }
  async function fix() {
    if (!selected.size || busy) return;
    busy = "fix";
    try {
      result = await invoke<Result>("tool_registry_fix", { items: items.filter((i) => selected.has(i.id)) });
      showToast(result.failed.length ? "info" : "success", `${result.removed} ${$t("tools.winreg.removed")}`);
      await scan();
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{scanned ? `${items.length} ${$t("tools.winreg.found")}` : $t("tools.winreg.title")}</div>
          <div class="group-row-sub">{$t("tools.winreg.hint")}</div>
          {#if busy === "scan"}<div class="group-row-sub">{progress?.message ?? "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}
        </div>
        <div class="group-row-trailing btn-row">
          {#if backupsDir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => openPath(backupsDir!)}>{$t("tools.winreg.backups")}</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={scan}>{$t("tools.common.refresh")}</button>
          <button class="btn btn-danger" type="button" disabled={busy !== null || !selected.size} onclick={fix}>{busy === "fix" ? $t("tools.common.working") : $t("tools.winreg.run")}</button>
        </div>
      </div>
    </div>
  </section>
  {#if result}
    <section><div class="group">
      <div class="group-row"><div class="group-row-content"><div class="group-row-title">{result.removed} {$t("tools.winreg.removed")}</div><div class="group-row-sub">{result.backups.length} {$t("tools.winreg.backups_made")}</div></div></div>
      {#each result.failed as f (f)}<div class="group-row"><div class="group-row-sub mono"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {f}</div></div>{/each}
    </div></section>
  {/if}
  {#each CATS as c (c)}
    {@const list = items.filter((i) => i.category === c)}
    {#if list.length}
      <section>
        <h3 class="group-title">{$t(`tools.winreg.cat_${c.replace("-", "_")}`)} <span class="dim">· {list.length}</span></h3>
        <div class="group">
          {#each list as o (o.id)}
            <label class="group-row orphan">
              <input type="checkbox" checked={selected.has(o.id)} onchange={() => toggle(o.id)} disabled={busy !== null} />
              <div class="group-row-content">
                <div class="group-row-title">{o.name || o.data}</div>
                <div class="group-row-sub mono">{o.key}{#if o.value} → {o.value}{/if}</div>
                <div class="group-row-sub">{o.reason}: <span class="mono">{o.data}</span></div>
              </div>
            </label>
          {/each}
        </div>
      </section>
    {/if}
  {/each}
  {#if scanned && !items.length}
    <div class="tools-empty"><img class="empty-state-art" src="/emoji/sparkles.png" alt="" width="72" height="72" /><h2>{$t("tools.winreg.clean")}</h2></div>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .group-title { margin: 0 0 var(--space-2); font-size: var(--text-sm); font-weight: 600; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em; }
  .orphan { cursor: pointer; gap: var(--space-3); }
  .orphan input { accent-color: var(--accent); }
  .tools-empty { display: flex; flex-direction: column; align-items: center; gap: var(--space-2); padding: var(--space-6); text-align: center; }
  .tools-empty h2 { margin: 0; font-size: var(--text-base); color: var(--text-muted); }
</style>
