<script lang="ts">
  /** ⚠️ Só Windows. Atualizar programas em massa (estudo 10, Kudu) com winget, Chocolatey e Scoop. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, pct, type ToolProgress } from "$lib/tools/rt";

  type Item = { manager: string; id: string; name: string; current: string; available: string };
  type Status = { winget: boolean; choco: boolean; scoop: boolean; items: Item[] };
  type Result = { upgraded: string[]; failed: string[] };

  let status = $state<Status | null>(null);
  let selected = $state<Set<string>>(new Set());
  let busy = $state<"scan" | "upgrade" | null>(null);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);

  let unlisten: (() => void) | null = null;
  onMount(async () => {
    unlisten = await onToolProgress((p) => { if (p.id === "updater") progress = p; });
    await refresh();
  });
  onDestroy(() => unlisten?.());

  const key = (i: Item) => `${i.manager}:${i.id}`;
  async function refresh() {
    busy = "scan"; result = null;
    try { status = await invoke<Status>("tool_updater_status"); selected = new Set(status.items.map(key)); }
    catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }
  function toggle(k: string) { const s = new Set(selected); if (s.has(k)) s.delete(k); else s.add(k); selected = s; }
  async function upgrade() {
    if (!status || !selected.size || busy) return;
    busy = "upgrade";
    try {
      result = await invoke<Result>("tool_updater_upgrade", { items: status.items.filter((i) => selected.has(key(i))) });
      showToast(result.failed.length ? "info" : "success", `${result.upgraded.length} ${$t("tools.updater.upgraded")}`);
      await refresh();
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; progress = null; }
  }
  let managers = $derived(status ? [status.winget && "winget", status.choco && "Chocolatey", status.scoop && "Scoop"].filter(Boolean).join(", ") : "");
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{status ? `${status.items.length} ${$t("tools.updater.available")}` : "…"}</div>
          <div class="group-row-sub">{managers ? `${$t("tools.updater.managers")}: ${managers}` : $t("tools.updater.no_manager")}</div>
          {#if busy === "upgrade"}<div class="group-row-sub">{progress?.message ?? "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}
        </div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={refresh}>{busy === "scan" ? $t("tools.common.working") : $t("tools.common.refresh")}</button>
          <button class="btn btn-primary" type="button" disabled={busy !== null || !selected.size} onclick={upgrade}>{busy === "upgrade" ? $t("tools.common.working") : $t("tools.updater.run")}</button>
        </div>
      </div>
    </div>
  </section>
  {#if result}
    <section><div class="group">
      <div class="group-row"><div class="group-row-content"><div class="group-row-title">{result.upgraded.length} {$t("tools.updater.upgraded")}</div><div class="group-row-sub">{result.upgraded.join(", ")}</div></div></div>
      {#each result.failed as f (f)}<div class="group-row"><div class="group-row-sub mono"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {f}</div></div>{/each}
    </div></section>
  {/if}
  <section>
    <div class="group">
      {#if status && !status.items.length && busy === null}<div class="group-row"><div class="group-row-sub">{$t("tools.updater.up_to_date")}</div></div>{/if}
      {#each status?.items ?? [] as i (key(i))}
        <label class="group-row item">
          <input type="checkbox" checked={selected.has(key(i))} onchange={() => toggle(key(i))} disabled={busy !== null} />
          <div class="group-row-content">
            <div class="group-row-title">{i.name} <span class="tag">{i.manager}</span></div>
            <div class="group-row-sub mono">{i.id} · {i.current} → <strong>{i.available}</strong></div>
          </div>
        </label>
      {/each}
    </div>
  </section>
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .item { cursor: pointer; gap: var(--space-3); }
  .item input { accent-color: var(--accent); }
</style>
