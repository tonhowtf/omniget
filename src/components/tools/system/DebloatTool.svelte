<script lang="ts">
  /** ⚠️ Só Windows. Debloat de apps da Store (estudo 25, Sophia). */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, pct, type ToolProgress } from "$lib/tools/rt";

  type Pkg = { name: string; full_name: string; version: string; publisher: string; suggested: boolean; label: string; non_removable: boolean };
  type Result = { removed: string[]; failed: string[] };
  let pkgs = $state<Pkg[]>([]);
  let selected = $state<Set<string>>(new Set());
  let loading = $state(true);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let provisioned = $state(false);
  let showAll = $state(false);
  let result = $state<Result | null>(null);
  let error = $state("");

  let unlisten: (() => void) | null = null;
  onMount(async () => {
    unlisten = await onToolProgress((p) => { if (p.id === "debloat") progress = p; });
    await refresh();
  });
  onDestroy(() => unlisten?.());

  async function refresh() {
    loading = true; error = "";
    try { pkgs = await invoke<Pkg[]>("tool_debloat_list"); selected = new Set(pkgs.filter((p) => p.suggested && !p.non_removable).map((p) => p.name)); }
    catch (e) { error = errText(e); } finally { loading = false; }
  }
  function toggle(n: string) { const s = new Set(selected); if (s.has(n)) s.delete(n); else s.add(n); selected = s; }
  async function remove() {
    if (!selected.size || busy) return;
    busy = true; result = null;
    try {
      result = await invoke<Result>("tool_debloat_remove", { names: [...selected], provisioned });
      showToast(result.failed.length ? "info" : "success", `${result.removed.length} ${$t("tools.debloat.removed")}`);
      await refresh();
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; progress = null; }
  }
  async function restore(name: string) {
    try { await invoke("tool_debloat_restore", { name }); showToast("success", $t("tools.common.done") as string); await refresh(); }
    catch (e) { showToast("error", errText(e)); }
  }
  let visible = $derived(showAll ? pkgs : pkgs.filter((p) => p.suggested));
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{pkgs.filter((p) => p.suggested).length} {$t("tools.debloat.suggested_count")} <span class="dim">· {pkgs.length} {$t("tools.debloat.total")}</span></div>
          <div class="group-row-sub">{error || $t("tools.debloat.hint")}</div>
          {#if busy}<div class="group-row-sub">{progress?.message ?? "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}
        </div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => (showAll = !showAll)}>{showAll ? $t("tools.debloat.only_suggested") : $t("tools.debloat.show_all")}</button>
          <button class="btn btn-secondary btn-sm" type="button" disabled={loading} onclick={refresh}>{$t("tools.common.refresh")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.debloat.provisioned")}</div><div class="group-row-sub">{$t("tools.debloat.provisioned_hint")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={provisioned} type="button" role="switch" aria-checked={provisioned} aria-label={$t("tools.debloat.provisioned")} onclick={() => (provisioned = !provisioned)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{selected.size} {$t("tools.sysclean.selected")}</div></div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => (selected = new Set())}>{$t("tools.sysclean.select_none")}</button>
          <button class="btn btn-danger" type="button" disabled={busy || !selected.size} onclick={remove}>{busy ? $t("tools.common.working") : $t("tools.debloat.run")}</button>
        </div>
      </div>
    </div>
  </section>
  {#if result}
    <section><div class="group">
      <div class="group-row"><div class="group-row-content"><div class="group-row-title">{result.removed.length} {$t("tools.debloat.removed")}</div></div></div>
      {#each result.removed as n (n)}<div class="group-row"><div class="group-row-content"><div class="group-row-sub mono">{n}</div></div><div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => restore(n)}>{$t("tools.debloat.restore")}</button></div></div>{/each}
      {#each result.failed as f (f)}<div class="group-row"><div class="group-row-sub mono"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {f}</div></div>{/each}
    </div></section>
  {/if}
  <section>
    <div class="group">
      {#if loading}<div class="group-row"><div class="group-row-sub">…</div></div>{/if}
      {#each visible as p (p.name)}
        <label class="group-row pkg">
          <input type="checkbox" checked={selected.has(p.name)} disabled={p.non_removable || busy} onchange={() => toggle(p.name)} />
          <div class="group-row-content">
            <div class="group-row-title">{p.label} {#if p.suggested}<span class="tag tag-warning">{$t("tools.debloat.bloat")}</span>{/if}{#if p.non_removable}<span class="tag">{$t("tools.debloat.non_removable")}</span>{/if}</div>
            <div class="group-row-sub mono">{p.name} · {p.version}{#if p.publisher} · {p.publisher}{/if}</div>
          </div>
        </label>
      {/each}
      {#if !loading && !visible.length && !error}<div class="group-row"><div class="group-row-sub">{$t("tools.debloat.none")}</div></div>{/if}
    </div>
  </section>
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .pkg { cursor: pointer; gap: var(--space-3); }
  .pkg input { accent-color: var(--accent); }
</style>
