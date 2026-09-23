<script lang="ts">
  /**
   * Full guard report: the commands the component will execute (always shown
   * first, highlighted), then findings grouped by validator with file:line
   * and the offending snippet (secrets already redacted by the backend).
   *
   * When `onproceed` is given and the level is `block`, the proceed button
   * needs two presses ("install anyway" → "confirm"), like the skill scan
   * dialog. Nothing here ever calls a component safe.
   */
  import { onMount } from "svelte";
  import { t, locale } from "$lib/i18n";
  import GuardBadge from "./GuardBadge.svelte";
  import {
    findingsByValidator,
    guardRules,
    ruleText,
    rulesByCode,
    visibleFindings,
    type GuardReport,
    type GuardRule,
  } from "$lib/central/guard";

  let {
    report,
    busy = false,
    proceedLabel = "",
    onclose,
    onproceed,
  }: {
    report: GuardReport;
    busy?: boolean;
    proceedLabel?: string;
    onclose?: () => void;
    onproceed?: () => void;
  } = $props();

  let rules = $state<Map<string, GuardRule>>(new Map());
  let showInfo = $state(false);
  let armed = $state(false);

  onMount(() => {
    guardRules()
      .then((r) => (rules = rulesByCode(r)))
      .catch(() => {});
  });

  let groups = $derived(
    findingsByValidator(report)
      .map(([v, list]) => [v, visibleFindings(list, showInfo)] as const)
      .filter(([, list]) => list.length > 0),
  );
  let hiddenInfo = $derived(report.findings.filter((f) => f.severity === "info").length);
  let risky = $derived(report.commands.filter((c) => c.codes.length > 0 || c.auto_runs));

  function title(code: string, fallback: string): string {
    return ruleText(rules.get(code), $locale, fallback);
  }

  function where(file: string, line?: number, jsonPath?: string): string {
    const loc = line ? `${file}:${line}` : file;
    return jsonPath ? `${loc} · ${jsonPath}` : loc;
  }

  function press() {
    if (busy) return;
    if (report.level === "block" && !armed) {
      armed = true;
      return;
    }
    onproceed?.();
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape") onclose?.();
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose?.()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={$t("llm.central.guard.report_title")}>
    <header class="head">
      <div class="titles">
        <h2>{$t("llm.central.guard.report_title")}</h2>
        <p class="sub mono" title={report.label}>{report.label}</p>
      </div>
      <GuardBadge {report} />
    </header>

    <div class="body">
      <dl class="facts">
        <div>
          <dt>{$t("llm.central.guard.kind")}</dt>
          <dd>{report.kind}</dd>
        </div>
        <div>
          <dt>{$t("llm.central.guard.format")}</dt>
          <dd>{report.detected_tool}</dd>
        </div>
        {#if report.origin_tool}
          <div>
            <dt>{$t("llm.central.guard.target")}</dt>
            <dd>{report.origin_tool}</dd>
          </div>
        {/if}
        <div>
          <dt>{$t("llm.central.guard.files")}</dt>
          <dd>{report.files.length}</dd>
        </div>
      </dl>

      {#if report.foreign_format}
        <p class="note">{$t("llm.central.guard.foreign_note", { tool: report.foreign_format })}</p>
      {/if}

      {#if risky.length > 0}
        <section>
          <h3>{$t("llm.central.guard.commands_title")}</h3>
          <p class="hint">{$t("llm.central.guard.commands_hint")}</p>
          <ul class="cmds">
            {#each risky as c, i (i)}
              <li class="cmd" class:bad={c.worst === "critical" || c.worst === "high"}>
                <div class="cmd-meta">
                  <span class="chip">{$t(`llm.central.guard.origin_${c.origin}`)}</span>
                  {#if c.context}<span class="ctx">{c.context}</span>{/if}
                  {#if c.auto_runs}<span class="flag">{$t("llm.central.guard.auto_runs")}</span>{/if}
                  {#if c.network}<span class="flag">{$t("llm.central.guard.network")}</span>{/if}
                  <span class="where mono">{where(c.file, c.line, c.json_path)}</span>
                </div>
                <pre class="code">{c.command}</pre>
                {#if c.codes.length > 0}
                  <div class="codes">
                    {#each c.codes as code (code)}
                      <span class="code-tag" title={title(code, code)}>{code}</span>
                    {/each}
                  </div>
                {/if}
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#each groups as [validator, list] (validator)}
        <section>
          <h3>{$t(`llm.central.guard.validator_${validator}`)}</h3>
          <ul class="findings">
            {#each list as f, i (f.code + f.file + (f.line ?? "") + i)}
              <li class="finding">
                <span class="sev {f.severity}">{$t(`llm.central.guard.sev_${f.severity}`)}</span>
                <div class="what">
                  <div class="rule">
                    <span class="mono code-id">{f.code}</span>
                    {title(f.code, f.detail)}
                    {#if f.downgraded}<span class="lowered" title={$t("llm.central.guard.lowered_hint")}>↓</span>{/if}
                  </div>
                  {#if f.detail}<div class="detail">{f.detail}</div>{/if}
                  {#if f.file}<div class="where mono">{where(f.file, f.line, f.json_path)}</div>{/if}
                  {#if f.snippet}<pre class="snippet">{f.snippet}</pre>{/if}
                </div>
              </li>
            {/each}
          </ul>
        </section>
      {/each}

      {#if groups.length === 0 && risky.length === 0}
        <p class="empty">{$t("llm.central.guard.no_findings")}</p>
      {/if}

      {#if hiddenInfo > 0}
        <button type="button" class="link" onclick={() => (showInfo = !showInfo)}>
          {showInfo ? $t("llm.central.guard.hide_info") : $t("llm.central.guard.show_info", { n: String(hiddenInfo) })}
        </button>
      {/if}

      <p class="note">{$t("llm.central.guard.static_only")}</p>
      {#if armed}
        <p class="warn" role="alert">{$t("llm.central.guard.confirm_warning")}</p>
      {/if}
    </div>

    <footer class="foot">
      <button type="button" class="button" onclick={() => onclose?.()}>{$t("llm.central.guard.close")}</button>
      {#if onproceed}
        <button type="button" class="button" class:danger={report.level === "block"} disabled={busy} onclick={press}>
          {#if report.level === "block"}
            {armed ? $t("llm.central.guard.proceed_confirm") : $t("llm.central.guard.proceed_anyway")}
          {:else}
            {proceedLabel || $t("llm.central.guard.proceed")}
          {/if}
        </button>
      {/if}
    </footer>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: grid;
    place-items: center;
    z-index: 950;
  }

  .dialog {
    width: min(760px, 94vw);
    max-height: 88vh;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: var(--hairline) solid var(--separator);
  }

  .titles {
    min-width: 0;
  }

  h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
  }

  .sub {
    margin: 2px 0 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .body {
    overflow: auto;
    padding: var(--space-3) var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .facts {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-5);
  }

  .facts dt {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .facts dd {
    margin: 0;
    font-size: var(--text-sm);
  }

  h3 {
    margin: 0 0 var(--space-2);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .hint,
  .note,
  .empty {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    line-height: var(--leading-base);
  }

  .warn {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--danger);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .cmd {
    border-radius: var(--radius-md);
    padding: var(--space-2);
    background: var(--surface-mut, var(--surface-hi));
    border-left: 3px solid var(--separator);
  }

  .cmd.bad {
    border-left-color: var(--danger);
  }

  .cmd-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .chip,
  .flag {
    padding: 0 var(--space-1);
    border-radius: var(--radius-sm);
    border: var(--hairline) solid var(--separator);
  }

  .where {
    font-size: var(--text-xs);
    color: var(--text-dim);
    word-break: break-all;
  }

  .code,
  .snippet {
    margin: var(--space-1) 0 0;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--surface);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 12em;
    overflow: auto;
  }

  .code {
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .codes {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin-top: var(--space-1);
  }

  .code-tag,
  .code-id {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .code-tag {
    padding: 0 var(--space-1);
    border-radius: var(--radius-sm);
    background: var(--surface);
  }

  .finding {
    display: grid;
    grid-template-columns: 5.5em 1fr;
    gap: var(--space-2);
    align-items: start;
  }

  .sev {
    font-size: var(--text-xs);
    text-align: center;
    padding: 1px var(--space-1);
    border-radius: var(--radius-sm);
    color: var(--text-dim);
    border: var(--hairline) solid var(--separator);
  }

  .sev.critical,
  .sev.high {
    border-color: transparent;
    background: color-mix(in srgb, var(--danger) 16%, transparent);
    color: var(--danger);
  }

  .sev.medium {
    border-color: transparent;
    background: color-mix(in srgb, var(--warning, var(--danger)) 16%, transparent);
    color: var(--warning, var(--danger));
  }

  .what {
    min-width: 0;
    font-size: var(--text-sm);
  }

  .rule {
    display: flex;
    gap: var(--space-2);
    align-items: baseline;
    flex-wrap: wrap;
  }

  .lowered {
    color: var(--text-dim);
    cursor: help;
  }

  .detail {
    font-size: var(--text-xs);
    color: var(--text-dim);
    word-break: break-word;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .link {
    align-self: flex-start;
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    font: inherit;
    font-size: var(--text-xs);
    cursor: pointer;
  }

  .foot {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-top: var(--hairline) solid var(--separator);
  }
</style>
