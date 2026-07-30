<script lang="ts">
  import { t } from "$lib/i18n";
  import { GOAL_FIELDS, ROLES, type GoalKey, type Role } from "./shared";

  let {
    goalValue,
    setGoal,
    resetGoals,
    active,
  }: {
    goalValue: (role: Role, key: GoalKey) => number;
    setGoal: (role: Role, key: GoalKey, value: number) => void;
    resetGoals: (role: Role) => void;
    active?: boolean;
  } = $props();

  let goalRole = $state<Role>("MIDDLE");
</script>

{#if active !== false}
  <section class="card">
    <div class="card-head">
      <h3>{$t("league.goals_title")}</h3>
      <select class="select-role" bind:value={goalRole} aria-label={$t("league.goals_role") as string}>
        {#each ROLES as r (r)}
          <option value={r}>{$t(`league.role_${r.toLowerCase()}`)}</option>
        {/each}
      </select>
    </div>
    <p class="win-disclaimer">{$t("league.goals_desc")}</p>
    <div class="goal-config">
      {#each GOAL_FIELDS as field (field.key)}
        <label class="goal-field">
          <span class="goal-field-label">{$t(field.labelKey)}</span>
          <input
            type="number"
            class="input-text"
            min="0"
            step={field.step}
            value={goalValue(goalRole, field.key)}
            onchange={(e) => setGoal(goalRole, field.key, Number(e.currentTarget.value))}
          />
        </label>
      {/each}
    </div>
    <button class="button" onclick={() => resetGoals(goalRole)}>{$t("league.goals_reset")}</button>
  </section>
{/if}
