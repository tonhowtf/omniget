<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "$lib/devtools/api";
  import { pluralKey } from "$lib/home/plural";
  import { budgetMeter, excludedRows, parseBudget, parseGlobs } from "$lib/devtools/helpers";
  import type { DevtoolsError, PackBuilt, PackPlan, PackRequest } from "$lib/devtools/types";
  import DevShell from "#components/devtools/DevShell.svelte";
  import ErrorNotice from "#components/devtools/ErrorNotice.svelte";
  import CopyButton from "#components/palette/CopyButton.svelte";
  import StatusIcon from "#components/converter/StatusIcon.svelte";

  let path = $state("");
  let include = $state("");
  let exclude = $state("");
  let budget = $state("");
  let busy = $state<"plan" | "build" | null>(null);
  let plan = $state<PackPlan | null>(null);
  let plannedBudget = $state<number | undefined>(undefined);
  let built = $state<PackBuilt | null>(null);
  let error = $state<DevtoolsError | null>(null);
  let dragging = $state(false);

  let meter = $derived(plan ? budgetMeter(plan.totals.tokens, plannedBudget) : null);
  let left = $derived(plan ? excludedRows(plan) : []);

  function count(n: number): string {
    return n.toLocaleString();
  }

  function request(): PackRequest {
    return { path: path.trim(), include: parseGlobs(include), exclude: parseGlobs(exclude), maxTokens: parseBudget(budget) };
  }

  async function run(kind: "plan" | "build") {
    if (!path.trim() || busy) return;
    busy = kind;
    error = null;
    built = null;
    const req = request();
    try {
      if (kind === "plan") {
        plan = await api.packPlan(req);
      } else {
        built = await api.packBuild(req);
        plan = built.manifest;
      }
      plannedBudget = req.maxTokens;
    } catch (e) {
      error = api.parseError(e);
    } finally {
      busy = null;
    }
  }

  function use(folder: string) {
    path = folder;
    void run("plan");
  }

  async function choose() {
    const picked = await openDialog({ directory: true, multiple: false, title: $t("pack.choose") as string }).catch(() => null);
    if (typeof picked === "string") use(picked);
  }

  onMount(() => {
    let disposed = false;
    let off: (() => void) | null = null;
    try {
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          const p = event.payload as { type: string; paths?: string[] };
          if (p.type === "enter" || p.type === "over") dragging = true;
          else if (p.type === "leave") dragging = false;
          else if (p.type === "drop") {
            dragging = false;
            if (p.paths?.length) use(p.paths[0]);
          }
        })
        .then((fn) => (disposed ? fn() : (off = fn)))
        .catch(() => {});
    } catch {
      dragging = false;
    }
    return () => {
      disposed = true;
      off?.();
    };
  });
</script>

<DevShell icon="pack" title={$t("pack.title") as string} subtitle={$t("pack.subtitle") as string} busy={busy !== null}>
  <form
    class="dv-panel"
    onsubmit={(e) => {
      e.preventDefault();
      void run("plan");
    }}
  >
    <div class="dv-row">
      <label class="dv-field wide">
        {$t("pack.folder")}
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input input-lg dv-mono" bind:value={path} placeholder={$t("pack.folder_hint") as string} spellcheck="false" autocomplete="off" autofocus />
      </label>
      <button type="button" class="btn btn-secondary btn-lg" onclick={choose}>{$t("pack.choose")}</button>
    </div>
    <div class="dv-row">
      <label class="dv-field">
        {$t("pack.include")}
        <input class="input dv-mono" bind:value={include} placeholder="src/**, *.md" spellcheck="false" autocomplete="off" />
      </label>
      <label class="dv-field">
        {$t("pack.exclude")}
        <input class="input dv-mono" bind:value={exclude} placeholder="**/*.test.ts" spellcheck="false" autocomplete="off" />
      </label>
      <label class="dv-field pk-budget">
        {$t("pack.budget")}
        <input class="input dv-mono" bind:value={budget} placeholder={$t("pack.budget_none") as string} inputmode="numeric" autocomplete="off" />
      </label>
    </div>
    <div class="dv-bar">
      <p class="dv-meta">{$t("pack.hint")}</p>
      <div class="dv-bar-group">
        <button type="submit" class="btn {plan ? 'btn-secondary' : 'btn-primary'}" disabled={!path.trim() || busy !== null}>
          {#if busy === "plan"}<span class="spinner" aria-hidden="true"></span>{/if}
          {$t("pack.plan")}
        </button>
        <button type="button" class="btn {plan ? 'btn-primary' : 'btn-secondary'}" disabled={!path.trim() || busy !== null} onclick={() => run("build")}>
          {#if busy === "build"}<span class="spinner" aria-hidden="true"></span>{/if}
          {$t("pack.build")}
        </button>
      </div>
    </div>
  </form>

  {#if error}
    <ErrorNotice {error} />
  {/if}

  {#if built}
    <div class="dv-panel" role="status">
      <div class="pk-built">
      <StatusIcon name="done" size={18} />
      <div class="pk-built-copy">
        <h2 class="dv-title">{$t("pack.built_title")}</h2>
        <p class="dv-meta dv-mono">{built.path}</p>
        <p class="dv-meta">{$t("devtools.tokens", { count: count(built.tokens) })} · {$t("api.size", { bytes: count(built.bytes) })}</p>
      </div>
      <div class="dv-bar-group">
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => built && void api.reveal(built.path).catch(() => {})}>{$t("pack.reveal")}</button>
        <CopyButton text={built.path} label={$t("pack.copy_path") as string} named />
        </div>
      </div>
    </div>
  {/if}

  {#if plan}
    <div class="dv-panel">
      <div class="dv-bar">
        <h2 class="dv-title">
          {#if plannedBudget}
            {$t(pluralKey("pack.total_budget", plan.totals.files), { tokens: count(plan.totals.tokens), budget: count(plannedBudget), files: count(plan.totals.files) })}
          {:else}
            {$t(pluralKey("pack.total", plan.totals.files), { tokens: count(plan.totals.tokens), files: count(plan.totals.files) })}
          {/if}
        </h2>
        <p class="dv-meta dv-mono">{plan.root}</p>
      </div>
      {#if meter}
        <div class="pk-meter" class:over={meter.over || plan.totals.overBudget} role="meter" aria-valuemin="0" aria-valuemax={plannedBudget} aria-valuenow={plan.totals.tokens} aria-label={$t("pack.budget") as string}>
          <span style:width="{meter.ratio * 100}%"></span>
        </div>
      {/if}
      {#if plan.totals.overBudget || plan.totals.notIncluded > 0}
        <p class="dv-warn">{$t("pack.over_budget", { count: count(plan.totals.notIncluded) })}</p>
      {/if}
      {#if plan.reduced.length > 0}
        <p class="dv-meta">{$t("pack.reduced", { count: count(plan.reduced.length) })}</p>
      {/if}
      {#if plan.files.length === 0}
        <p class="dv-empty">{$t("pack.no_files")}</p>
      {:else}
        <div class="dv-scroll">
          <table class="dv-table">
            <thead>
              <tr>
                <th>{$t("pack.col_file")}</th>
                <th>{$t("pack.col_detail")}</th>
                <th class="num">{$t("pack.col_tokens")}</th>
              </tr>
            </thead>
            <tbody>
              {#each plan.files as file (file.path)}
                <tr>
                  <td class="dv-mono">{file.path}</td>
                  <td class:dim={file.mode === "full"}>{$t(`pack.mode_${file.mode}`)}</td>
                  <td class="num dv-mono">{count(file.tokens)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        {#if plan.truncated}
          <p class="dv-meta">{$t("pack.files_truncated", { shown: count(plan.files.length), total: count(plan.total) })}</p>
        {/if}
      {/if}
    </div>

    {#if left.length > 0}
      <div class="dv-panel">
        <h2 class="dv-title">{$t("pack.excluded_title", { count: count(plan.totals.excluded) })}</h2>
        <div class="dv-scroll">
          <table class="dv-table">
            <tbody>
              {#each left as row (row.where + row.detail)}
                <tr>
                  <td class="dv-mono">{row.where}</td>
                  <td>
                    <span class="tag" class:tag-warning={row.reason === "secret"}>{$t(`pack.reason_${row.reason}`)}</span>
                  </td>
                  <td class="dim dv-mono">{row.detail}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      </div>
    {/if}
  {/if}

  {#if dragging}
    <div class="dv-dropping" aria-hidden="true">{$t("pack.drop")}</div>
  {/if}
</DevShell>

<style>
  .dv-row > .pk-budget {
    flex: 0 1 140px;
  }

  .pk-built {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: var(--space-3);
  }

  .pk-built-copy {
    display: flex;
    flex: 1 1 260px;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .pk-meter {
    height: 6px;
    overflow: hidden;
    border-radius: var(--radius-full);
    background: var(--fill-1);
  }

  .pk-meter span {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--accent);
    transition: width var(--duration-base) var(--ease-out);
  }

  .pk-meter.over span {
    background: var(--warning);
  }
</style>
