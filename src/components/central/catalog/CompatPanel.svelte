<script lang="ts">
  /**
   * "How it looks in each tool": compatibility per tool (computed by the
   * backend with the real converters, `agentkit_parse_preview`), and for the
   * chosen tool a preview plan (`agentkit_plan`, never applied): the file it
   * would get side by side with the original, what is lost, the commands it
   * would run and, for merged config files, the diff.
   */
  import { t } from "$lib/i18n";
  import DiffViewer from "$components/central/vcs/DiffViewer.svelte";
  import {
    agentkitPlan,
    decodeBase64Text,
    errText,
    filePlanToDiff,
    type Component,
    type Compat,
    type InstallPlan,
    type Scope,
    type TargetAdapter,
  } from "$lib/central/catalog";
  import CodeView from "./CodeView.svelte";
  import ToolDot from "./ToolDot.svelte";

  let {
    component,
    original = "",
    originalPath = "",
    targets = [],
    detected = [],
    projectDir = null,
  }: {
    component: Component;
    original?: string;
    originalPath?: string;
    targets?: TargetAdapter[];
    detected?: string[];
    projectDir?: string | null;
  } = $props();

  let onlyDetected = $state(true);
  let selected = $state<string | null>(null);
  let plans = $state<Record<string, InstallPlan | { error: string }>>({});
  let loading = $state<string | null>(null);
  let fileIdx = $state(0);

  let compat = $derived<Record<string, Compat>>(component.compat ?? {});
  let rank = (s: string | undefined) => ["native", "converted", "degraded", "unsupported"].indexOf(s ?? "unsupported");
  let list = $derived(
    [...targets]
      .filter((x) => !onlyDetected || detected.includes(x.id) || !detected.length)
      .sort(
        (a, b) =>
          (detected.includes(a.id) ? 0 : 1) - (detected.includes(b.id) ? 0 : 1) ||
          rank(compat[a.id]?.status) - rank(compat[b.id]?.status) ||
          a.name.localeCompare(b.name),
      ),
  );
  let summary = $derived.by(() => {
    const c: Record<string, number> = { native: 0, converted: 0, degraded: 0, unsupported: 0 };
    for (const x of targets) {
      const s = compat[x.id]?.status;
      if (s) c[s]++;
    }
    return c;
  });

  $effect(() => {
    if (!selected && list.length) {
      const first = list.find((x) => compat[x.id] && compat[x.id].status !== "unsupported") ?? list[0];
      selected = first.id;
    }
  });

  let scope = $derived<Scope>(projectDir ? "project" : "global");

  async function preview(tid: string) {
    const key = `${tid}|${scope}|${projectDir ?? ""}`;
    if (plans[key]) return;
    loading = tid;
    try {
      const p = await agentkitPlan({
        components: [component],
        targets: [tid],
        scope,
        projectDir,
        policy: "rename",
      });
      plans = { ...plans, [key]: p };
    } catch (e) {
      plans = { ...plans, [key]: { error: errText(e) } };
    } finally {
      loading = null;
    }
  }

  $effect(() => {
    const s = selected;
    if (s && compat[s]?.status !== "unsupported") {
      fileIdx = 0;
      void preview(s);
    }
  });

  let current = $derived(selected ? plans[`${selected}|${scope}|${projectDir ?? ""}`] : undefined);
  let plan = $derived(current && !("error" in current) ? (current as InstallPlan) : null);
  let unit = $derived(plan?.units[0] ?? null);
  let unitFiles = $derived(plan ? plan.units.flatMap((u) => u.files) : []);
  let file = $derived(unitFiles[fileIdx] ?? null);
  let converted = $derived(file?.content ? decodeBase64Text(file.content) : "");
  let mergeDiff = $derived(file && !file.content ? plan?.files.find((f) => f.path === file.path) : null);
  let selectedTarget = $derived(targets.find((x) => x.id === selected) ?? null);
</script>

<div class="compat">
  <aside class="tools">
    <div class="sum">
      <span class="s native">{summary.native}</span>
      <span class="s converted">{summary.converted}</span>
      <span class="s degraded">{summary.degraded}</span>
      <span class="s unsupported">{summary.unsupported}</span>
    </div>
    <label class="only">
      <input type="checkbox" bind:checked={onlyDetected} />
      {$t("llm.central.catalog.compat.only_detected")}
    </label>
    <ul role="listbox" aria-label={$t("llm.central.catalog.compat.tools")}>
      {#each list as tg (tg.id)}
        <li>
          <button type="button" role="option" aria-selected={selected === tg.id} class:on={selected === tg.id} onclick={() => (selected = tg.id)}>
            <ToolDot id={tg.id} name={tg.name} compat={compat[tg.id] ?? null} size={20} detected={detected.includes(tg.id)} />
            <span class="nm">{tg.name}</span>
            <span class="st {compat[tg.id]?.status ?? 'unknown'}">
              {compat[tg.id] ? $t(`llm.central.agentkit.compat.${compat[tg.id].status}`) : "—"}
            </span>
          </button>
        </li>
      {/each}
    </ul>
  </aside>

  <section class="view">
    {#if !selected || !selectedTarget}
      <p class="muted">{$t("llm.central.catalog.compat.pick")}</p>
    {:else}
      {@const c = compat[selected]}
      <header class="vh">
        <h3>{selectedTarget.name}{#if selectedTarget.beta}<span class="beta" title={$t("llm.central.agentkit.beta")}>β</span>{/if}</h3>
        <span class="muted small">{$t(`llm.central.catalog.scope.${scope}`)}{projectDir ? ` · ${projectDir}` : ""}</span>
      </header>
      {#if c?.status === "unsupported"}
        <p class="warn">{c.reason}</p>
      {:else if loading === selected && !current}
        <p class="muted">{$t("llm.central.catalog.compat.converting")}</p>
      {:else if current && "error" in current}
        <p class="err">{current.error}</p>
      {:else if plan}
        {#if c?.status === "degraded" || unit?.losses.length}
          <div class="lost">
            <strong>{$t("llm.central.catalog.compat.lost")}</strong>
            <ul>
              {#each [...new Set([...(c?.status === "degraded" ? c.lost : []), ...plan.units.flatMap((u) => u.losses)])] as l (l)}<li>{l}</li>{/each}
            </ul>
          </div>
        {/if}
        {#if plan.units.some((u) => u.commands.length)}
          <div class="cmds">
            <strong>{$t("llm.central.catalog.plan.commands")}</strong>
            {#each [...new Set(plan.units.flatMap((u) => u.commands))] as cmd (cmd)}<code>{cmd}</code>{/each}
          </div>
        {/if}
        {#if plan.units.some((u) => u.notes.length)}
          <p class="muted small">{plan.units.flatMap((u) => u.notes).join(" · ")}</p>
        {/if}
        {#if unit?.error}<p class="err">{unit.error}</p>{/if}

        {#if unitFiles.length > 1}
          <div class="files" role="tablist">
            {#each unitFiles as f, i (i)}
              <button type="button" role="tab" aria-selected={i === fileIdx} onclick={() => (fileIdx = i)} title={f.path}>{f.label || f.path.split(/[\\/]/).pop()}</button>
            {/each}
          </div>
        {/if}

        {#if file}
          <p class="path mono" title={file.path}>{file.path}{file.executable ? " (+x)" : ""}</p>
          {#if file.content}
            <div class="side-by-side">
              <div>
                <p class="cap">{$t("llm.central.catalog.compat.original")} <span class="mono">{originalPath}</span></p>
                <CodeView text={original} />
              </div>
              <div>
                <p class="cap">{$t("llm.central.catalog.compat.converted_file", { tool: selectedTarget.name })}</p>
                <CodeView text={converted} />
              </div>
            </div>
          {:else if mergeDiff}
            <p class="cap">{$t("llm.central.catalog.compat.merged", { format: file.format ?? "" })}</p>
            <DiffViewer files={[filePlanToDiff(mergeDiff)]} height="40vh" toolbar={false} />
          {:else}
            <p class="muted small">{$t("llm.central.catalog.compat.no_change")}</p>
          {/if}
        {:else}
          <p class="muted small">{$t("llm.central.catalog.compat.no_files")}</p>
        {/if}
      {/if}
    {/if}
  </section>
</div>

<style>
  .compat {
    display: grid;
    grid-template-columns: 240px minmax(0, 1fr);
    gap: var(--space-4);
  }
  .tools {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .sum {
    display: flex;
    gap: 4px;
  }
  .s {
    flex: 1;
    text-align: center;
    padding: 2px 0;
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
    font-weight: 600;
  }
  .s.native {
    background: color-mix(in srgb, var(--success) 18%, transparent);
    color: var(--success);
  }
  .s.converted {
    background: color-mix(in srgb, var(--blue) 18%, transparent);
    color: var(--blue);
  }
  .s.degraded {
    background: color-mix(in srgb, var(--warning) 18%, transparent);
    color: var(--warning);
  }
  .s.unsupported {
    background: var(--fill-1);
    color: var(--text-muted);
  }
  .only {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .only input {
    accent-color: var(--accent);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 60vh;
    overflow: auto;
  }
  li button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: 4px var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text);
    font-size: var(--text-sm);
    text-align: left;
    cursor: pointer;
  }
  li button:hover {
    background: var(--fill-1);
  }
  li button.on {
    background: var(--accent-soft);
  }
  .nm {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .st {
    font-size: var(--text-xs);
    color: var(--text-faint);
  }
  .st.native {
    color: var(--success);
  }
  .st.converted {
    color: var(--blue);
  }
  .st.degraded {
    color: var(--warning);
  }
  .view {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }
  .vh {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
  }
  h3 {
    margin: 0;
    font-size: var(--text-lg);
  }
  .beta {
    margin-left: 6px;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: var(--text-sm);
    margin: 0;
  }
  .warn {
    color: var(--warning);
  }
  .err {
    color: var(--error);
  }
  .lost {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--warning) 10%, transparent);
    font-size: var(--text-sm);
  }
  .lost ul {
    max-height: none;
    margin: 4px 0 0;
    padding-left: var(--space-4);
    list-style: disc;
  }
  .cmds {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--warning) 10%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--warning) 40%, transparent);
    font-size: var(--text-sm);
  }
  .cmds code {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    word-break: break-all;
  }
  .files {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .files button {
    height: 24px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text-muted);
    font-size: var(--text-xs);
    cursor: pointer;
  }
  .files button[aria-selected="true"] {
    background: var(--accent);
    color: var(--on-accent);
  }
  .path {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
    word-break: break-all;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .cap {
    margin: 0 0 4px;
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
  }
  .side-by-side {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
  }
  .side-by-side :global(.code) {
    max-height: 56vh;
  }
  @media (max-width: 1000px) {
    .compat,
    .side-by-side {
      grid-template-columns: 1fr;
    }
  }
</style>
