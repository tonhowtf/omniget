<script lang="ts">
  /** Renomear em massa com prévia (estudo 29, PowerRename). */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, pickFiles } from "$lib/tools/rt";

  type Plan = { from: string; to: string; changed: boolean; conflict: boolean };
  let files = $state<string[]>([]);
  let pattern = $state("");
  let replacement = $state("");
  let ci = $state(true);
  let kase = $state("");
  let start = $state(1);
  let applyExt = $state(false);
  let plans = $state<Plan[]>([]);
  let error = $state("");
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function preview() {
    if (!files.length) { plans = []; return; }
    try {
      plans = await invoke<Plan[]>("tool_rename_plan", { opts: { files, pattern, replacement, case_insensitive: ci, case: kase, counter_start: start, apply_to_extension: applyExt } });
      error = "";
    } catch (e) { error = errText(e); plans = []; }
  }
  function schedule() { if (timer) clearTimeout(timer); timer = setTimeout(preview, 150); }

  async function apply() {
    const todo = plans.filter((p) => p.changed && !p.conflict);
    if (!todo.length) return;
    try {
      const r = await invoke<{ renamed: number; failed: string[] }>("tool_rename_apply", { plans: todo });
      showToast(r.failed.length ? "info" : "success", `${r.renamed} ${$t("tools.rename.renamed")}`);
      files = plans.map((p) => (p.changed && !p.conflict ? p.to : p.from));
      await preview();
    } catch (e) { showToast("error", errText(e)); }
  }

  let changed = $derived(plans.filter((p) => p.changed && !p.conflict).length);
  let conflicts = $derived(plans.filter((p) => p.conflict).length);
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{files.length} {$t("tools.common.files")}</div></div>
        <div class="group-row-trailing btn-row">{#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => { files = []; plans = []; }}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles())]; await preview(); }}>{$t("tools.common.add")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.rename.find")}</div><div class="group-row-sub">{$t("tools.rename.find_hint")}</div></div>
        <div class="group-row-trailing"><input class="input mono" type="text" bind:value={pattern} oninput={schedule} placeholder="^Aula (\d+) - " /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.rename.replace")}</div><div class="group-row-sub">{$t("tools.rename.replace_hint")}</div></div>
        <div class="group-row-trailing"><input class="input mono" type="text" bind:value={replacement} oninput={schedule} placeholder="{'{n:2}'}. $1" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.rename.options")}</div></div>
        <div class="group-row-trailing btn-row">
          <label class="opt"><input class="checkbox" type="checkbox" bind:checked={ci} onchange={preview} /> Aa</label>
          <label class="opt"><input class="checkbox" type="checkbox" bind:checked={applyExt} onchange={preview} /> .ext</label>
          <select class="input" bind:value={kase} onchange={preview}><option value="">{$t("tools.rename.case_keep")}</option><option value="lower">lower</option><option value="upper">UPPER</option><option value="title">Title</option></select>
          <input class="input" type="number" min="0" bind:value={start} oninput={schedule} style:width="5em" title="{'{n}'} start" />
        </div>
      </div>
      {#if error}<div class="group-row"><div class="group-row-sub"><span class="tag tag-danger">{error}</span></div></div>{/if}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-sub">{changed} {$t("tools.rename.will_change")}{#if conflicts} · <span class="tag tag-warning">{conflicts} {$t("tools.rename.conflicts")}</span>{/if}</div></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={!changed} onclick={apply}>{$t("tools.rename.apply")}</button></div>
      </div>
    </div>
  </section>
  {#if plans.length}
    <section><div class="group">
      {#each plans as p (p.from)}
        <div class="group-row"><div class="group-row-content plan" class:changed={p.changed} class:conflict={p.conflict}><span class="mono from">{baseName(p.from)}</span><span class="arrow">→</span><span class="mono to">{baseName(p.to)}</span></div></div>
      {/each}
    </div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .opt { display: inline-flex; align-items: center; gap: 4px; font-size: var(--text-sm); }
  .plan { display: grid; grid-template-columns: 1fr auto 1fr; gap: var(--space-2); align-items: center; }
  .plan .to { color: var(--text-dim); }
  .plan.changed .to { color: var(--accent-hi); }
  .plan.conflict .to { color: var(--warning); }
  .arrow { color: var(--text-dim); }
</style>
