<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import {
    diffBodies,
    estimateRound,
    formatCost,
    getEstimate,
    getRound,
    getWireError,
    isLoading,
    isRunning,
    loadLast,
    MAX_OUTPUT_TOKENS,
    prettyBody,
    PROBE_PARAMS,
    runRound,
    tally,
    verdictKey,
    verdictTone,
    type ProbeParam,
    type ProbeResult,
  } from "$lib/stores/llm-wire-store.svelte";

  let {
    provider = "",
    model = "",
  }: { provider?: string; model?: string } = $props();

  let selected = $state<ProbeParam[]>(["temperature", "top_p", "max_tokens", "stop"]);
  let open = $state<string | null>(null);

  let round = $derived(getRound());
  let estimate = $derived(getEstimate());
  let running = $derived(isRunning());
  let loading = $derived(isLoading());
  let error = $derived(getWireError());
  let counts = $derived(tally(round));
  let ready = $derived(provider.length > 0 && model.length > 0 && selected.length > 0);

  onMount(() => {
    void loadLast();
  });

  function toggleParam(param: ProbeParam) {
    selected = selected.includes(param)
      ? selected.filter((p) => p !== param)
      : [...selected, param];
  }

  async function preview() {
    if (!ready) return;
    await estimateRound(provider, model, selected);
  }

  async function start() {
    if (!ready || running) return;
    await runRound(provider, model, selected);
  }

  function toggleRow(result: ProbeResult) {
    open = open === result.param ? null : result.param;
  }
</script>

<section class="wire-probe">
  <header class="wire-head">
    <div class="setting-col">
      <span class="setting-label">{$t("llm.wire.title")}</span>
      <span class="setting-path">{$t("llm.wire.desc")}</span>
    </div>
  </header>

  <div class="card">
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t("llm.wire.params")}</span>
        <span class="setting-path">{$t("llm.wire.params_desc")}</span>
      </div>
      <div class="chips">
        {#each PROBE_PARAMS as param}
          <button
            type="button"
            class="chip"
            class:on={selected.includes(param)}
            aria-pressed={selected.includes(param)}
            onclick={() => toggleParam(param)}
            disabled={running}
          >
            {param}
          </button>
        {/each}
      </div>
    </div>

    <div class="divider"></div>

    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">
          {$t("llm.wire.cost", {
            requests: estimate?.requests ?? selected.length * 2,
            tokens: MAX_OUTPUT_TOKENS,
          })}
        </span>
        <span class="setting-path">
          {estimate
            ? $t("llm.wire.cost_estimate", { cost: formatCost(estimate.cost_usd) })
            : $t("llm.wire.cost_unknown")}
        </span>
      </div>
      <div class="actions">
        <button class="btn btn-secondary" onclick={preview} disabled={!ready || running}>
          {$t("llm.wire.estimate")}
        </button>
        <button class="btn btn-primary" onclick={start} disabled={!ready || running}>
          {running
            ? $t("llm.wire.running")
            : round
              ? $t("llm.wire.rerun")
              : $t("llm.wire.run")}
        </button>
      </div>
    </div>

    {#if !ready}
      <div class="divider"></div>
      <p class="setting-path note">{$t("llm.wire.pick_model")}</p>
    {/if}

    {#if error}
      <div class="divider"></div>
      <p class="setting-path error">{$t(error)}</p>
    {/if}
  </div>

  {#if loading && !round}
    <p class="setting-path note">{$t("llm.wire.loading")}</p>
  {:else if round}
    <div class="card results">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{round.provider} · {round.model}</span>
          <span class="setting-path">
            {$t("llm.wire.summary", {
              proven: counts.proven,
              sent: counts.sent,
              ignored: counts.ignored,
              unsupported: counts.unsupported,
            })} · {formatCost(round.cost_usd)}
          </span>
        </div>
      </div>

      {#each round.results as result (result.param)}
        <div class="divider"></div>
        <div class="result">
          <button type="button" class="result-head" onclick={() => toggleRow(result)}>
            <span class="param">{result.param}</span>
            <span class="badge" data-tone={verdictTone(result.verdict)}>
              {$t(verdictKey(result.verdict))}
            </span>
            <span class="detail">{result.detail}</span>
            <span class="caret" aria-hidden="true">{open === result.param ? "−" : "+"}</span>
          </button>

          {#if open === result.param}
            <div class="evidence">
              <span class="ev-label">{$t("llm.wire.sent_diff")}</span>
              <pre class="diff">{#each diffBodies(result.sent_a, result.sent_b) as line}<span
                    class="line"
                    data-kind={line.kind}>{line.kind === "same" ? "  " : line.kind === "a" ? "- " : "+ "}{line.text}</span
                  >{/each}</pre>

              <div class="replies">
                <div class="reply">
                  <span class="ev-label">{$t("llm.wire.reply_a")}</span>
                  <pre>{result.reply_a || "—"}</pre>
                </div>
                <div class="reply">
                  <span class="ev-label">{$t("llm.wire.reply_b")}</span>
                  <pre>{result.reply_b || "—"}</pre>
                </div>
              </div>

              {#if !result.differs}
                <p class="setting-path note">{$t("llm.wire.same_reply")}</p>
              {/if}

              <details class="raw">
                <summary>{$t("llm.wire.raw_bodies")}</summary>
                <pre>{prettyBody(result.sent_a)}</pre>
                <pre>{prettyBody(result.sent_b)}</pre>
              </details>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {:else if !loading}
    <p class="setting-path note">{$t("llm.wire.empty")}</p>
  {/if}
</section>

<style>
  .wire-probe {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    justify-content: flex-end;
  }

  .chip {
    border: 1px solid var(--border-color);
    background: transparent;
    color: var(--text-secondary);
    border-radius: 999px;
    padding: 3px 10px;
    font-size: 12px;
    font-family: var(--font-mono, ui-monospace, monospace);
    cursor: pointer;
  }

  .chip.on {
    border-color: var(--accent-color, #c77dff);
    color: var(--text-primary);
    background: color-mix(in srgb, var(--accent-color, #c77dff) 16%, transparent);
  }

  .chip:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .note {
    margin: 0;
    padding: 4px 0;
  }

  .error {
    margin: 0;
    padding: 4px 0;
    color: var(--danger-color, #ff6b6b);
  }

  .result-head {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 0;
    background: transparent;
    border: 0;
    cursor: pointer;
    text-align: left;
    color: inherit;
    min-width: 0;
  }

  .param {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 13px;
    color: var(--text-primary);
    flex: 0 0 auto;
  }

  .badge {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 2px 8px;
    border-radius: 999px;
    flex: 0 0 auto;
  }

  .badge[data-tone="ok"] {
    background: color-mix(in srgb, #4cd964 22%, transparent);
    color: #2e8b3d;
  }

  .badge[data-tone="warn"] {
    background: color-mix(in srgb, #ffb340 24%, transparent);
    color: #8a5a00;
  }

  .badge[data-tone="bad"] {
    background: color-mix(in srgb, #ff6b6b 22%, transparent);
    color: #a32f2f;
  }

  .badge[data-tone="muted"] {
    background: color-mix(in srgb, var(--text-secondary) 16%, transparent);
    color: var(--text-secondary);
  }

  .detail {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .caret {
    flex: 0 0 auto;
    color: var(--text-secondary);
  }

  .evidence {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-bottom: 8px;
  }

  .ev-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
  }

  pre {
    margin: 0;
    padding: 8px 10px;
    border-radius: 8px;
    background: var(--bg-secondary, rgba(127, 127, 127, 0.08));
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
    overflow-x: auto;
  }

  .diff .line {
    display: block;
  }

  .diff .line[data-kind="same"] {
    color: var(--text-secondary);
  }

  .diff .line[data-kind="a"] {
    color: #a32f2f;
  }

  .diff .line[data-kind="b"] {
    color: #2e8b3d;
  }

  .replies {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 8px;
  }

  .reply {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .raw summary {
    font-size: 12px;
    color: var(--text-secondary);
    cursor: pointer;
  }

  .raw pre {
    margin-top: 6px;
  }
</style>
