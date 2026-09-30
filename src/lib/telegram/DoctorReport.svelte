<script lang="ts">
  import { t } from "$lib/i18n";
  import TgIcon from "./TgIcon.svelte";
  import type { DoctorReport } from "./api";

  let { report }: { report: DoctorReport } = $props();

  const ICON: Record<string, string> = { ok: "check", warn: "warn", fail: "x", skip: "chevron" };
</script>

<div class="doctor">
  <div class="verdict" data-verdict={report.verdict}>
    <TgIcon name={report.verdict === "connected" ? "check" : "warn"} size={18} />
    <div>
      <div class="v-title">{$t(`telegram.doctor.verdicts.${report.verdict}.title`)}</div>
      <div class="v-text">{$t(`telegram.doctor.verdicts.${report.verdict}.text`)}</div>
    </div>
  </div>
  <ol class="steps">
    {#each report.steps as step (step.id)}
      <li class="step" data-state={step.state}>
        <span class="mark"><TgIcon name={ICON[step.state] ?? "info"} size={12} /></span>
        <div class="s-body">
          <div class="s-title">{$t(`telegram.doctor.steps.${step.id}`)}</div>
          <div class="s-hint">{$t(`telegram.doctor.hints.${step.hint}`)}</div>
          {#if step.detail}<pre class="s-detail">{step.detail}</pre>{/if}
        </div>
      </li>
    {/each}
  </ol>
</div>

<style>
  .doctor {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .verdict {
    display: flex;
    gap: var(--space-3);
    align-items: flex-start;
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--warning) 12%, transparent);
    color: var(--warning);
  }

  .verdict[data-verdict="connected"] {
    background: color-mix(in srgb, var(--success) 12%, transparent);
    color: var(--success);
  }

  .verdict[data-verdict="no_internet"] {
    background: color-mix(in srgb, var(--danger) 10%, transparent);
    color: var(--danger);
  }

  .v-title {
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .v-text {
    margin-top: 2px;
    font-size: var(--text-sm);
    color: var(--text-muted);
    line-height: 1.5;
  }

  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .step {
    display: flex;
    gap: var(--space-3);
    align-items: flex-start;
  }

  .mark {
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-full);
    flex-shrink: 0;
    margin-top: 1px;
    background: var(--fill-2);
    color: var(--text-dim);
  }

  [data-state="ok"] .mark {
    background: color-mix(in srgb, var(--success) 18%, transparent);
    color: var(--success);
  }

  [data-state="warn"] .mark {
    background: color-mix(in srgb, var(--warning) 18%, transparent);
    color: var(--warning);
  }

  [data-state="fail"] .mark {
    background: color-mix(in srgb, var(--danger) 14%, transparent);
    color: var(--danger);
  }

  .s-body {
    min-width: 0;
    flex: 1;
  }

  .s-title {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }

  .s-hint {
    font-size: var(--text-sm);
    color: var(--text-muted);
    line-height: 1.5;
  }

  [data-state="skip"] .s-title,
  [data-state="skip"] .s-hint {
    color: var(--text-dim);
  }

  .s-detail {
    margin: var(--space-1) 0 0;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
    white-space: pre-wrap;
    word-break: break-all;
  }
</style>
