<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import type { CanvasNode } from "$lib/canvas/model/types";
  import {
    errorText,
    formatCredits,
    generateBudget,
    generateModels,
    generateQuote,
    generateReconcile,
    generateRelease,
    generateSchema,
    generateSubmit,
    onGenerateProgress,
    type BudgetView,
    type GenerationState,
    type ModelSchema,
    type ModelSummary,
    type ParamSpec,
    type QuoteView,
    type Version,
  } from "$lib/canvas/generate/client";

  let {
    node,
    workspaceId = "",
    onpatch,
  }: {
    node: CanvasNode;
    selected?: boolean;
    focused?: boolean;
    zoom?: number;
    workspaceId?: string;
    onpatch?: (patch: Partial<CanvasNode>) => void;
  } = $props();

  const engine = useEngine();

  let models = $state<ModelSummary[]>([]);
  let modelsError = $state("");
  let schema = $state<ModelSchema | null>(null);
  let budget = $state<BudgetView | null>(null);
  let busy = $state<"" | "quote" | "submit" | "reconcile" | "release" | "budget">("");
  let localError = $state("");
  let now = $state(Date.now());
  let editingBudget = $state(false);
  let budgetDraft = $state("");
  let promptDraft = $state("");
  let promptTimer: ReturnType<typeof setTimeout> | null = null;
  let tick: ReturnType<typeof setInterval> | null = null;
  let liveStatus = $state("");
  let promptFocused = $state(false);

  const data = $derived(node.data as Record<string, unknown>);
  const model = $derived(String(data.model ?? ""));
  const prompt = $derived(String(data.prompt ?? ""));
  const params = $derived((data.params as Record<string, unknown> | undefined) ?? {});
  const gstate = $derived(String(data.state ?? "draft") as GenerationState);
  const quote = $derived((data.quote as QuoteView | null | undefined) ?? null);
  const versions = $derived(((data.versions as Version[] | undefined) ?? []).slice());
  const latest = $derived(versions.length ? versions[versions.length - 1] : null);
  const usedIn = $derived((data.usedIn as { text?: string; nodeId?: string } | null | undefined) ?? null);
  const usedInNode = $derived.by(() => {
    void engine.store.version;
    return usedIn?.nodeId ? engine.store.node(usedIn.nodeId) : undefined;
  });
  const nodeError = $derived(typeof data.error === "string" ? data.error : "");
  const progress = $derived((data.progress as { status?: string; startedAt?: number } | null | undefined) ?? null);
  const quoteLive = $derived(!!quote && quote.expiresAt > now);
  const editable = $derived((schema?.params ?? []).filter((p) => p.editable));

  const references = $derived.by(() => {
    void engine.store.version;
    const results = new Set(versions.map((v) => v.nodeId));
    if (usedIn?.nodeId) results.add(usedIn.nodeId);
    const seen = new Set<string>();
    const out: Array<{ id: string; title: string; src: string }> = [];
    for (const c of engine.store.connections) {
      if (c.from !== node.id && c.to !== node.id) continue;
      const other = c.from === node.id ? c.to : c.from;
      if (results.has(other) || seen.has(other)) continue;
      const n = engine.store.node(other);
      if (!n) continue;
      const isImage = n.kind === "image" || (n.kind === "file" && n.data.isImage === true);
      if (!isImage) continue;
      seen.add(other);
      out.push({ id: n.id, title: n.title ?? "", src: srcOf(n.data.path) });
    }
    return out;
  });

  const elapsed = $derived(progress?.startedAt ? Math.max(0, Math.round((now - progress.startedAt) / 1000)) : 0);

  function srcOf(p: unknown): string {
    if (typeof p !== "string" || !p) return "";
    try {
      return convertFileSrc(p);
    } catch {
      return "";
    }
  }

  function patchData(d: Record<string, unknown>, invalidate = true) {
    const extra: Record<string, unknown> = {};
    if (invalidate && (gstate === "quoted" || gstate === "awaiting_approval")) {
      extra.quote = null;
      extra.state = versions.length ? "done" : "draft";
    }
    onpatch?.({ data: { ...d, ...extra } } as Partial<CanvasNode>);
  }

  async function loadSchema(m: string) {
    if (!m) {
      schema = null;
      return;
    }
    try {
      schema = await generateSchema(m);
    } catch (e) {
      schema = null;
      localError = errorText(e);
    }
  }

  async function loadBudget() {
    if (!workspaceId) return;
    try {
      budget = await generateBudget(workspaceId);
    } catch {
      budget = null;
    }
  }

  onMount(() => {
    promptDraft = prompt;
    void generateModels()
      .then((m) => (models = m))
      .catch((e) => (modelsError = errorText(e)));
    void loadBudget();
  });

  $effect(() => {
    void loadSchema(model);
  });

  $effect(() => {
    const p = prompt;
    if (!promptFocused && !promptTimer) promptDraft = p;
  });

  $effect(() => {
    const ticking = gstate === "running" || gstate === "quoted" || gstate === "awaiting_approval";
    if (ticking && !tick) {
      tick = setInterval(() => (now = Date.now()), 1000);
    } else if (!ticking && tick) {
      clearInterval(tick);
      tick = null;
    }
  });

  const off = onGenerateProgress((p) => {
    if (p.nodeId !== node.id) return;
    if (p.status) liveStatus = p.status;
    if (p.state === "done" || p.state === "failed" || p.state === "uncertain") {
      liveStatus = "";
      void loadBudget();
    }
  });

  onDestroy(() => {
    void off.then((f) => f());
    if (tick) clearInterval(tick);
    if (promptTimer) clearTimeout(promptTimer);
  });

  function setModel(m: string) {
    patchData({ model: m, modelName: models.find((x) => x.model === m)?.name ?? m, params: {} });
  }

  function onPrompt(v: string) {
    promptDraft = v;
    if (promptTimer) clearTimeout(promptTimer);
    promptTimer = setTimeout(() => {
      promptTimer = null;
      patchData({ prompt: promptDraft });
    }, 350);
  }

  function flushPrompt() {
    if (!promptTimer) return;
    clearTimeout(promptTimer);
    promptTimer = null;
    patchData({ prompt: promptDraft });
  }

  function setParam(p: ParamSpec, raw: unknown) {
    const next = { ...params };
    let v: unknown = raw;
    if (p.type.includes("integer") || p.type.includes("number")) {
      const s = String(raw ?? "").trim();
      v = s === "" ? null : Number(s);
      if (typeof v === "number" && !Number.isFinite(v)) v = null;
    }
    if (v === null || v === undefined || v === "" || v === p.default) delete next[p.name];
    else next[p.name] = v;
    patchData({ params: next });
  }

  function paramValue(p: ParamSpec): unknown {
    return params[p.name] ?? p.default ?? "";
  }

  function setUsedIn(text: string) {
    const v = text.trim();
    const next = { ...(usedIn ?? {}), text: v };
    patchData({ usedIn: v || next.nodeId ? next : null }, false);
  }

  async function run<T>(what: typeof busy, f: () => Promise<T>): Promise<T | undefined> {
    busy = what;
    localError = "";
    try {
      return await f();
    } catch (e) {
      localError = errorText(e);
      return undefined;
    } finally {
      busy = "";
    }
  }

  async function price() {
    flushPrompt();
    await new Promise((r) => setTimeout(r, 60));
    await run("quote", () => generateQuote(node.id));
    void loadBudget();
  }

  async function go() {
    const q = quote;
    if (!q || q.expiresAt <= Date.now() || !q.fitsBudget) return;
    await run("submit", () => generateSubmit(node.id, q.quoteId));
    void loadBudget();
  }

  async function saveBudget() {
    const n = Number(budgetDraft.replace(",", "."));
    if (!Number.isFinite(n) || n < 0) return;
    const r = await run("budget", () => generateBudget(workspaceId, { ceiling: n }));
    if (r) {
      budget = r;
      editingBudget = false;
    }
  }

  function showNode(id: string) {
    engine.selection.set([id]);
    engine.reveal(id);
    engine.invalidate();
  }

  function stateLabel(s: GenerationState): string {
    const fallback: Record<GenerationState, string> = {
      draft: "Draft",
      quoted: "Priced",
      awaiting_approval: "Needs approval",
      running: "Generating",
      done: "Done",
      uncertain: "Uncertain",
      failed: "Failed",
    };
    return $t(`canvas.generate.state.${s}`, { default: fallback[s] });
  }

  function statusLabel(s: string): string {
    const known: Record<string, string> = { submitting: "Sending", queued: "Queued", running: "Rendering", completed: "Saving" };
    return known[s] ? $t(`canvas.generate.status.${s}`, { default: known[s] }) : s;
  }

  const shownError = $derived(localError || (gstate === "failed" || gstate === "uncertain" || gstate === "draft" ? nodeError : ""));
</script>

<div class="gen" data-gen-state={gstate}>
  <div class="top">
    <label class="model">
      <span class="label">{$t("canvas.generate.model", { default: "Model" })}</span>
      {#if modelsError}
        <span class="warn">{$t("canvas.generate.cliMissing", { default: "Higgsfield CLI not found. Install it and run higgsfield auth login." })}</span>
      {:else}
        <select value={model} onchange={(e) => setModel((e.currentTarget as HTMLSelectElement).value)} disabled={gstate === "running"}>
          {#if !model}<option value="" disabled>{models.length ? "—" : $t("canvas.generate.modelLoading", { default: "Loading models…" })}</option>{/if}
          {#if model && !models.some((m) => m.model === model)}<option value={model}>{String(data.modelName ?? model)}</option>{/if}
          {#each models as m (m.model)}
            <option value={m.model}>{m.name}</option>
          {/each}
        </select>
      {/if}
    </label>
    <span class="chip state-{gstate}">{stateLabel(gstate)}</span>
    {#if gstate === "done" && latest && latest.credits != null}
      <span class="spent">{$t("canvas.generate.spent", { default: "Spent {{credits}} credits", credits: formatCredits(latest.credits) })}</span>
    {/if}
  </div>

  <label class="field">
    <span class="label">{$t("canvas.generate.prompt", { default: "Prompt" })}</span>
    <textarea
      rows="3"
      value={promptDraft}
      placeholder={$t("canvas.generate.promptPlaceholder", { default: "Describe the image…" })}
      oninput={(e) => onPrompt((e.currentTarget as HTMLTextAreaElement).value)}
      onfocus={() => (promptFocused = true)}
      onblur={() => {
        promptFocused = false;
        flushPrompt();
      }}
      disabled={gstate === "running"}
    ></textarea>
  </label>

  {#if editable.length}
    <div class="params">
      {#each editable as p (p.name)}
        <label class="param">
          <span class="pname">{p.name.replace(/_/g, " ")}</span>
          {#if p.options?.length}
            <select value={String(paramValue(p))} onchange={(e) => setParam(p, (e.currentTarget as HTMLSelectElement).value)} disabled={gstate === "running"}>
              {#each p.options as o (o)}<option value={o}>{o}</option>{/each}
            </select>
          {:else if p.type.startsWith("boolean")}
            <input type="checkbox" checked={paramValue(p) === true} onchange={(e) => setParam(p, (e.currentTarget as HTMLInputElement).checked)} disabled={gstate === "running"} />
          {:else if p.type.includes("number") || p.type.includes("integer")}
            <input type="number" value={paramValue(p) === "" ? "" : String(paramValue(p))} onchange={(e) => setParam(p, (e.currentTarget as HTMLInputElement).value)} disabled={gstate === "running"} />
          {:else}
            <input type="text" value={String(paramValue(p) ?? "")} onchange={(e) => setParam(p, (e.currentTarget as HTMLInputElement).value)} disabled={gstate === "running"} />
          {/if}
        </label>
      {/each}
    </div>
  {/if}

  <div class="refs">
    <span class="label">{$t("canvas.generate.references", { default: "References" })}</span>
    {#if references.length}
      <div class="thumbs">
        {#each references as r (r.id)}
          <button type="button" class="thumb" title={r.title} onclick={() => showNode(r.id)}>
            {#if r.src}<img src={r.src} alt={r.title} draggable="false" />{:else}<span>?</span>{/if}
          </button>
        {/each}
      </div>
      {#if schema && !schema.references}
        <span class="warn">{$t("canvas.generate.referencesUnsupported", { default: "This model takes no reference images." })}</span>
      {/if}
    {:else}
      <span class="hint">{$t("canvas.generate.referencesHint", { default: "Cord image nodes into this node to use them as references." })}</span>
    {/if}
  </div>

  <label class="field used">
    <span class="label">{$t("canvas.generate.usedIn", { default: "Will be used in" })}</span>
    <div class="used-row">
      <input
        type="text"
        value={usedIn?.text ?? ""}
        placeholder={$t("canvas.generate.usedInPlaceholder", { default: "e.g. login screen of the app" })}
        onchange={(e) => setUsedIn((e.currentTarget as HTMLInputElement).value)}
      />
      {#if usedInNode}
        <button type="button" class="target" onclick={() => showNode(usedInNode.id)} title={usedInNode.title ?? ""}>→ {usedInNode.title || usedInNode.kind}</button>
      {/if}
    </div>
  </label>

  <div class="preview">
    {#if latest}
      <button type="button" class="hero" onclick={() => showNode(latest.nodeId)} title={latest.prompt}>
        <img src={srcOf(latest.path)} alt={`v${latest.version}`} draggable="false" />
        <span class="badge">v{latest.version}</span>
      </button>
      {#if versions.length > 1}
        <div class="versions" aria-label={$t("canvas.generate.versions", { default: "Versions" })}>
          {#each versions.slice().reverse() as v (v.version)}
            <button type="button" class="vthumb" class:current={v.version === latest.version} onclick={() => showNode(v.nodeId)} title={`v${v.version} · ${formatCredits(v.credits)} · ${v.prompt}`}>
              <img src={srcOf(v.path)} alt={`v${v.version}`} draggable="false" />
              <span>v{v.version}</span>
            </button>
          {/each}
        </div>
      {/if}
    {:else if gstate === "running"}
      <div class="placeholder running"><div class="shimmer"></div></div>
    {:else}
      <div class="placeholder">{$t("canvas.generate.noVersions", { default: "Nothing generated yet." })}</div>
    {/if}
  </div>

  <div class="footer">
    {#if gstate === "running"}
      <div class="progress">
        <div class="bar"><div class="fill"></div></div>
        <span class="status">{statusLabel(liveStatus || progress?.status || "queued")} · {$t("canvas.generate.elapsed", { default: "{{seconds}} s", seconds: elapsed })}</span>
      </div>
    {:else if gstate === "uncertain"}
      <div class="actions">
        <button type="button" class="ghost" disabled={!!busy} onclick={() => run("reconcile", () => generateReconcile(node.id))}>
          {$t("canvas.generate.searchAgain", { default: "Search again" })}
        </button>
        <button type="button" class="ghost danger" disabled={!!busy} onclick={() => run("release", () => generateRelease(node.id)).then(loadBudget)}>
          {$t("canvas.generate.release", { default: "Release credits" })}
        </button>
      </div>
    {:else}
      {#if gstate === "awaiting_approval"}
        <div class="approval">{$t("canvas.generate.awaiting", { default: "An agent asked to generate this. It needs your approval." })}</div>
      {/if}
      <div class="actions">
        {#if quote && quoteLive}
          <span class="quote" class:bad={!quote.fitsBudget}>
            {$t("canvas.generate.quote", { default: "≈ {{credits}} credits · {{left}} left", credits: formatCredits(quote.credits), left: formatCredits(quote.left) })}
          </span>
        {:else}
          <button type="button" class="ghost" disabled={!!busy || !model || !promptDraft.trim()} onclick={price}>
            {busy === "quote" ? $t("canvas.generate.pricing", { default: "Pricing…" }) : $t("canvas.generate.getPrice", { default: "Get price" })}
          </button>
        {/if}
        <button
          type="button"
          class="primary"
          disabled={!!busy || !quote || !quoteLive || !quote.fitsBudget}
          onclick={go}
        >
          {#if busy === "submit"}…{:else if gstate === "awaiting_approval"}{$t("canvas.generate.approve", { default: "Approve and generate" })}{:else}{$t("canvas.generate.generate", { default: "Generate" })}{/if}
        </button>
      </div>
    {/if}

    {#if shownError}
      <div class="error">{errorText(shownError)}</div>
    {:else if quote && quoteLive && quote.reason}
      <div class="error">{errorText(quote.reason)}</div>
    {/if}

    <div class="budget">
      {#if editingBudget}
        <input
          type="number"
          min="0"
          step="1"
          value={budgetDraft}
          oninput={(e) => (budgetDraft = (e.currentTarget as HTMLInputElement).value)}
          onkeydown={(e) => {
            if (e.key === "Enter") void saveBudget();
            if (e.key === "Escape") editingBudget = false;
          }}
        />
        <button type="button" class="link" disabled={busy === "budget"} onclick={saveBudget}>{$t("canvas.generate.save", { default: "Save" })}</button>
        <button type="button" class="link" onclick={() => (editingBudget = false)}>{$t("canvas.generate.cancel", { default: "Cancel" })}</button>
      {:else if budget}
        <span class="muted" title={$t("canvas.generate.agentLimit", { default: "Agents alone: up to {{limit}} credits per image, {{perHour}} per hour", limit: formatCredits(budget.autoLimit), perHour: budget.autoPerHour })}>
          {#if budget.ceiling > 0}
            {$t("canvas.generate.budgetLine", { default: "Budget: {{left}} of {{ceiling}} credits left", left: formatCredits(budget.left), ceiling: formatCredits(budget.ceiling) })}
          {:else}
            {$t("canvas.generate.budgetUnset", { default: "No budget set: nothing can be spent yet." })}
          {/if}
        </span>
        <button
          type="button"
          class="link"
          onclick={() => {
            budgetDraft = budget && budget.ceiling > 0 ? String(budget.ceiling) : "";
            editingBudget = true;
          }}>{$t("canvas.generate.setBudget", { default: "Set budget" })}</button
        >
      {/if}
    </div>
  </div>
</div>

<style>
  .gen {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    overflow: auto;
    color: var(--text);
    font-size: 12px;
    box-sizing: border-box;
  }
  .top {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }
  .model {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .label {
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  select,
  input[type="text"],
  input[type="number"],
  textarea {
    width: 100%;
    box-sizing: border-box;
    font: inherit;
    color: var(--text);
    background: var(--input-bg, color-mix(in srgb, var(--text) 5%, transparent));
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 6px 8px;
    outline: none;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }
  select:focus,
  input:focus,
  textarea:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  textarea {
    resize: none;
    min-height: 64px;
    line-height: 1.45;
    font-size: 13px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .chip {
    flex-shrink: 0;
    padding: 3px 9px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    background: color-mix(in srgb, var(--text) 8%, transparent);
    color: var(--text-muted);
    margin-bottom: 3px;
  }
  .chip.state-quoted {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .chip.state-running {
    background: color-mix(in srgb, #3b82f6 18%, transparent);
    color: #3b82f6;
  }
  .chip.state-done {
    background: color-mix(in srgb, #22c55e 18%, transparent);
    color: #16a34a;
  }
  .chip.state-awaiting_approval,
  .chip.state-uncertain {
    background: color-mix(in srgb, #f59e0b 20%, transparent);
    color: #d97706;
  }
  .chip.state-failed {
    background: color-mix(in srgb, #ef4444 18%, transparent);
    color: #dc2626;
  }
  .params {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(104px, 1fr));
    gap: 6px;
  }
  .param {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .spent {
    font-size: 10.5px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .pname {
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.04em;
    color: var(--text-muted);
    text-transform: uppercase;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .param input[type="checkbox"] {
    width: 16px;
    height: 16px;
    margin: 6px 0;
    accent-color: var(--accent);
  }
  .refs {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .thumbs {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .thumb,
  .vthumb,
  .hero,
  .target,
  .link,
  .ghost,
  .primary {
    all: unset;
    cursor: pointer;
    box-sizing: border-box;
  }
  .thumb {
    width: 40px;
    height: 40px;
    border-radius: 8px;
    overflow: hidden;
    border: 1px solid var(--border);
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--text) 5%, transparent);
  }
  .thumb img,
  .vthumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .hint,
  .muted {
    color: var(--text-muted);
    font-size: 11.5px;
  }
  .warn {
    color: #d97706;
    font-size: 11.5px;
  }
  .used-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .target {
    flex-shrink: 0;
    max-width: 45%;
    padding: 5px 9px;
    border-radius: 999px;
    border: 1px dashed var(--accent);
    color: var(--accent);
    font-size: 11.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .preview {
    flex: 1;
    min-height: 110px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .hero {
    position: relative;
    flex: 1;
    min-height: 100px;
    border-radius: 10px;
    overflow: hidden;
    background:
      repeating-conic-gradient(color-mix(in srgb, var(--text) 6%, transparent) 0 25%, transparent 0 50%) 0 0 / 16px 16px;
    border: 1px solid var(--border);
  }
  .hero img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .badge {
    position: absolute;
    top: 8px;
    left: 8px;
    padding: 2px 7px;
    border-radius: 6px;
    font-size: 11px;
    font-weight: 600;
    color: #fff;
    background: rgba(0, 0, 0, 0.55);
    backdrop-filter: blur(6px);
  }
  .versions {
    display: flex;
    gap: 6px;
    overflow-x: auto;
    padding-bottom: 2px;
  }
  .vthumb {
    position: relative;
    flex-shrink: 0;
    width: 52px;
    height: 52px;
    border-radius: 8px;
    overflow: hidden;
    border: 2px solid transparent;
    opacity: 0.75;
    transition: opacity 0.15s ease, border-color 0.15s ease;
  }
  .vthumb:hover,
  .vthumb.current {
    opacity: 1;
    border-color: var(--accent);
  }
  .vthumb span {
    position: absolute;
    bottom: 2px;
    right: 3px;
    font-size: 9.5px;
    font-weight: 700;
    color: #fff;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.7);
  }
  .placeholder {
    flex: 1;
    min-height: 100px;
    border-radius: 10px;
    border: 1px dashed var(--border);
    display: grid;
    place-items: center;
    color: var(--text-muted);
    position: relative;
    overflow: hidden;
  }
  .placeholder.running {
    border-style: solid;
    background: color-mix(in srgb, var(--text) 4%, transparent);
  }
  .shimmer {
    position: absolute;
    inset: 0;
    background: linear-gradient(
      100deg,
      transparent 20%,
      color-mix(in srgb, var(--accent) 18%, transparent) 50%,
      transparent 80%
    );
    background-size: 200% 100%;
    animation: shimmer 1.6s linear infinite;
  }
  @keyframes shimmer {
    from {
      background-position: 200% 0;
    }
    to {
      background-position: -200% 0;
    }
  }
  .footer {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-top: 1px solid var(--border);
    padding-top: 10px;
  }
  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
  }
  .quote {
    margin-right: auto;
    font-size: 12.5px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .quote.bad {
    color: #dc2626;
  }
  .ghost {
    margin-right: auto;
    padding: 7px 12px;
    border-radius: 8px;
    border: 1px solid var(--border);
    font-weight: 600;
  }
  .ghost + .ghost {
    margin-right: 0;
  }
  .ghost:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text) 6%, transparent);
  }
  .ghost.danger {
    color: #dc2626;
  }
  .primary {
    padding: 7px 16px;
    border-radius: 8px;
    font-weight: 700;
    color: var(--on-accent, #fff);
    background: var(--accent);
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.15) inset, 0 4px 14px color-mix(in srgb, var(--accent) 35%, transparent);
    transition: transform 0.1s ease, filter 0.15s ease;
  }
  .primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .primary:active:not(:disabled) {
    transform: translateY(1px);
  }
  button:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .approval {
    padding: 8px 10px;
    border-radius: 8px;
    background: color-mix(in srgb, #f59e0b 14%, transparent);
    color: #b45309;
    font-size: 12px;
  }
  .progress {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .bar {
    height: 6px;
    border-radius: 999px;
    overflow: hidden;
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .fill {
    width: 40%;
    height: 100%;
    border-radius: 999px;
    background: linear-gradient(90deg, var(--accent), #3b82f6);
    animation: slide 1.4s ease-in-out infinite;
  }
  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(250%);
    }
  }
  .status {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .error {
    color: #dc2626;
    font-size: 11.5px;
    line-height: 1.4;
  }
  .budget {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11.5px;
  }
  .budget input {
    width: 90px;
    padding: 4px 6px;
  }
  .link {
    color: var(--accent);
    font-weight: 600;
  }
  .link:hover {
    text-decoration: underline;
  }
</style>
