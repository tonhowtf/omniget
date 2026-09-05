<script lang="ts">
  /** Limpar caches (estudo 10, Kudu): varrer → revisar → limpar. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtBytes, onToolProgress, pct, type ToolProgress } from "$lib/tools/rt";

  type Rule = { id: string; group: string; name: string; risk: string; paths: string[]; bytes: number; files: number };
  type Result = { freed: number; removed: number; failed: string[] };
  const GROUPS = ["system", "browsers", "apps", "dev", "gaming"];

  let rules = $state<Rule[]>([]);
  let selected = $state<Set<string>>(new Set());
  let busy = $state<"scan" | "clean" | null>(null);
  let progress = $state<ToolProgress | null>(null);
  let toTrash = $state(false);
  let result = $state<Result | null>(null);
  let scanned = $state(false);

  let unlisten: (() => void) | null = null;
  onMount(async () => {
    unlisten = await onToolProgress((p) => { if (p.id === "sysclean") progress = p; });
    await scan();
  });
  onDestroy(() => unlisten?.());

  async function scan() {
    busy = "scan"; result = null; progress = null;
    try {
      rules = await invoke<Rule[]>("tool_clean_scan");
      selected = new Set(rules.filter((r) => r.risk === "safe").map((r) => r.id));
      scanned = true;
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; progress = null; }
  }
  function toggle(id: string) { const s = new Set(selected); if (s.has(id)) s.delete(id); else s.add(id); selected = s; }
  let total = $derived(rules.filter((r) => selected.has(r.id)).reduce((n, r) => n + r.bytes, 0));
  let grandTotal = $derived(rules.reduce((n, r) => n + r.bytes, 0));

  async function clean() {
    if (!selected.size || busy) return;
    busy = "clean"; progress = null;
    try {
      result = await invoke<Result>("tool_clean_run", { req: { ids: [...selected], to_trash: toTrash } });
      showToast(result.failed.length ? "info" : "success", `${fmtBytes(result.freed)} ${$t("tools.sysclean.freed")}`);
      await scan();
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; progress = null; }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{scanned ? `${fmtBytes(grandTotal)} ${$t("tools.sysclean.found")}` : $t("tools.sysclean.title")}</div>
          <div class="group-row-sub">{$t("tools.sysclean.hint")}</div>
          {#if busy}
            <div class="group-row-sub">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={scan}>{$t("tools.common.refresh")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.sysclean.to_trash")}</div><div class="group-row-sub">{$t("tools.sysclean.to_trash_hint")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={toTrash} type="button" role="switch" aria-checked={toTrash} aria-label={$t("tools.sysclean.to_trash")} onclick={() => (toTrash = !toTrash)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{fmtBytes(total)} <span class="dim">· {selected.size} {$t("tools.sysclean.selected")}</span></div></div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => (selected = new Set(rules.filter((r) => r.risk === "safe").map((r) => r.id)))}>{$t("tools.sysclean.select_safe")}</button>
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => (selected = new Set())}>{$t("tools.sysclean.select_none")}</button>
          <button class="btn btn-primary" type="button" disabled={busy !== null || !selected.size} onclick={clean}>{busy === "clean" ? $t("tools.common.working") : $t("tools.sysclean.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section><div class="group">
      <div class="group-row"><div class="group-row-content"><div class="group-row-title">{fmtBytes(result.freed)} {$t("tools.sysclean.freed")}</div><div class="group-row-sub">{result.removed} {$t("tools.sysclean.items_removed")}{#if result.failed.length} · {result.failed.length} {$t("tools.common.failed")}{/if}</div></div></div>
      {#each result.failed.slice(0, 8) as f (f)}<div class="group-row"><div class="group-row-sub mono">{f}</div></div>{/each}
    </div></section>
  {/if}

  {#each GROUPS as g (g)}
    {@const list = rules.filter((r) => r.group === g)}
    {#if list.length}
      <section>
        <h3 class="group-title">{$t(`tools.sysclean.group_${g}`)} <span class="dim">· {fmtBytes(list.reduce((n, r) => n + r.bytes, 0))}</span></h3>
        <div class="group">
          {#each list as r (r.id)}
            <label class="group-row rule" class:off={!selected.has(r.id)}>
              <input type="checkbox" checked={selected.has(r.id)} onchange={() => toggle(r.id)} disabled={busy !== null} />
              <div class="group-row-content">
                <div class="group-row-title">{r.name} {#if r.risk === "review"}<span class="tag tag-warning">{$t("tools.sysclean.review")}</span>{/if}</div>
                <div class="group-row-sub mono" title={r.paths.join("\n")}>{r.paths[0]}{r.paths.length > 1 ? ` (+${r.paths.length - 1})` : ""}</div>
              </div>
              <div class="group-row-trailing size"><strong>{fmtBytes(r.bytes)}</strong><span class="dim">{r.files} {$t("tools.common.files")}</span></div>
            </label>
          {/each}
        </div>
      </section>
    {/if}
  {/each}
  {#if scanned && !rules.length}
    <div class="tools-empty"><img class="empty-state-art" src="/emoji/sparkles.png" alt="" width="72" height="72" /><h2>{$t("tools.sysclean.nothing")}</h2></div>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .group-title { margin: 0 0 var(--space-2); font-size: var(--text-sm); font-weight: 600; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em; }
  .rule { cursor: pointer; gap: var(--space-3); }
  .rule.off .group-row-title { color: var(--text-muted); }
  .rule input { accent-color: var(--accent); }
  .size { display: flex; flex-direction: column; align-items: flex-end; gap: 2px; font-size: var(--text-sm); }
  .tools-empty { display: flex; flex-direction: column; align-items: center; gap: var(--space-2); padding: var(--space-6); text-align: center; }
  .tools-empty h2 { margin: 0; font-size: var(--text-base); color: var(--text-muted); }
</style>
