<script lang="ts">
  import { t } from "$lib/i18n";
  import { duplicateFinds } from "$lib/dictation/logic";
  import type { Replacement } from "$lib/dictation/types";
  import Icon from "./Icon.svelte";

  let {
    rules,
    disabled = false,
    onchange,
  }: {
    rules: Replacement[];
    disabled?: boolean;
    onchange: (rules: Replacement[]) => void;
  } = $props();

  let dups = $derived(duplicateFinds(rules));

  function update(i: number, patch: Partial<Replacement>) {
    onchange(rules.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  }

  function add() {
    onchange([...rules, { find: "", replace: "" }]);
    queueMicrotask(() => {
      const inputs = document.querySelectorAll<HTMLInputElement>(".dc-rep-find");
      inputs[inputs.length - 1]?.focus();
    });
  }

  function remove(i: number) {
    onchange(rules.filter((_, j) => j !== i));
  }
</script>

<div class="dc-rep">
  {#if rules.length === 0}
    <p class="field-hint">{$t("dictation.rep_empty")}</p>
  {:else}
    <div class="dc-rep-head" aria-hidden="true">
      <span>{$t("dictation.rep_find")}</span>
      <span>{$t("dictation.rep_replace")}</span>
    </div>
    <ul class="dc-rep-list">
      {#each rules as rule, i (i)}
        <li class="dc-rep-row">
          <input
            class="input dc-rep-find"
            class:error={dups.has(i)}
            type="text"
            value={rule.find}
            {disabled}
            placeholder={$t("dictation.rep_find_ph") as string}
            aria-label={$t("dictation.rep_find_label", { n: i + 1 }) as string}
            oninput={(e) => update(i, { find: (e.currentTarget as HTMLInputElement).value })}
          />
          <span class="dc-rep-arrow" aria-hidden="true">→</span>
          <input
            class="input"
            type="text"
            value={rule.replace}
            {disabled}
            placeholder={$t("dictation.rep_replace_ph") as string}
            aria-label={$t("dictation.rep_replace_label", { n: i + 1 }) as string}
            oninput={(e) => update(i, { replace: (e.currentTarget as HTMLInputElement).value })}
          />
          <button type="button" class="btn btn-ghost btn-sm btn-icon" {disabled} onclick={() => remove(i)} aria-label={$t("dictation.rep_remove", { n: i + 1 }) as string}>
            <Icon name="close" size={12} />
          </button>
        </li>
      {/each}
    </ul>
    {#if dups.size > 0}
      <p class="field-error">{$t("dictation.rep_duplicate")}</p>
    {/if}
  {/if}
  <button type="button" class="btn btn-secondary btn-sm dc-rep-add" {disabled} onclick={add}>
    <Icon name="plus" size={12} />
    {$t("dictation.rep_add")}
  </button>
</div>

<style>
  .dc-rep {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .dc-rep-head,
  .dc-rep-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 14px minmax(0, 1fr) 24px;
    align-items: center;
    gap: var(--space-1);
  }

  .dc-rep-head {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .dc-rep-head span:nth-child(2) {
    grid-column: 3;
  }

  .dc-rep-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .dc-rep-row .input {
    width: 100%;
    min-width: 0;
  }

  .dc-rep-arrow {
    text-align: center;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .dc-rep-add {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    align-self: flex-start;
  }
</style>
