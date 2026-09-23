<script lang="ts">
  /**
   * The stack drawer: items, target tools (default: tools detected for the
   * chosen project), scope, conflict policy; "See plan" (diff dialog),
   * "Install" (plan + apply, result with undo), save as collection, export
   * `.omnistack`, copy the equivalent `omniget agentkit install …` line.
   */
  import { save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    agentkitDetect,
    agentkitPlan,
    cliCommand,
    collections as coll,
    errText,
    type DetectedTarget,
    type InstallPlan,
    type Policy,
    type Scope,
    type TargetAdapter,
  } from "$lib/central/catalog";
  import { stack } from "$lib/central/stack-store.svelte";
  import KindIcon from "./KindIcon.svelte";
  import PlanDialog from "./PlanDialog.svelte";
  import ProjectPicker from "./ProjectPicker.svelte";

  let {
    targets = [],
    oncollections,
  }: {
    targets?: TargetAdapter[];
    /** Collections changed (saved/exported): the sidebar reloads. */
    oncollections?: () => void;
  } = $props();

  let detected = $state<DetectedTarget[]>([]);
  let detecting = $state(false);
  let plan = $state<InstallPlan | null>(null);
  let autoApply = $state(false);
  let planning = $state(false);
  let confirmClear = $state(false);
  let saving = $state(false);
  let savedId = $state<string | null>(null);
  let panel = $state<HTMLElement | null>(null);

  let installedIds = $derived(detected.filter((d) => d.installed).map((d) => d.id));
  let inProject = $derived(detected.filter((d) => d.project_markers.length > 0).map((d) => d.id));
  let autoTargets = $derived.by(() => {
    if (stack.scope !== "global" && inProject.length) return inProject;
    return detected.filter((d) => d.installed && d.enabled_by_default).map((d) => d.id);
  });
  let ordered = $derived(
    [...targets].sort((a, b) => {
      const r = (id: string) => (inProject.includes(id) ? 0 : installedIds.includes(id) ? 1 : 2);
      return r(a.id) - r(b.id) || a.tier - b.tier || a.name.localeCompare(b.name);
    }),
  );
  let showAll = $state(false);
  let visibleTargets = $derived(
    showAll ? ordered : ordered.filter((t) => installedIds.includes(t.id) || stack.targets.includes(t.id)),
  );
  let catalogIds = $derived(stack.items.filter((i) => !i.component).map((i) => i.id));
  let canPlan = $derived(stack.items.length > 0 && stack.targets.length > 0 && (stack.scope === "global" || !!stack.projectDir));
  let command = $derived(
    cliCommand({
      ids: catalogIds,
      targets: stack.targets,
      scope: stack.scope,
      projectDir: stack.projectDir,
      policy: stack.policy,
    }),
  );

  async function detect(fresh = false) {
    detecting = true;
    try {
      detected = await agentkitDetect(stack.scope === "global" ? null : stack.projectDir, fresh);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      detecting = false;
    }
  }

  $effect(() => {
    if (stack.open) {
      void stack.projectDir;
      void stack.scope;
      void detect();
    }
  });

  $effect(() => {
    if (stack.targetsAuto && detected.length) {
      const next = autoTargets;
      if (next.join(",") !== stack.targets.join(",")) stack.setTargets(next, true);
    }
  });

  $effect(() => {
    if (stack.open) panel?.focus();
  });

  async function makePlan(apply: boolean) {
    if (!canPlan) return;
    planning = true;
    try {
      plan = await agentkitPlan({
        catalogIds,
        components: stack.inlineComponents(),
        targets: stack.targets,
        scope: stack.scope,
        projectDir: stack.scope === "global" ? null : stack.projectDir,
        policy: stack.policy,
      });
      autoApply = apply;
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      planning = false;
    }
  }

  async function ensureCollection(): Promise<string | null> {
    if (!catalogIds.length) {
      showToast("error", $t("llm.central.catalog.stack.nothing_to_save"));
      return null;
    }
    const name = stack.name.trim() || `Stack ${new Date().toLocaleDateString()}`;
    saving = true;
    try {
      let id = savedId;
      if (id) {
        const list = await coll.list();
        if (!list.some((c) => c.id === id)) id = null;
      }
      if (!id) {
        const c = await coll.create(name, null, stack.targets, stack.scope);
        id = c.id;
      } else {
        await coll.rename(id, name);
        await coll.setDefaults(id, stack.targets, stack.scope);
      }
      const current = (await coll.list()).find((c) => c.id === id);
      const have = new Set(current?.items.map((i) => i.item_id) ?? []);
      for (const item of catalogIds) if (!have.has(item)) await coll.add(id, item);
      for (const old of have) if (!catalogIds.includes(old)) await coll.remove(id, old);
      savedId = id;
      if (!stack.name.trim()) stack.set("name", name);
      oncollections?.();
      return id;
    } catch (e) {
      showToast("error", errText(e));
      return null;
    } finally {
      saving = false;
    }
  }

  async function saveCollection() {
    const id = await ensureCollection();
    if (id) showToast("success", $t("llm.central.catalog.stack.saved"));
  }

  async function exportStack() {
    const id = await ensureCollection();
    if (!id) return;
    try {
      const file = (stack.name.trim() || "stack").replace(/[^\w.-]+/g, "-");
      const path = await saveDialog({
        defaultPath: `${file}.omnistack`,
        filters: [{ name: "OmniGet stack", extensions: ["omnistack"] }],
      });
      if (!path) return;
      const out = await coll.export(id, path);
      showToast("success", $t("llm.central.catalog.stack.exported", { path: out }));
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function copyCommand() {
    try {
      await navigator.clipboard.writeText(command);
      showToast("success", $t("llm.central.catalog.copied"));
    } catch {
      showToast("error", $t("llm.central.catalog.copy_failed"));
    }
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape" && stack.open && !plan) stack.open = false;
  }

  const SCOPES: Scope[] = ["project", "local", "global"];
  const POLICIES: Policy[] = ["rename", "skip", "overwrite"];
</script>

<svelte:window onkeydown={onkey} />

{#if stack.open}
  <div class="scrim" role="presentation" onclick={() => (stack.open = false)}></div>
  <aside class="drawer" aria-labelledby="stack-title" tabindex="-1" bind:this={panel}>
    <header class="head">
      <h2 id="stack-title">{$t("llm.central.catalog.stack.title")} <span class="count tabular">{stack.count}</span></h2>
      <button type="button" class="x" onclick={() => (stack.open = false)} aria-label={$t("llm.central.catalog.close")}>✕</button>
    </header>

    <div class="body">
      <input
        class="name"
        value={stack.name}
        oninput={(e) => stack.set("name", (e.currentTarget as HTMLInputElement).value)}
        placeholder={$t("llm.central.catalog.stack.name")}
        aria-label={$t("llm.central.catalog.stack.name")}
      />

      {#if stack.items.length === 0}
        <p class="empty">{$t("llm.central.catalog.stack.empty")}</p>
      {:else}
        <ul class="items">
          {#each stack.items as it, i (it.id)}
            <li>
              <KindIcon kind={it.kind} size={22} />
              <span class="nm" title={it.id}>{it.name}</span>
              {#if it.component}<span class="tag">{$t("llm.central.catalog.stack.inline")}</span>{/if}
              <span class="acts">
                <button type="button" onclick={() => stack.move(it.id, -1)} disabled={i === 0} aria-label={$t("llm.central.catalog.move_up")}>↑</button>
                <button type="button" onclick={() => stack.move(it.id, 1)} disabled={i === stack.items.length - 1} aria-label={$t("llm.central.catalog.move_down")}>↓</button>
                <button type="button" onclick={() => stack.remove(it.id)} aria-label={$t("llm.central.catalog.stack.remove")}>✕</button>
              </span>
            </li>
          {/each}
        </ul>
        <div class="row-end">
          {#if confirmClear}
            <button type="button" class="link danger" onclick={() => { stack.clear(); confirmClear = false; savedId = null; }}>{$t("llm.central.catalog.stack.clear_confirm")}</button>
            <button type="button" class="link" onclick={() => (confirmClear = false)}>{$t("llm.central.catalog.cancel")}</button>
          {:else}
            <button type="button" class="link" onclick={() => (confirmClear = true)}>{$t("llm.central.catalog.stack.clear")}</button>
          {/if}
        </div>
      {/if}

      <section>
        <h3>{$t("llm.central.catalog.stack.where")}</h3>
        <div class="seg" role="radiogroup" aria-label={$t("llm.central.catalog.stack.scope")}>
          {#each SCOPES as s (s)}
            <button type="button" role="radio" aria-checked={stack.scope === s} onclick={() => stack.set("scope", s)}>{$t(`llm.central.catalog.scope.${s}`)}</button>
          {/each}
        </div>
        <p class="hint">{$t(`llm.central.catalog.scope.${stack.scope}_hint`)}</p>
        {#if stack.scope !== "global"}
          <ProjectPicker value={stack.projectDir} onchange={(v) => stack.set("projectDir", v)} />
        {/if}
      </section>

      <section>
        <div class="h3row">
          <h3>{$t("llm.central.catalog.stack.targets")}</h3>
          <label class="auto">
            <input type="checkbox" checked={stack.targetsAuto} onchange={(e) => stack.setTargets(autoTargets, (e.currentTarget as HTMLInputElement).checked)} />
            {$t("llm.central.catalog.stack.auto_targets")}
          </label>
        </div>
        {#if detecting}<p class="hint">{$t("llm.central.catalog.detecting")}</p>{/if}
        <div class="chips">
          {#each visibleTargets as tg (tg.id)}
            <button
              type="button"
              class="chip"
              class:on={stack.targets.includes(tg.id)}
              aria-pressed={stack.targets.includes(tg.id)}
              onclick={() => stack.toggleTarget(tg.id)}
              title={tg.beta ? $t("llm.central.agentkit.beta") : tg.name}
            >
              {tg.name}
              {#if inProject.includes(tg.id)}<span class="mark">●</span>{/if}
              {#if tg.beta}<span class="beta">β</span>{/if}
            </button>
          {/each}
          <button type="button" class="chip more" onclick={() => (showAll = !showAll)}>
            {showAll ? $t("llm.central.catalog.show_less") : $t("llm.central.catalog.stack.all_tools", { count: String(targets.length) })}
          </button>
        </div>
        <p class="hint">{$t("llm.central.catalog.stack.targets_hint")}</p>
      </section>

      <section>
        <h3>{$t("llm.central.catalog.stack.policy")}</h3>
        <select value={stack.policy} onchange={(e) => stack.set("policy", (e.currentTarget as HTMLSelectElement).value as Policy)} aria-label={$t("llm.central.catalog.stack.policy")}>
          {#each POLICIES as p (p)}<option value={p}>{$t(`llm.central.agentkit.policy.${p}`)}</option>{/each}
        </select>
      </section>

      {#if catalogIds.length}
        <section>
          <h3>{$t("llm.central.catalog.stack.cli")}</h3>
          <div class="cli">
            <code>{command}</code>
            <button type="button" class="link" onclick={copyCommand}>{$t("llm.central.catalog.copy")}</button>
          </div>
        </section>
      {/if}
    </div>

    <footer class="foot">
      <div class="secondary">
        <button type="button" class="btn" onclick={saveCollection} disabled={saving || !catalogIds.length}>{$t("llm.central.catalog.stack.save")}</button>
        <button type="button" class="btn" onclick={exportStack} disabled={saving || !catalogIds.length}>{$t("llm.central.catalog.stack.export")}</button>
      </div>
      <div class="primary-row">
        <button type="button" class="btn" onclick={() => makePlan(false)} disabled={!canPlan || planning}>{$t("llm.central.catalog.stack.see_plan")}</button>
        <button type="button" class="btn primary" onclick={() => makePlan(true)} disabled={!canPlan || planning}>
          {planning ? $t("llm.central.catalog.planning") : $t("llm.central.catalog.stack.install")}
        </button>
      </div>
      {#if !canPlan && stack.items.length}
        <p class="hint">{stack.targets.length ? $t("llm.central.catalog.stack.need_project") : $t("llm.central.catalog.stack.need_targets")}</p>
      {/if}
    </footer>
  </aside>
{/if}

{#if plan}
  <PlanDialog
    {plan}
    {autoApply}
    onclose={() => (plan = null)}
    onreplan={() => {
      const apply = autoApply;
      plan = null;
      void makePlan(apply);
    }}
  />
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: color-mix(in srgb, var(--dialog-backdrop) 50%, transparent);
  }
  .drawer {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 41;
    width: min(420px, 96vw);
    display: flex;
    flex-direction: column;
    background: var(--popup-bg);
    box-shadow: var(--elev-3);
    outline: none;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4) var(--space-4) var(--space-2);
  }
  h2 {
    margin: 0;
    font-size: var(--text-lg);
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .count {
    padding: 0 8px;
    border-radius: var(--radius-full);
    background: var(--accent);
    color: var(--on-accent);
    font-size: var(--text-sm);
  }
  h3 {
    margin: 0 0 var(--space-1);
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
  }
  .h3row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .x {
    border: none;
    background: var(--fill-1);
    width: 28px;
    height: 28px;
    border-radius: var(--radius-full);
    color: var(--text);
    cursor: pointer;
  }
  .body {
    flex: 1;
    overflow: auto;
    padding: 0 var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .name,
  select {
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font-size: var(--text-base);
    width: 100%;
  }
  .empty,
  .hint {
    margin: 4px 0 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .items {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .items li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px var(--space-1);
    border-radius: var(--radius-sm);
  }
  .items li:hover {
    background: var(--fill-1);
  }
  .nm {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-base);
  }
  .tag {
    font-size: var(--text-caption);
    padding: 0 4px;
    border-radius: var(--radius-xs);
    background: var(--fill-2);
    color: var(--text-muted);
  }
  .acts {
    display: inline-flex;
    gap: 2px;
  }
  .acts button {
    width: 22px;
    height: 22px;
    border: none;
    border-radius: var(--radius-xs);
    background: none;
    color: var(--text-muted);
    cursor: pointer;
  }
  .acts button:hover:not(:disabled) {
    background: var(--fill-2);
    color: var(--text);
  }
  .acts button:disabled {
    opacity: 0.3;
  }
  .row-end {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
  .link {
    border: none;
    background: none;
    color: var(--accent-hi);
    font-size: var(--text-sm);
    cursor: pointer;
    padding: 0;
  }
  .link.danger {
    color: var(--error);
  }
  .seg {
    display: flex;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .seg button {
    flex: 1;
    height: 26px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .seg button[aria-checked="true"] {
    background: var(--surface-hi);
    color: var(--text);
    box-shadow: var(--elev-1);
  }
  section :global(.picker) {
    margin-top: var(--space-2);
  }
  .auto {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 26px;
    padding: 0 10px;
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .chip.on {
    background: var(--accent);
    color: var(--on-accent);
  }
  .chip.more {
    background: none;
    color: var(--accent-hi);
  }
  .mark {
    font-size: 8px;
    color: var(--success);
  }
  .chip.on .mark {
    color: inherit;
  }
  .beta {
    font-size: var(--text-xs);
    opacity: 0.7;
  }
  .cli {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
  }
  .cli code {
    flex: 1;
    min-width: 0;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--surface-mut);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    word-break: break-all;
  }
  .foot {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4) var(--space-4);
    border-top: 1px solid var(--separator);
  }
  .secondary,
  .primary-row {
    display: flex;
    gap: var(--space-2);
  }
  .secondary .btn,
  .primary-row .btn {
    flex: 1;
  }
  .btn {
    height: var(--control-h-lg);
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-base);
    font-weight: 500;
    cursor: pointer;
  }
  .btn:hover:not(:disabled) {
    background: var(--fill-2);
  }
  .btn.primary {
    background: var(--cta);
    color: var(--on-cta);
  }
  .btn.primary:hover:not(:disabled) {
    background: var(--cta-hover);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
