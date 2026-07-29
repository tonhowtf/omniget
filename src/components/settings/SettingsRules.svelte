<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Condition =
    | { HostIs: { value: string } }
    | { UrlContains: { value: string } }
    | { PlatformIs: { value: string } };

  type Rule = {
    enabled: boolean;
    name: string;
    when: Condition;
    then: {
      output_dir?: string | null;
      quality?: string | null;
      audio_only?: boolean | null;
      subtitles?: boolean | null;
      tags?: string[];
    };
  };

  type CondKind = "HostIs" | "UrlContains" | "PlatformIs";

  let rules = $state<Rule[]>([]);
  let loading = $state(true);
  let testUrl = $state("");
  let testResult = $state<string | null>(null);

  function kindOf(c: Condition): CondKind {
    if ("HostIs" in c) return "HostIs";
    if ("UrlContains" in c) return "UrlContains";
    return "PlatformIs";
  }

  function valueOf(c: Condition): string {
    if ("HostIs" in c) return c.HostIs.value;
    if ("UrlContains" in c) return c.UrlContains.value;
    return c.PlatformIs.value;
  }

  function buildCondition(kind: CondKind, value: string): Condition {
    if (kind === "HostIs") return { HostIs: { value } };
    if (kind === "UrlContains") return { UrlContains: { value } };
    return { PlatformIs: { value } };
  }

  async function load() {
    loading = true;
    try {
      rules = await invoke<Rule[]>("list_rules");
    } catch {
      rules = [];
    } finally {
      loading = false;
    }
  }

  async function persist() {
    try {
      await invoke("save_rules", { rules });
      showToast("success", $t("settings.rules.saved") as string);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", msg);
    }
  }

  function addRule() {
    rules = [
      ...rules,
      {
        enabled: true,
        name: "",
        when: { HostIs: { value: "" } },
        then: {},
      },
    ];
  }

  function removeRule(i: number) {
    rules = rules.filter((_, idx) => idx !== i);
    void persist();
  }

  // A ordem é a prioridade: a primeira regra que casa vence. Sem poder mover,
  // a única saída seria apagar e recriar na ordem certa.
  function move(i: number, delta: number) {
    const j = i + delta;
    if (j < 0 || j >= rules.length) return;
    const copy = [...rules];
    [copy[i], copy[j]] = [copy[j], copy[i]];
    rules = copy;
    void persist();
  }

  async function pickFolder(i: number) {
    const chosen = await open({ directory: true, title: $t("settings.rules.pick_folder") as string });
    if (typeof chosen !== "string") return;
    rules[i].then.output_dir = chosen;
    void persist();
  }

  async function runTest() {
    testResult = null;
    if (!testUrl.trim()) return;
    try {
      const hit = await invoke<Rule | null>("preview_rule_match", { url: testUrl, platform: null });
      testResult = hit ? hit.name : ($t("settings.rules.test_no_match") as string);
    } catch {
      testResult = $t("settings.rules.test_no_match") as string;
    }
  }

  $effect(() => {
    void load();
  });
</script>

<section class="section">
  <h5 class="section-title">{$t("settings.rules.title")}</h5>
  <p class="muted">{$t("settings.rules.description")}</p>

  <div class="card">
    {#if loading}
      <div class="rules-empty">{$t("common.loading")}</div>
    {:else if rules.length === 0}
      <!-- Empty state que direciona a primeira ação de valor, em vez de card vazio. -->
      <div class="rules-empty">
        <p>{$t("settings.rules.empty_title")}</p>
        <p class="muted small">{$t("settings.rules.empty_hint")}</p>
      </div>
    {:else}
      {#each rules as rule, i (i)}
        <div class="rule-row">
          <div class="rule-head">
            <label class="rule-toggle">
              <input
                type="checkbox"
                bind:checked={rule.enabled}
                onchange={() => persist()}
              />
              <span class="visually-hidden">{$t("settings.rules.enabled")}</span>
            </label>

            <input
              class="rule-name"
              type="text"
              bind:value={rule.name}
              onblur={() => persist()}
              placeholder={$t("settings.rules.name_placeholder") as string}
            />

            <div class="rule-order">
              <button
                type="button"
                class="icon-btn"
                disabled={i === 0}
                onclick={() => move(i, -1)}
                aria-label={$t("settings.rules.move_up") as string}
              >↑</button>
              <button
                type="button"
                class="icon-btn"
                disabled={i === rules.length - 1}
                onclick={() => move(i, 1)}
                aria-label={$t("settings.rules.move_down") as string}
              >↓</button>
              <button
                type="button"
                class="icon-btn danger"
                onclick={() => removeRule(i)}
                aria-label={$t("settings.rules.remove") as string}
              >×</button>
            </div>
          </div>

          <div class="rule-body">
            <div class="rule-field">
              <span class="rule-label">{$t("settings.rules.when")}</span>
              <select
                value={kindOf(rule.when)}
                onchange={(e) => {
                  rule.when = buildCondition(e.currentTarget.value as CondKind, valueOf(rule.when));
                  void persist();
                }}
              >
                <option value="HostIs">{$t("settings.rules.cond_host")}</option>
                <option value="UrlContains">{$t("settings.rules.cond_contains")}</option>
                <option value="PlatformIs">{$t("settings.rules.cond_platform")}</option>
              </select>
              <input
                type="text"
                value={valueOf(rule.when)}
                oninput={(e) => {
                  rule.when = buildCondition(kindOf(rule.when), e.currentTarget.value);
                }}
                onblur={() => persist()}
                placeholder={$t("settings.rules.value_placeholder") as string}
              />
            </div>

            <div class="rule-field">
              <span class="rule-label">{$t("settings.rules.then")}</span>
              <button type="button" class="button ghost" onclick={() => pickFolder(i)}>
                {rule.then.output_dir ?? $t("settings.rules.choose_folder")}
              </button>
              <input
                class="rule-quality"
                type="text"
                bind:value={rule.then.quality}
                onblur={() => persist()}
                placeholder={$t("settings.rules.quality_placeholder") as string}
              />
              <label class="rule-check">
                <input
                  type="checkbox"
                  checked={rule.then.audio_only ?? false}
                  onchange={(e) => {
                    rule.then.audio_only = e.currentTarget.checked;
                    void persist();
                  }}
                />
                {$t("settings.rules.audio_only")}
              </label>
            </div>
          </div>
        </div>
      {/each}
    {/if}
  </div>

  <div class="rules-actions">
    <button type="button" class="button" onclick={addRule}>
      {$t("settings.rules.add")}
    </button>
  </div>

  <!-- Responder "esta regra pega o que eu espero?" antes de descobrir no
       download errado. -->
  <div class="card rules-test">
    <span class="rule-label">{$t("settings.rules.test_title")}</span>
    <input
      type="text"
      bind:value={testUrl}
      placeholder={$t("settings.rules.test_placeholder") as string}
    />
    <button type="button" class="button ghost" onclick={runTest}>
      {$t("settings.rules.test_run")}
    </button>
    {#if testResult}
      <span class="test-result">{testResult}</span>
    {/if}
  </div>
</section>

<style>
  .rules-empty {
    padding: var(--space-5, 16px);
    text-align: center;
  }

  .rule-row {
    padding: var(--space-3, 12px);
    border-bottom: 1px solid var(--border);
  }

  .rule-row:last-child {
    border-bottom: none;
  }

  .rule-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .rule-name {
    flex: 1;
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-sm, 6px);
    color: var(--text);
    font: inherit;
    font-weight: 600;
    padding: 4px 8px;
  }

  .rule-name:hover,
  .rule-name:focus {
    border-color: var(--border);
    outline: none;
  }

  .rule-order {
    display: flex;
    gap: 4px;
  }

  .icon-btn {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm, 6px);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    font: inherit;
    line-height: 1;
    padding: 4px 8px;
  }

  .icon-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .icon-btn.danger:hover:not(:disabled) {
    border-color: var(--danger);
    color: var(--danger);
  }

  .rule-body {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }

  .rule-field {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .rule-label {
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-secondary);
    min-width: 5ch;
  }

  .rule-field input[type="text"],
  .rule-field select {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm, 6px);
    color: var(--text);
    font: inherit;
    padding: 4px 8px;
  }

  .rule-quality {
    max-width: 12ch;
  }

  .rule-check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm, 0.875rem);
  }

  .rules-actions {
    margin-top: var(--space-3, 12px);
  }

  .rules-test {
    margin-top: var(--space-5, 16px);
    padding: var(--space-3, 12px);
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .rules-test input {
    flex: 1;
    min-width: 20ch;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm, 6px);
    color: var(--text);
    font: inherit;
    padding: 4px 8px;
  }

  .test-result {
    font-size: var(--text-sm, 0.875rem);
    color: var(--text-secondary);
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
