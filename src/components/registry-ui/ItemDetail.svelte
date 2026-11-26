<script lang="ts" module>
  import type { ItemDetail, OutputLine, ProjectInfo, RunOutcome } from "$lib/components/types";
  import type { WrittenFiles } from "$lib/components/command";

  export type DetailState = { state: "loading" } | { state: "ready"; data: ItemDetail; offline: boolean } | { state: "error"; code: string; detail: string | null };

  export type RunState = {
    kind: "add" | "init";
    itemKey: string | null;
    name: string;
    state: "running" | "done" | "failed" | "cancelled";
    display: string;
    lines: OutputLine[];
    outcome: RunOutcome | null;
    written: WrittenFiles | null;
  };
</script>

<script lang="ts">
  import { t } from "$lib/i18n";
  import { errorKey } from "$lib/components/errors";
  import type { BuiltCommand, ItemSummary, RegistrySource } from "$lib/components/types";
  import StatusIcon from "#components/converter/StatusIcon.svelte";
  import CodeViewer from "./CodeViewer.svelte";
  import CopyButton from "./CopyButton.svelte";

  let {
    item,
    source,
    detail,
    project,
    managerLabel,
    ambiguous,
    addCommand,
    initCommand,
    previewUrl,
    run,
    yarnClassic = false,
    overwrite = $bindable(false),
    onadd,
    oninit,
    oncancel,
    onchooseproject,
    onopen,
    onretry,
    ondismiss,
  }: {
    item: ItemSummary;
    source: RegistrySource | null;
    detail: DetailState;
    project: ProjectInfo | null;
    managerLabel: string;
    ambiguous: boolean;
    addCommand: BuiltCommand | null;
    initCommand: BuiltCommand | null;
    previewUrl: string | null;
    run: RunState | null;
    yarnClassic?: boolean;
    overwrite?: boolean;
    onadd: () => void;
    oninit: () => void;
    oncancel: () => void;
    onchooseproject: () => void;
    onopen: (url: string) => void;
    onretry: () => void;
    ondismiss: () => void;
  } = $props();

  let running = $derived(run?.state === "running");
  let ownRun = $derived(!!run && (run.itemKey === item.key || run.kind === "init"));
  let ready = $derived(!!project && project.has_package_json && project.has_components_json && !project.components_json_error);
  let needsInit = $derived(!!project && project.has_package_json && !project.has_components_json);
  let deps = $derived(detail.state === "ready" ? detail.data.dependencies : item.dependencies);
  let devDeps = $derived(detail.state === "ready" ? detail.data.devDependencies : []);
  let regDeps = $derived(detail.state === "ready" ? detail.data.registryDependencies : item.registryDependencies);
  let description = $derived((detail.state === "ready" && detail.data.description) || item.description);
  let lastError = $derived.by(() => {
    if (!run) return "";
    const err = [...run.lines].reverse().find((l) => l.stream === "stderr" && l.line.trim());
    return err?.line.trim().slice(0, 300) ?? "";
  });
  let writtenCount = $derived(run?.written ? run.written.created.length + run.written.updated.length : 0);
  let logEl = $state<HTMLPreElement | null>(null);

  $effect(() => {
    void run?.lines.length;
    if (logEl) logEl.scrollTop = logEl.scrollHeight;
  });

  function blockedReason(): string | null {
    if (running) return $t("components.add_blocked_running", { name: run?.name ?? "" }) as string;
    if (!project) return $t("components.add_blocked_no_project") as string;
    if (!project.has_package_json) return $t("components.add_blocked_no_package") as string;
    if (project.components_json_error) return $t("components.add_blocked_bad_config") as string;
    if (!project.has_components_json) return $t("components.add_blocked_no_config") as string;
    if (!addCommand) return $t("components.add_blocked_no_url") as string;
    return null;
  }
</script>

<article class="cr-detail" aria-labelledby="cr-detail-title">
  <header class="cr-detail-head">
    <div class="cr-detail-titles">
      <h2 id="cr-detail-title">{item.title}</h2>
      <p class="cr-detail-meta">
        <span class="cr-chip">{item.type}</span>
        <span>{source?.name ?? item.registryId}</span>
        {#if item.title !== item.name}<span class="cr-mono">{item.name}</span>{/if}
      </p>
    </div>
    {#if previewUrl}
      <button type="button" class="btn btn-ghost btn-sm" onclick={() => previewUrl && onopen(previewUrl)}>
        {$t("components.preview_docs")}
      </button>
    {/if}
  </header>

  {#if description}
    <p class="cr-detail-desc">{description}</p>
  {/if}

  <dl class="cr-deps">
    <div>
      <dt>{$t("components.npm_dependencies")}</dt>
      <dd>
        {#if deps.length}
          {#each deps as d (d)}<span class="cr-chip cr-mono">{d}</span>{/each}
        {:else}
          <span class="cr-none">{$t("components.none")}</span>
        {/if}
      </dd>
    </div>
    {#if devDeps.length}
      <div>
        <dt>{$t("components.dev_dependencies")}</dt>
        <dd>{#each devDeps as d (d)}<span class="cr-chip cr-mono">{d}</span>{/each}</dd>
      </div>
    {/if}
    <div>
      <dt>{$t("components.registry_dependencies")}</dt>
      <dd>
        {#if regDeps.length}
          {#each regDeps as d (d)}<span class="cr-chip cr-mono">{d}</span>{/each}
        {:else}
          <span class="cr-none">{$t("components.none")}</span>
        {/if}
      </dd>
    </div>
  </dl>

  <section class="cr-add" aria-labelledby="cr-add-title">
    <h3 id="cr-add-title" class="cr-section-title">{$t("components.add_title")}</h3>

    {#if run && !ownRun}
      <div class="cr-other-run" role="status">
        <StatusIcon name={run.state === "running" ? "running" : run.state === "done" ? "done" : run.state === "cancelled" ? "stopped" : "failed"} size={14} />
        <p>
          {run.state === "running"
            ? $t("components.add_running", { name: run.name })
            : run.state === "done"
              ? $t("components.add_done", { name: run.name, project: project?.name ?? "" })
              : run.state === "cancelled"
                ? $t("components.run_cancelled_title")
                : $t("components.add_failed", { name: run.name })}
        </p>
        {#if run.state === "running"}
          <button type="button" class="btn btn-secondary btn-sm" onclick={oncancel}>{$t("components.stop")}</button>
        {:else}
          <button type="button" class="btn btn-ghost btn-sm" onclick={ondismiss}>{$t("components.close_output")}</button>
        {/if}
      </div>
    {/if}

    {#if run && ownRun}
      {#if run.state === "running"}
        <p class="cr-run-status" role="status">
          <StatusIcon name="running" size={14} />
          {run.kind === "init" ? $t("components.init_running") : $t("components.add_running", { name: run.name })}
        </p>
      {:else if run.state === "done"}
        <div class="cr-result ok" role="status">
          <StatusIcon name="done" size={18} />
          <div>
            <p class="cr-result-title">
              {run.kind === "init"
                ? $t("components.init_done", { project: project?.name ?? "" })
                : $t("components.add_done", { name: run.name, project: project?.name ?? "" })}
            </p>
            {#if run.kind === "add" && run.written}
              {#if writtenCount > 0}
                <p class="cr-result-body">{$t("components.files_written", { count: writtenCount })}</p>
                <ul class="cr-written">
                  {#each [...run.written.created, ...run.written.updated] as f (f)}<li class="cr-mono">{f}</li>{/each}
                </ul>
              {/if}
              {#if run.written.skipped.length}
                <p class="cr-result-body">{$t("components.files_skipped", { count: run.written.skipped.length })}</p>
              {/if}
              {#if writtenCount === 0 && run.written.skipped.length === 0}
                <p class="cr-result-body">{$t("components.files_unknown")}</p>
              {/if}
            {:else if run.kind === "init"}
              <p class="cr-result-body">{$t("components.init_next")}</p>
            {/if}
          </div>
        </div>
      {:else}
        <div class="cr-result" role="alert">
          <StatusIcon name={run.state === "cancelled" ? "stopped" : "failed"} size={18} />
          <div>
            <p class="cr-result-title">
              {run.state === "cancelled" ? $t("components.run_cancelled_title") : run.kind === "init" ? $t("components.init_failed") : $t("components.add_failed", { name: run.name })}
            </p>
            {#if run.state !== "cancelled"}
              <p class="cr-result-body">{$t(errorKey(run.outcome?.error ?? "unknown"), { detail: run.outcome?.code ?? "" })}</p>
              {#if lastError}<p class="cr-result-body cr-mono">{lastError}</p>{/if}
            {/if}
          </div>
        </div>
      {/if}

      {#if run.lines.length}
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <pre class="cr-log" bind:this={logEl} role="log" aria-label={$t("components.output")} tabindex="0">{#each run.lines as l, i (i)}<span class:err={l.stream === "stderr"}>{l.line}
</span>{/each}</pre>
      {/if}

      <div class="cr-actions">
        {#if run.state === "running"}
          <button type="button" class="btn btn-secondary" onclick={oncancel}>{$t("components.stop")}</button>
        {:else}
          {#if run.state === "failed" || run.state === "cancelled"}
            <button type="button" class="btn btn-primary" onclick={onretry}>{$t("components.try_again")}</button>
          {/if}
          <button type="button" class="btn btn-ghost" onclick={ondismiss}>{$t("components.close_output")}</button>
        {/if}
      </div>
    {:else if needsInit && initCommand}
      <div class="cr-explain">
        <StatusIcon name="warning" size={16} />
        <div>
          <p class="cr-explain-title">{$t("components.init_needed_title", { project: project?.name ?? "" })}</p>
          <p class="cr-explain-body">{$t("components.init_needed_body")}</p>
        </div>
      </div>
      <div class="cr-cmd">
        <code class="cr-mono">{initCommand.display}</code>
        <CopyButton text={initCommand.display} label={$t("components.copy_command") as string} />
      </div>
      <div class="cr-actions">
        <button type="button" class="btn btn-primary" disabled={running} aria-describedby={running ? "cr-init-why" : undefined} onclick={oninit}>{$t("components.init_button")}</button>
        <button type="button" class="btn btn-ghost" onclick={onchooseproject}>{$t("components.choose_other_project")}</button>
      </div>
      {#if running}<p id="cr-init-why" class="cr-hint">{blockedReason()}</p>{/if}
      {#if addCommand}
        <p class="cr-hint">{$t("components.copy_for_later")}</p>
        <div class="cr-cmd">
          <code class="cr-mono">{addCommand.display}</code>
          <CopyButton text={addCommand.display} label={$t("components.copy_command") as string} />
        </div>
      {/if}
    {:else}
      {#if addCommand}
        <p class="cr-hint">
          {#if project}
            {$t("components.will_run", { project: project.name, manager: managerLabel })}
          {:else}
            {$t("components.command_preview")}
          {/if}
        </p>
        <div class="cr-cmd">
          <code class="cr-mono">{addCommand.display}</code>
          <CopyButton text={addCommand.display} label={$t("components.copy_command") as string} />
        </div>
        {#if ambiguous}<p class="cr-hint">{$t("components.lock_ambiguous", { manager: managerLabel })}</p>{/if}
        {#if yarnClassic}<p class="cr-hint">{$t("components.yarn_classic")}</p>{/if}
      {/if}
      {#if ready}
        <label class="cr-check">
          <input type="checkbox" class="checkbox" bind:checked={overwrite} />
          <span>{$t("components.overwrite")}</span>
        </label>
      {/if}
      <div class="cr-actions">
        <button type="button" class="btn btn-primary" disabled={!ready || !addCommand || running} aria-describedby={blockedReason() ? "cr-add-why" : undefined} onclick={onadd}>
          {$t("components.add_button")}
        </button>
        {#if !project || !ready}
          <button type="button" class="btn btn-secondary" onclick={onchooseproject}>
            {project ? $t("components.choose_other_project") : $t("components.choose_project")}
          </button>
        {/if}
      </div>
      {#if blockedReason()}
        <p id="cr-add-why" class="cr-hint">{blockedReason()}</p>
      {/if}
    {/if}
  </section>

  <section class="cr-files" aria-labelledby="cr-files-title">
    <h3 id="cr-files-title" class="cr-section-title">{$t("components.files_title")}</h3>
    {#if detail.state === "loading"}
      <p class="cr-run-status" role="status"><StatusIcon name="running" size={14} /> {$t("components.loading_files")}</p>
    {:else if detail.state === "error"}
      <div class="cr-result" role="alert">
        <StatusIcon name="failed" size={18} />
        <div>
          <p class="cr-result-title">{$t("components.item_failed")}</p>
          <p class="cr-result-body">{$t(errorKey(detail.code), { detail: detail.detail ?? "" })}</p>
          <button type="button" class="btn btn-secondary btn-sm" onclick={onretry}>{$t("components.try_again")}</button>
        </div>
      </div>
    {:else}
      {#if detail.offline}<p class="cr-hint"><StatusIcon name="warning" size={12} /> {$t("components.item_offline")}</p>{/if}
      {#if detail.data.cssVarCount > 0}<p class="cr-hint">{$t("components.item_css_vars", { count: detail.data.cssVarCount })}</p>{/if}
      {#if detail.data.files.length === 0}
        <p class="cr-hint">{$t("components.no_files")}</p>
      {:else}
        <div class="cr-file-list">
          {#each detail.data.files as file (file.path)}
            <CodeViewer path={file.target ?? file.path} content={file.content} note={file.type} />
          {/each}
        </div>
      {/if}
    {/if}
  </section>
</article>

<style>
  .cr-other-run {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-2);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cr-other-run p {
    flex: 1;
    min-width: 0;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .cr-detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  .cr-detail-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .cr-detail-titles h2 {
    margin: 0 0 var(--space-1);
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .cr-detail-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .cr-detail-desc {
    margin: 0;
    font-size: var(--text-base);
    color: var(--text-muted);
  }

  .cr-mono {
    font-family: var(--font-mono);
  }

  .cr-chip {
    display: inline-flex;
    align-items: center;
    height: 20px;
    padding: 0 var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .cr-deps {
    display: grid;
    gap: var(--space-2);
    margin: 0;
  }

  .cr-deps > div {
    display: grid;
    grid-template-columns: 170px 1fr;
    gap: var(--space-3);
    align-items: baseline;
  }

  .cr-deps dt {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .cr-deps dd {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin: 0;
  }

  .cr-none {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .cr-add,
  .cr-files {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .cr-add {
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .cr-section-title {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .cr-hint {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .cr-cmd {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-2);
  }

  .cr-cmd code {
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .cr-check {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .cr-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .cr-explain,
  .cr-result {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
  }

  .cr-explain-title,
  .cr-result-title {
    margin: 0 0 var(--space-1);
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
  }

  .cr-explain-body,
  .cr-result-body {
    margin: 0 0 var(--space-1);
    font-size: var(--text-sm);
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .cr-written {
    margin: 0;
    padding-left: var(--space-4);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .cr-run-status {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .cr-log {
    max-height: 220px;
    margin: 0;
    padding: var(--space-2) var(--space-3);
    overflow: auto;
    border-radius: var(--radius-md);
    background: var(--fill-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.55;
    color: var(--text-muted);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .cr-log:focus-visible {
    outline: var(--focus-ring);
  }

  .cr-log .err {
    color: var(--text);
  }

  .cr-file-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
</style>
