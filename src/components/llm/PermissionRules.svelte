<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { emptyPermissionRuleDraft, normalizePermissionRule, type PermissionRuleAction } from "$lib/llm/permission-rules";

  type Action = PermissionRuleAction;
  type Rule = { tool: string; pattern: string; action: Action };

  let { agentId = null, refreshKey = 0 }: { agentId?: string | null; refreshKey?: number } = $props();

  let rules = $state<Rule[]>([]);
  let failed = $state(false);
  let adding = $state(false);
  let invalidDraft = $state(false);
  let draft = $state<Rule>(emptyPermissionRuleDraft());

  async function load(id: string | null) {
    if (!id) {
      rules = [];
      return;
    }
    try {
      rules = (await invoke<Rule[]>("llm_permission_rules_get", { agentId: id })) ?? [];
      failed = false;
    } catch {
      failed = true;
    }
  }

  $effect(() => {
    void refreshKey;
    void load(agentId);
  });

  async function save(next: Rule[]) {
    if (!agentId) return;
    rules = next;
    try {
      await invoke("llm_permission_rules_set", { agentId, rules: next });
      failed = false;
    } catch {
      failed = true;
      await load(agentId);
    }
  }

  function remove(index: number) {
    void save(rules.filter((_, i) => i !== index));
  }

  function patch(index: number, change: Partial<Rule>) {
    const next = rules.map((r, i) => (i === index ? { ...r, ...change } : r));
    if (!next[index].pattern.trim()) return;
    void save(next);
  }

  function add() {
    const rule = normalizePermissionRule(draft);
    if (!rule) {
      invalidDraft = true;
      return;
    }
    invalidDraft = false;
    void save([...rules, rule]);
    draft = emptyPermissionRuleDraft();
    adding = false;
  }
</script>

<section class="block" data-testid="permission-rules">
  <h3>{$t("llm.rules.title")}</h3>
  {#if !agentId}
    <p class="dim">{$t("llm.rules.no_agent")}</p>
  {:else}
    {#if rules.length === 0}
      <p class="dim">{$t("llm.rules.empty")}</p>
    {:else}
      <ul class="rule-list">
        {#each rules as rule, index (index + rule.tool + rule.pattern)}
          <li>
            <span class="mono tool">{rule.tool}</span>
            <input
              class="input mono"
              value={rule.pattern}
              aria-label={$t("llm.rules.pattern")}
              onchange={(e) => patch(index, { pattern: e.currentTarget.value })}
            />
            <select
              class="input"
              value={rule.action}
              aria-label={$t("llm.rules.action")}
              onchange={(e) => patch(index, { action: e.currentTarget.value as Action })}
            >
              <option value="allow">{$t("llm.rules.allow")}</option>
              <option value="ask">{$t("llm.rules.ask")}</option>
              <option value="deny">{$t("llm.rules.deny")}</option>
            </select>
            <button
              type="button"
              class="button remove"
              aria-label={$t("llm.rules.remove")}
              title={$t("llm.rules.remove")}
              onclick={() => remove(index)}
            >
              ×
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    {#if adding}
      <div class="add-form">
        <input class="input mono" bind:value={draft.tool} aria-label={$t("llm.rules.tool")} aria-invalid={invalidDraft && !draft.tool.trim()} oninput={() => invalidDraft = false} placeholder="shell_exec" />
        <input class="input mono" bind:value={draft.pattern} aria-label={$t("llm.rules.pattern")} aria-invalid={invalidDraft && !draft.pattern.trim()} aria-describedby={invalidDraft ? "llm-rule-scope-error" : undefined} oninput={() => invalidDraft = false} placeholder="git status *" />
        <select class="input" bind:value={draft.action} aria-label={$t("llm.rules.action")}>
          <option value="allow">{$t("llm.rules.allow")}</option>
          <option value="ask">{$t("llm.rules.ask")}</option>
          <option value="deny">{$t("llm.rules.deny")}</option>
        </select>
        <div class="add-actions">
          <button type="button" class="button primary" onclick={add}>{$t("llm.rules.save")}</button>
          <button type="button" class="button" onclick={() => (adding = false)}>{$t("llm.rules.cancel")}</button>
        </div>
        {#if invalidDraft}<p id="llm-rule-scope-error" class="rule-error" role="alert">{$t("llm.rules.scope_required")}</p>{/if}
      </div>
    {:else}
      <button type="button" class="button add" onclick={() => (adding = true)}>{$t("llm.rules.add")}</button>
    {/if}
    <p class="dim hint">{$t("llm.rules.hint")}</p>
    {#if failed}<p class="error">{$t("llm.rules.failed")}</p>{/if}
  {/if}
</section>

<style>
  h3 {
    margin: 0 0 var(--space-2);
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
  }

  p {
    margin: 0;
    font-size: var(--text-sm);
  }
  .rule-error { color: var(--text); margin: 0; font-size: var(--text-sm); }

  .dim {
    color: var(--text-dim);
  }

  .hint {
    margin-top: var(--space-2);
    font-size: var(--text-caption);
  }

  .error {
    color: var(--danger, #ff453a);
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
  }

  .rule-list {
    list-style: none;
    margin: 0 0 var(--space-2);
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .rule-list li {
    display: grid;
    grid-template-columns: 1fr auto auto;
    grid-template-areas: "tool tool remove" "pattern action action";
    gap: var(--space-1) var(--space-2);
    align-items: center;
  }

  .rule-list .tool {
    grid-area: tool;
    overflow-wrap: anywhere;
  }

  .rule-list input {
    grid-area: pattern;
    min-width: 0;
  }

  .rule-list select {
    grid-area: action;
  }

  .remove {
    grid-area: remove;
    justify-self: end;
    padding: 0 var(--space-2);
    line-height: 1.4;
  }

  .add-form {
    display: grid;
    gap: var(--space-2);
  }

  .add-actions {
    display: flex;
    gap: var(--space-2);
  }
</style>
