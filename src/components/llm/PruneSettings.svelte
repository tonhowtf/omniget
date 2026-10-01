<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";

  type Judge = "local" | "jev";
  type PruneStatus = {
    enabled: boolean;
    judge: Judge;
    has_key: boolean;
    omitted: number;
    min_tokens?: number;
    receipts?: { agent: string; receipt: Receipt }[];
  };
  type Receipt = {
    request: number;
    omitted: number;
    est_tokens_before: number;
    est_tokens_after: number;
    input_tokens: number | null;
  };

  let enabled = $state(false);
  let judge = $state<Judge>("local");
  let hasKey = $state(false);
  let omitted = $state(0);
  let minTokens = $state(50000);
  let receipts = $state<{ agent: string; receipt: Receipt }[]>([]);
  let keyInput = $state("");
  let busy = $state(false);
  let offline = $state(false);
  let saved = $state(false);
  let ready = $state(false);
  let confirmed = { enabled: false, judge: "local" as Judge, minTokens: 50000 };
  const judgeGroup = $props.id();

  async function refresh() {
    if (busy) return;
    busy = true;
    try {
      const status = await invoke<PruneStatus | null>("llm_prune_status");
      if (!status) throw new Error("ERR_PRUNE_STATUS");
      enabled = status.enabled;
      judge = status.judge === "jev" ? "jev" : "local";
      hasKey = status.has_key;
      omitted = status.omitted;
      minTokens = status.min_tokens ?? 50000;
      receipts = (status.receipts ?? []).slice().reverse();
      confirmed = { enabled, judge, minTokens };
      ready = true;
      offline = false;
    } catch {
      ready = false;
      offline = true;
    } finally { busy = false; }
  }

  onMount(refresh);

  async function push() {
    if (busy || !ready) return;
    if (!Number.isFinite(minTokens)) { minTokens = confirmed.minTokens; return; }
    minTokens = Math.min(1_000_000, Math.max(1_000, Math.round(minTokens)));
    busy = true;
    try {
      await invoke("llm_prune_set_config", { enabled, judge, minTokens });
      confirmed = { enabled, judge, minTokens };
      offline = false;
    } catch {
      ({ enabled, judge, minTokens } = confirmed);
      ready = false;
      offline = true;
    } finally {
      busy = false;
    }
  }

  function toggle() {
    if (busy || !ready) return;
    enabled = !enabled;
    void push();
  }

  function pick(next: Judge) {
    if (busy || !ready || judge === next) return;
    judge = next;
    void push();
  }

  async function saveKey() {
    const key = keyInput.trim();
    if (busy || key.length === 0) return;
    busy = true;
    try {
      await invoke("llm_prune_set_jev_key", { key });
      hasKey = true;
      saved = true;
      offline = false;
    } catch {
      offline = true;
    } finally {
      keyInput = "";
      busy = false;
    }
  }

  async function forgetKey() {
    if (busy) return;
    busy = true;
    try {
      await invoke("llm_prune_set_jev_key", { key: "" });
      hasKey = false;
      saved = false;
      offline = false;
    } catch {
      offline = true;
    } finally {
      busy = false;
    }
  }
</script>

<section class="prune">
  <div class="group">
    <div class="group-row">
      <div class="group-row-content">
        <div class="group-row-title">{$t("llm.prune.title")}</div>
        <div class="group-row-sub">{$t("llm.prune.subtitle")}</div>
      </div>
      <div class="group-row-trailing">
        <button
          class="toggle"
          class:on={enabled}
          type="button"
          role="switch"
          aria-checked={enabled}
          aria-label={$t("llm.prune.title")}
          disabled={busy || !ready}
          onclick={toggle}
        >
          <span class="toggle-knob"></span>
        </button>
      </div>
    </div>

    {#if enabled}
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("llm.prune.judge")}</div>
          <div class="group-row-sub">
            {judge === "jev" ? $t("llm.prune.judge_jev_desc") : $t("llm.prune.judge_local_desc")}
          </div>
        </div>
        <div class="group-row-trailing">
          <fieldset class="judge-choice">
            <legend class="field-label">{$t("llm.prune.judge")}</legend>
            <label><input type="radio" name={judgeGroup} checked={judge === "local"} disabled={busy || !ready} onchange={() => pick("local")} />{$t("llm.prune.judge_local")}</label>
            <label><input type="radio" name={judgeGroup} checked={judge === "jev"} disabled={busy || !ready} onchange={() => pick("jev")} />{$t("llm.prune.judge_jev")}</label>
          </fieldset>
        </div>
      </div>

      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("llm.prune.omitted")}</div>
          <div class="group-row-sub">{$t("llm.prune.omitted_count", { count: omitted })}</div>
        </div>
      </div>

      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("llm.prune.min_tokens")}</div>
          <div class="group-row-sub">{$t("llm.prune.min_tokens_desc")}</div>
        </div>
        <input
          class="input min-tokens"
          type="number"
          min="1000"
          step="1000"
          max="1000000"
          disabled={busy || !ready}
          bind:value={minTokens}
          onchange={() => void push()}
          aria-label={$t("llm.prune.min_tokens")}
        />
      </div>

      <div class="group-row stack">
        <div class="group-row-title">{$t("llm.prune.receipts")}</div>
        {#if receipts.length === 0}
          <div class="group-row-sub">{$t("llm.prune.receipts_empty")}</div>
        {:else}
          <ul class="receipts">
            {#each receipts as r (r.agent + r.receipt.request + r.receipt.est_tokens_before)}
              <li>
                <span class="receipt-agent">{r.agent}</span>
                <span>
                  {$t("llm.prune.receipt_line", {
                    request: r.receipt.request,
                    omitted: r.receipt.omitted,
                    before: r.receipt.est_tokens_before,
                    after: r.receipt.est_tokens_after,
                    billed: r.receipt.input_tokens ?? "—",
                  })}
                </span>
              </li>
            {/each}
          </ul>
        {/if}
        <button type="button" class="button" onclick={() => void refresh()}>{$t("llm.prune.refresh")}</button>
      </div>
    {/if}
  </div>

  {#if enabled && judge === "jev"}
    <div class="group">
      <div class="group-row stack">
        <p class="warning">{$t("llm.prune.privacy_warning")}</p>
        <label class="field">
          <span class="field-label">{$t("llm.prune.key_label")}</span>
          <input
            class="input mono"
            type="password"
            autocomplete="off"
            spellcheck="false"
            bind:value={keyInput}
            placeholder={hasKey ? $t("llm.prune.key_set") : "ts-…"}
            disabled={busy}
          />
          <span class="field-hint">{$t("llm.prune.key_hint")}</span>
        </label>
        <div class="row">
          <button
            type="button"
            class="button primary"
            disabled={busy || keyInput.trim().length === 0}
            onclick={saveKey}
          >
            {$t("llm.prune.key_save")}
          </button>
          {#if hasKey}
            <button type="button" class="button" disabled={busy} onclick={forgetKey}>
              {$t("llm.prune.key_forget")}
            </button>
          {/if}
        </div>
        {#if saved}
          <p class="hint">{$t("llm.prune.key_saved")}</p>
        {:else if !hasKey}
          <p class="hint">{$t("llm.prune.key_missing")}</p>
        {/if}
      </div>
    </div>
  {/if}

  {#if offline}
    <p class="hint danger" role="alert">{$t("llm.prune.save_failed")}</p>
    <button type="button" class="button" disabled={busy} onclick={refresh}>{$t("llm.surface.retry")}</button>
  {/if}
</section>

<style>
  .judge-choice { border: 0; margin: 0; padding: 0; display: flex; gap: var(--space-3); flex-wrap: wrap; }
  .judge-choice label { display: flex; align-items: center; gap: var(--space-2); font-size: var(--text-sm); }
  .prune {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .stack {
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-3);
  }

  .warning {
    margin: 0;
    padding: var(--space-3);
    border: var(--hairline) solid var(--warning, var(--danger));
    border-radius: var(--radius-md);
    color: var(--text);
    font-size: var(--text-sm);
    background: var(--accent-soft);
  }

  .row {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .danger {
    color: var(--danger);
  }

  @media (max-width: 720px) {
    .prune :global(.group-row) {
      flex-direction: column;
      align-items: stretch;
    }
  }
  .min-tokens {
    width: 9ch;
    text-align: right;
  }

  .receipts {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--space-1);
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    color: var(--text-dim);
  }

  .receipt-agent {
    color: var(--text);
    margin-right: var(--space-2);
  }
</style>
