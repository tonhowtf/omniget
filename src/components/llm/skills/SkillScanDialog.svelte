<script lang="ts">
  import { t } from "$lib/i18n";
  import {
    SCAN_RISK_THRESHOLD,
    scanPress,
    scanScore,
    type PendingInstall,
  } from "$lib/stores/llm-skills-store.svelte";

  let {
    pending,
    busy = false,
    onconfirm,
    ondiscard,
  }: {
    pending: PendingInstall;
    busy?: boolean;
    onconfirm?: () => void;
    ondiscard?: () => void;
  } = $props();

  let confirming = $state<string | null>(null);

  let scan = $derived(pending.scan);
  let score = $derived(scanScore(scan));
  let issues = $derived(scan.status === "scanned" ? (scan.issues ?? []) : []);
  let findings = $derived(scan.status === "scanned" ? (scan.findings ?? issues.length) : 0);

  function press() {
    if (busy) return;
    const step = scanPress(pending.manifest.name, scan, confirming);
    confirming = step.confirming;
    if (step.proceed) onconfirm?.();
  }
</script>

<div class="overlay" role="presentation">
  <div
    class="dialog"
    role="alertdialog"
    aria-modal="true"
    aria-label={$t("llm.skills.scan_dialog_title")}
  >
    <header class="head">
      <h2>{$t("llm.skills.scan_dialog_title")}</h2>
      {#if score !== null}
        <span class="score" aria-hidden="true">{score}</span>
      {/if}
    </header>

    <div class="body">
      <p class="lede">
        {$t("llm.skills.scan_dialog_lede", {
          name: pending.manifest.name,
          score: String(score ?? "?"),
          threshold: String(SCAN_RISK_THRESHOLD),
        })}
      </p>

      {#if scan.status === "scanned"}
        <dl class="facts">
          <div>
            <dt>{$t("llm.skills.scan_severity")}</dt>
            <dd>{scan.severity ?? "—"}</dd>
          </div>
          <div>
            <dt>{$t("llm.skills.scan_recommendation")}</dt>
            <dd>{(scan.recommendation ?? "").replace(/_/g, " ") || "—"}</dd>
          </div>
          <div>
            <dt>{$t("llm.skills.scan_findings")}</dt>
            <dd>{findings}</dd>
          </div>
        </dl>
      {/if}

      {#if issues.length > 0}
        <ul class="issues">
          {#each issues as issue, i (issue.id ?? i)}
            <li>
              <span class="sev">{issue.severity ?? "—"}</span>
              <span class="what">{issue.message || issue.id || "—"}</span>
              {#if issue.file}
                <span class="where mono">
                  {issue.file}{#if issue.line}:{issue.line}{/if}
                </span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}

      <p class="note">{$t("llm.skills.scan_static_only")}</p>

      {#if confirming === pending.manifest.name}
        <p class="warn" role="alert">{$t("llm.skills.scan_confirm_warning")}</p>
      {/if}
    </div>

    <footer class="foot">
      <button type="button" class="button" disabled={busy} onclick={() => ondiscard?.()}>
        {$t("llm.skills.scan_discard")}
      </button>
      <button type="button" class="button danger" disabled={busy} onclick={press}>
        {#if busy}
          {$t("llm.skills.installing")}
        {:else if confirming === pending.manifest.name}
          {$t("llm.skills.scan_install_confirm")}
        {:else}
          {$t("llm.skills.scan_install_anyway")}
        {/if}
      </button>
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
    width: min(560px, 92vw);
    max-height: 86vh;
    overflow: auto;
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
  }

  .head h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
  }

  .score {
    padding: 2px var(--space-2);
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--danger) 16%, transparent);
    color: var(--danger);
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-weight: 600;
  }

  .body {
    padding: 0 var(--space-4) var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .lede {
    margin: 0;
    font-size: var(--text-sm);
    line-height: var(--leading-base);
  }

  .facts {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-4);
  }

  .facts dt {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .facts dd {
    margin: 0;
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .issues {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-height: 30vh;
    overflow-y: auto;
  }

  .issues li {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-2);
    padding: var(--space-2);
    border-radius: var(--radius-md);
    box-shadow: inset 0 0 0 var(--hairline) var(--separator);
  }

  .sev {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--danger);
  }

  .what {
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .where {
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .note {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .warn {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--danger);
  }

  .foot {
    padding: var(--space-3) var(--space-4);
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
