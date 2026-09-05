<script lang="ts">
  /** Humanizar texto: o skill blader/humanizer (MIT) rodando na IA configurada. */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/tools/rt";

  type AiView = { provider: string; model: string; local_base_url: string; has_openai_key: boolean; has_anthropic_key: boolean };

  let ai = $state<AiView | null>(null);
  let text = $state("");
  let sample = $state("");
  let showSample = $state(false);
  let output = $state("");
  let busy = $state(false);
  let copied = $state(false);

  const MAX = 40000;
  let configured = $derived(!!ai && ai.provider !== "none" && ai.model.trim() !== "");
  let tooLong = $derived(text.length > MAX);
  let canRun = $derived(configured && !busy && text.trim().length > 0 && !tooLong);

  onMount(async () => {
    try {
      ai = await invoke<AiView>("ai_get_config");
    } catch {
      ai = null;
    }
  });

  async function run() {
    if (!canRun) return;
    busy = true;
    output = "";
    try {
      output = await invoke<string>("tool_humanize", { text, sample: showSample && sample.trim() ? sample : null });
      showToast("success", $t("tools.common.done") as string);
    } catch (e) {
      const msg = errText(e);
      showToast("error", msg === "ai_not_configured" ? ($t("tools.humanize.not_configured") as string) : msg);
    } finally {
      busy = false;
    }
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(output);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  function useAsInput() {
    text = output;
    output = "";
  }
</script>

<div class="tool">
  {#if ai && !configured}
    <div class="group">
      <div class="group-row">
        <div class="group-row-sub">
          {$t("tools.humanize.not_configured")}
          <a href="/settings">{$t("tools.humanize.open_settings")}</a>
        </div>
      </div>
    </div>
  {/if}

  <section>
    <span class="group-label">{$t("tools.humanize.input")}</span>
    <div class="group pad">
      <textarea
        class="input area"
        bind:value={text}
        placeholder={$t("tools.humanize.placeholder")}
        rows="10"
        spellcheck="false"
      ></textarea>
      <div class="meta">
        <span class="count" class:over={tooLong}>{text.length.toLocaleString()} / {MAX.toLocaleString()}</span>
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => (showSample = !showSample)} aria-expanded={showSample}>
          {showSample ? $t("tools.humanize.sample_hide") : $t("tools.humanize.sample_show")}
        </button>
      </div>
      {#if showSample}
        <p class="hint">{$t("tools.humanize.sample_hint")}</p>
        <textarea class="input area" bind:value={sample} placeholder={$t("tools.humanize.sample_placeholder")} rows="5" spellcheck="false"></textarea>
      {/if}
    </div>
    <div class="actions">
      <span class="model">{#if configured}{ai?.model}{/if}</span>
      <button type="button" class="btn btn-primary" disabled={!canRun} onclick={run}>
        {busy ? $t("tools.common.working") : $t("tools.humanize.run")}
      </button>
    </div>
  </section>

  {#if output}
    <section>
      <span class="group-label">{$t("tools.common.result")}</span>
      <div class="group pad">
        <textarea class="input area out" readonly value={output} rows="12"></textarea>
        <div class="meta">
          <span class="count">{output.length.toLocaleString()}</span>
          <span class="btn-row">
            <button type="button" class="btn btn-secondary btn-sm" onclick={useAsInput}>{$t("tools.humanize.again")}</button>
            <button type="button" class="btn btn-secondary btn-sm" onclick={copy}>{copied ? $t("tools.common.copied") : $t("tools.common.copy")}</button>
          </span>
        </div>
      </div>
    </section>
  {/if}

  <p class="credit">{$t("tools.humanize.credit")}</p>
</div>

<style>
  .tool {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }
  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .group.pad {
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .area {
    width: 100%;
    height: auto;
    min-height: 120px;
    resize: vertical;
    line-height: 1.5;
    font-family: var(--font-body);
  }
  .area.out {
    background: var(--fill-1);
  }
  .meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .count {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .count.over {
    color: var(--danger);
    font-weight: 600;
  }
  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-3);
  }
  .model {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-family: var(--font-mono);
  }
  .btn-row {
    display: inline-flex;
    gap: var(--space-2);
  }
  .credit {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
