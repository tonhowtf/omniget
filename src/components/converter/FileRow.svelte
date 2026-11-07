<script lang="ts">
  import { t } from "$lib/i18n";
  import { baseName, engineName, failureFix, formatBytes, groupByCategory, isAvailable, routeLabel, type RowPlan, type RowStatus } from "$lib/converter/plan";
  import type { ConvertOutcome, InspectResult, Route } from "$lib/converter/types";
  import StatusIcon, { type StatusIconName } from "./StatusIcon.svelte";

  let {
    file,
    status,
    percent,
    outcome,
    runError,
    plan,
    override,
    batchLabel,
    labelOf,
    locked,
    onoverride,
    onremove,
  }: {
    file: InspectResult;
    status: RowStatus;
    percent: number;
    outcome: ConvertOutcome | null;
    runError: string | null;
    plan: RowPlan;
    override: string | null;
    batchLabel: string | null;
    labelOf: (id: string) => string;
    locked: boolean;
    onoverride: (format: string | null) => void;
    onremove: () => void;
  } = $props();

  let native = $derived($t("converter.engine_native") as string);
  let readable = $derived(!file.error && !!file.format);
  let groups = $derived(groupByCategory(file.targets));

  let route = $derived.by<Route | null>(() => {
    if (outcome?.route && outcome.route.steps.length) return outcome.route;
    if (plan.kind === "convert") return plan.option.route;
    return null;
  });

  let chip = $derived(route ? routeLabel(route, labelOf, native) : "");

  let info = $derived.by<{ icon: StatusIconName; text: string; detail?: string; fix?: string }>(() => {
    const tr = $t;
    if (status === "queued") return { icon: "queued", text: tr("converter.status.queued") as string };
    if (status === "running") return { icon: "running", text: tr("converter.status.running", { percent: Math.round(percent) }) as string };
    if (status === "done") return { icon: "done", text: tr("converter.status.done") as string };
    if (status === "stopped") return { icon: "stopped", text: tr("converter.status.stopped") as string, fix: tr("converter.fix.stopped") as string };
    if (status === "skipped") {
      return { icon: "skipped", text: tr("converter.status.skipped", { format: file.format?.label ?? "" }) as string };
    }
    if (status === "failed") {
      const detail = outcome?.error ?? runError ?? undefined;
      const fix = failureFix(detail);
      return {
        icon: "failed",
        text: tr("converter.status.failed") as string,
        detail,
        fix:
          fix.kind === "missing_tool"
            ? (tr("converter.fix.missing_tool_retry", { tool: engineName(fix.tool, native) }) as string)
            : (tr("converter.fix.generic") as string),
      };
    }
    switch (plan.kind) {
      case "convert":
        return { icon: "ready", text: tr("converter.status.ready") as string };
      case "skip":
        return { icon: "skipped", text: tr("converter.status.skipped", { format: plan.format.label }) as string };
      case "none":
        return { icon: "idle", text: tr("converter.status.none") as string };
      case "missing": {
        const tool = engineName(plan.tool, native);
        return { icon: "warning", text: tr("converter.status.missing", { tool }) as string, fix: tr("converter.fix.missing_tool", { tool }) as string };
      }
      case "unsupported":
        return {
          icon: "warning",
          text: tr("converter.status.unsupported", { format: labelOf(plan.target) }) as string,
          fix: tr("converter.fix.unsupported") as string,
        };
      case "unreadable":
        return { icon: "failed", text: tr("converter.status.unreadable") as string, detail: plan.error, fix: tr("converter.fix.unreadable") as string };
      case "unknown":
        return { icon: "warning", text: tr("converter.status.unknown") as string, fix: tr("converter.fix.unknown") as string };
    }
  });

  let meta = $derived([file.format?.label, formatBytes(file.size_bytes)].filter(Boolean).join(" · "));
  let outName = $derived(status === "done" && outcome?.output ? baseName(outcome.output) : null);
</script>

<li class="cv-row" class:attention={info.icon === "failed" || info.icon === "warning"}>
  <div class="cv-row-main">
    <div class="cv-row-file">
      <span class="cv-row-name" title={file.path}>{file.name}</span>
      <span class="cv-row-meta">
        {meta}
        {#if outName}
          <span class="cv-row-out" title={outcome?.output ?? undefined}>· {$t("converter.row_output", { name: outName })}</span>
        {/if}
      </span>
    </div>

    {#if chip}
      <span class="cv-route" title={chip}>
        <span class="cv-route-text">{chip}</span>
        {#if route?.lossy}
          <span class="cv-route-lossy" title={$t("converter.lossy_title") as string}>{$t("converter.lossy")}</span>
        {/if}
      </span>
    {/if}

    {#if readable}
      <select
        class="cv-row-select"
        aria-label={$t("converter.row_target", { name: file.name }) as string}
        value={override ?? ""}
        disabled={locked}
        onchange={(e) => onoverride((e.currentTarget as HTMLSelectElement).value || null)}
      >
        <option value="">{batchLabel ? $t("converter.row_same", { format: batchLabel }) : $t("converter.row_choose")}</option>
        {#each groups as group (group.category)}
          <optgroup label={$t(`converter.category.${group.category}`) as string}>
            {#each group.items as option (option.format)}
              <option value={option.format} disabled={!isAvailable(option)}>
                {isAvailable(option)
                  ? option.label
                  : `${option.label} · ${$t("converter.needs_tool", { tool: engineName(option.missing_tool ?? "native", native) })}`}
              </option>
            {/each}
          </optgroup>
        {/each}
      </select>
    {/if}

    <span class="cv-row-status">
      <StatusIcon name={info.icon} />
      <span class="cv-row-status-text">{info.text}</span>
    </span>

    <button
      type="button"
      class="btn btn-ghost btn-icon btn-sm cv-row-remove"
      aria-label={$t("converter.row_remove", { name: file.name }) as string}
      title={$t("converter.row_remove", { name: file.name }) as string}
      disabled={locked}
      onclick={onremove}
    >
      <svg viewBox="0 0 12 12" width="11" height="11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="m3 3 6 6M9 3 3 9" /></svg>
    </button>
  </div>

  {#if status === "running"}
    <div class="progress" role="progressbar" aria-label={file.name} aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(percent)}>
      <div class="progress-fill" style:width="{Math.max(2, Math.min(100, percent))}%"></div>
    </div>
  {/if}

  {#if info.detail || info.fix}
    <div class="cv-row-help">
      {#if info.detail}<p class="cv-row-detail">{info.detail}</p>{/if}
      {#if info.fix}<p class="cv-row-fix">{info.fix}</p>{/if}
    </div>
  {/if}
</li>

<style>
  .cv-row {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-top: var(--hairline) solid var(--content-border);
  }

  .cv-row:first-child {
    border-top: none;
  }

  .cv-row-main {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 176px 176px auto;
    grid-template-areas: "file route select status remove";
    align-items: center;
    gap: var(--space-3);
  }

  .cv-row-file {
    grid-area: file;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .cv-row-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-base);
    font-weight: 550;
    color: var(--text);
  }

  .cv-row-meta {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .cv-route {
    grid-area: route;
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    max-width: 260px;
    min-width: 0;
    padding: 2px var(--space-2);
    border-radius: var(--radius-full);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .cv-route-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cv-route-lossy {
    flex-shrink: 0;
    font-style: italic;
    color: var(--text-dim);
  }

  .cv-row-select {
    grid-area: select;
    width: 100%;
    max-width: 200px;
    font-size: var(--text-sm);
  }

  .cv-row-status {
    grid-area: status;
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .cv-row-status-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cv-row.attention .cv-row-status-text {
    color: var(--text);
    font-weight: 550;
  }

  .cv-row-remove {
    grid-area: remove;
  }

  .cv-row-help {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }

  .cv-row-detail,
  .cv-row-fix {
    margin: 0;
    font-size: var(--text-sm);
    line-height: var(--leading-sm);
    overflow-wrap: anywhere;
  }

  .cv-row-detail {
    color: var(--text);
  }

  .cv-row-fix {
    color: var(--text-dim);
  }

  @media (max-width: 760px) {
    .cv-row-main {
      grid-template-columns: minmax(0, 1fr) auto;
      grid-template-areas:
        "file remove"
        "status status"
        "route route"
        "select select";
    }

    .cv-route,
    .cv-row-select {
      justify-self: start;
    }
  }
</style>
