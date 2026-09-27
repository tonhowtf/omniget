<script lang="ts">
  // Diagnóstico de uma ferramenta. Login: comando de status da própria
  // ferramenta ou só a presença de um arquivo; nenhuma credencial é lida.
  import { t } from "$lib/i18n";
  import type { DoctorReport, PathCheck } from "$lib/central/clitools";

  let { report, onclose }: { report: DoctorReport; onclose: () => void } = $props();
  let det = $derived(report.detection);
  const authTag = { present: "tag-success", absent: "tag-danger", unknown: "" } as const;

  let sections = $derived<{ key: string; list: PathCheck[] }[]>([
    { key: "config", list: report.config },
    { key: "logs", list: report.logs },
    { key: "deprecated", list: report.deprecated },
  ]);

  function mark(p: PathCheck): string {
    return p.exists === null ? "·" : p.exists ? "✓" : "—";
  }
</script>

<section class="surface-card panel">
  <header class="panel-head">
    <h2>{$t("llm.central.tools.doctor")} · {report.name}</h2>
    <button class="button" type="button" onclick={onclose}>{$t("llm.central.tools.close")}</button>
  </header>

  {#if report.issues.length > 0}
    <ul class="issues">
      {#each report.issues as i}<li>{i}</li>{/each}
    </ul>
  {:else}
    <p class="ok">{$t("llm.central.tools.doctor_ok")}</p>
  {/if}

  <dl class="grid">
    <dt>{$t("llm.central.tools.installed")}</dt>
    <dd>{det.installed ? (det.version ?? det.app_version ?? "?") : $t("llm.central.tools.not_installed")}</dd>
    <dt>{$t("llm.central.tools.latest")}</dt>
    <dd>{report.latest.latest ?? "—"} <span class="dim">({report.latest.source})</span></dd>
    {#if det.resolved_path}
      <dt>{$t("llm.central.tools.path")}</dt>
      <dd><code>{det.resolved_path}</code></dd>
      <dt>{$t("llm.central.tools.real_path")}</dt>
      <dd><code>{det.real_path}</code></dd>
    {/if}
    {#if det.app_path}
      <dt>{$t("llm.central.tools.app")}</dt>
      <dd><code>{det.app_path}</code></dd>
    {/if}
    <dt>{$t("llm.central.tools.owner")}</dt>
    <dd>
      {$t(`llm.central.tools.owner_method.${det.owner.method}`)} {det.owner.proven ? "✓" : "?"}
      <span class="dim">{det.owner.evidence}</span>
    </dd>
    <dt>{$t("llm.central.tools.auth")}</dt>
    <dd>
      <span class={`tag ${authTag[report.auth.state]}`}>{$t(`llm.central.tools.auth_state.${report.auth.state}`)}</span>
      <span class="dim">{report.auth.detail}</span>
      {#if report.auth.output}<code class="out">{report.auth.output}</code>{/if}
    </dd>
    {#if report.config_env}
      <dt>{report.config_env[0]}</dt>
      <dd><code>{report.config_env[1] ?? $t("llm.central.tools.unset")}</code></dd>
    {/if}
  </dl>

  {#each sections as sec (sec.key)}
    {#if sec.list.length > 0}
      <h3>{$t(`llm.central.tools.section.${sec.key}`)}</h3>
      <ul class="paths">
        {#each sec.list as p (p.path)}
          <li class:hit={sec.key === "deprecated" && p.exists}>
            <span class="mark">{mark(p)}</span>
            <code>{p.path}</code>
            {#if p.note}<span class="dim">{p.note}</span>{/if}
          </li>
        {/each}
      </ul>
    {/if}
  {/each}
</section>

<style>
  .panel {
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h2 {
    margin: 0;
    font-size: var(--text-md);
  }
  h3 {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
  }
  .issues {
    margin: 0;
    padding-left: 1.2em;
    color: var(--warning);
    font-size: var(--text-sm);
  }
  .ok {
    margin: 0;
    color: var(--success);
    font-size: var(--text-sm);
  }
  .grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px var(--space-3);
    margin: 0;
    font-size: var(--text-sm);
  }
  dt {
    color: var(--text-dim);
  }
  dd {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    min-width: 0;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    word-break: break-all;
    user-select: text;
  }
  .out {
    flex-basis: 100%;
    color: var(--text-muted);
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--text-caption);
  }
  .paths {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--text-sm);
  }
  .paths li {
    display: flex;
    gap: var(--space-2);
    align-items: baseline;
    flex-wrap: wrap;
  }
  .paths li.hit code {
    color: var(--warning);
  }
  .mark {
    width: 1em;
    color: var(--text-dim);
  }
</style>
