<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SettingGroup from "./rebrand/SettingGroup.svelte";
  import { ruleQualityOptions } from "./downloads/download-options";
  import SettingsEmptyState from "./rebrand/SettingsEmptyState.svelte";

  type Condition =
    | { kind: "host_is"; value: string }
    | { kind: "url_contains"; value: string }
    | { kind: "platform_is"; value: string };

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

  const RULE_QUALITY_KEYS: Record<string, string> = {
    best: "omnibox.quality_best",
    "1080p": "omnibox.quality_1080p",
    "720p": "omnibox.quality_720p",
    "480p": "omnibox.quality_480p",
    "360p": "omnibox.quality_360p",
  };

  let rules = $state<Rule[]>([]);
  let loading = $state(true);
  let loadError = $state(false);
  let saveFailed = $state(false);
  let saving = $state(false);
  let testing = $state(false);
  let testError = $state(false);
  let removed = $state<{ rule: Rule; index: number } | null>(null);
  let savedRules = $state("[]");
  let dirty = $derived(JSON.stringify(rules) !== savedRules);
  let saveRequested = false;
  let savePromise: Promise<boolean> | null = null;
  let testUrl = $state("");
  let testResult = $state<string | null>(null);

  function kindOf(c: Condition): CondKind {
    if (c.kind === "host_is") return "HostIs";
    if (c.kind === "url_contains") return "UrlContains";
    return "PlatformIs";
  }

  function valueOf(c: Condition): string {
    return c.value;
  }

  function buildCondition(kind: CondKind, value: string): Condition {
    if (kind === "HostIs") return { kind: "host_is", value };
    if (kind === "UrlContains") return { kind: "url_contains", value };
    return { kind: "platform_is", value };
  }

  async function load() {
    loading = true;
    loadError = false;
    try {
      rules = await invoke<Rule[]>("list_rules");
      savedRules = JSON.stringify(rules);
    } catch {
      loadError = true;
    } finally {
      loading = false;
    }
  }

  function persist(): Promise<boolean> {
    if (loadError || loading) return Promise.resolve(false);
    saveRequested = true;
    if (savePromise) return savePromise;
    saving = true;
    saveFailed = false;
    savePromise = (async () => {
      while (saveRequested) {
        saveRequested = false;
        const snapshot = JSON.stringify(rules);
        try {
          await invoke("save_rules", { rules: JSON.parse(snapshot) });
          savedRules = snapshot;
        } catch {
          saveFailed = true;
          return false;
        }
      }
      return true;
    })().finally(() => {
      saving = false;
      savePromise = null;
    });
    return savePromise;
  }

  function addRule() {
    rules = [
      ...rules,
      {
        enabled: true,
        name: "",
        when: { kind: "host_is", value: "" },
        then: {},
      },
    ];
  }

  function removeRule(i: number) {
    if (saving) return;
    removed = { rule: structuredClone($state.snapshot(rules[i])), index: i };
    rules = rules.filter((_, idx) => idx !== i);
    void persist();
  }

  function undoRemove() {
    if (!removed || saving) return;
    const { rule, index } = removed;
    rules = [...rules.slice(0, index), rule, ...rules.slice(index)];
    removed = null;
    void persist();
  }

  function move(i: number, delta: number) {
    const j = i + delta;
    if (j < 0 || j >= rules.length) return;
    const copy = [...rules];
    [copy[i], copy[j]] = [copy[j], copy[i]];
    rules = copy;
    void persist();
  }

  async function pickFolder(i: number) {
    const rule = rules[i];
    try {
      const chosen = await open({ directory: true, title: $t("settings.rules.pick_folder") as string });
      if (typeof chosen !== "string" || !rules.includes(rule)) return;
      rule.then.output_dir = chosen;
      void persist();
    } catch {
      showToast("error", $t("common.error"));
    }
  }

  async function runTest() {
    if (!testUrl.trim() || testing || loadError || loading) return;
    testResult = null;
    testError = false;
    testing = true;
    try {
      if ((dirty || saving) && !(await persist())) return;
      const hit = await invoke<Rule | null>("preview_rule_match", { url: testUrl.trim(), platform: null });
      testResult = hit ? hit.name || $t("settings.rules.unnamed") : $t("settings.rules.test_no_match");
    } catch {
      testError = true;
    } finally {
      testing = false;
    }
  }

  onMount(load);
</script>

<SettingGroup id="rules" icon="blocks" title={$t("settings.rules.title")} description={$t("settings.rules.description")} advanced>
  <div class="card">
    {#if loading}
      <p class="rules-empty" role="status" aria-busy="true">{$t("settings.rebrand.state.loading")}</p>
    {:else if loadError}
      <SettingsEmptyState error title={$t("settings.rules.load_failed")} onRetry={() => void load()} />
    {:else if rules.length === 0}
      <SettingsEmptyState title={$t("settings.rules.empty_title")} hint={$t("settings.rules.empty_hint")} />
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
              aria-label={$t("settings.rules.name_placeholder")}
              type="text"
              bind:value={rule.name}
              onblur={() => persist()}
              placeholder={$t("settings.rules.name_placeholder") as string}
            />

            <div class="rule-order">
              <button
                type="button"
                class="icon-btn"
                disabled={saving || i === 0}
                onclick={() => move(i, -1)}
                aria-label={$t("settings.rules.move_up") as string}
              >↑</button>
              <button
                type="button"
                class="icon-btn"
                disabled={saving || i === rules.length - 1}
                onclick={() => move(i, 1)}
                aria-label={$t("settings.rules.move_down") as string}
              >↓</button>
              <button
                type="button"
                class="icon-btn danger"
                disabled={saving}
                onclick={() => removeRule(i)}
                aria-label={$t("settings.rules.remove") as string}
              >×</button>
            </div>
          </div>

          <div class="rule-body">
            <div class="rule-field">
              <span class="rule-label">{$t("settings.rules.when")}</span>
              <select
                aria-label={$t("settings.rules.when")} value={kindOf(rule.when)}
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
                aria-label={$t("settings.rules.value_placeholder")} value={valueOf(rule.when)}
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
              <select
                class="select rule-quality"
                aria-label={$t("settings.download.video_quality") as string}
                value={rule.then.quality ?? ""}
                onchange={(e) => {
                  rule.then.quality = e.currentTarget.value || null;
                  void persist();
                }}
              >
                <option value="">{$t("settings.downloads.rules.quality_any")}</option>
                {#each ruleQualityOptions(rule.then.quality) as q (q)}
                  <option value={q}>{RULE_QUALITY_KEYS[q] ? $t(RULE_QUALITY_KEYS[q]) : q}</option>
                {/each}
              </select>
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
    <button type="button" class="button" data-setting-key="settings.rules.add" disabled={loading || loadError || saving} onclick={addRule}>
      {$t("settings.rules.add")}
    </button>
    {#if dirty || saving}
      <button type="button" class="button" disabled={saving} onclick={persist}>
        {saving ? $t("settings.ai.saving") : $t("settings.ai.save")}
      </button>
    {/if}
  </div>
  {#if saveFailed}<p class="feedback-error" role="alert">{$t("settings.rules.save_failed")}</p>{/if}
  {#if removed}
    <div class="rules-actions" role="status">
      <span>{$t("settings.rules.removed", { name: removed.rule.name || $t("settings.rules.unnamed") })}</span>
      <button type="button" class="button" disabled={saving} onclick={undoRemove}>{$t("settings.rules.undo_remove")}</button>
    </div>
  {/if}

  <div class="card rules-test" data-setting-key="settings.rules.test_title">
    <label class="rule-label" for="rules-test-url">{$t("settings.rules.test_title")}</label>
    <input
      type="text"
      id="rules-test-url" bind:value={testUrl}
      placeholder={$t("settings.rules.test_placeholder") as string}
    />
    <button type="button" class="button ghost" disabled={testing || loading || loadError || !testUrl.trim()} onclick={runTest}>
      {testing ? $t("common.loading") : $t("settings.rules.test_run")}
    </button>
    {#if testError}<span class="feedback-error" role="alert">{$t("settings.rules.test_failed")}</span>{/if}
    {#if testResult}
      <span class="test-result" role="status">{testResult}</span>
    {/if}
  </div>
</SettingGroup>

<style>
  .feedback-error { color: var(--error); }

  .rules-empty {
    padding: var(--space-5, 16px);
    text-align: center;
  }

  .rule-row {
    padding: var(--space-3, 12px);
    border-bottom: none;
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
    border: none;
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
    color: var(--text-muted);
    min-width: 5ch;
  }

  .rule-field input[type="text"],
  .rule-field select {
    background: var(--bg);
    border: none;
    border-radius: var(--radius-sm, 6px);
    color: var(--text);
    font: inherit;
    padding: 4px 8px;
  }

  .rule-quality {
    max-width: 22ch;
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
    border: none;
    border-radius: var(--radius-sm, 6px);
    color: var(--text);
    font: inherit;
    padding: 4px 8px;
  }

  .test-result {
    font-size: var(--text-sm, 0.875rem);
    color: var(--text-muted);
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
