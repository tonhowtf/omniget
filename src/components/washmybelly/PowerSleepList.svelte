<script lang="ts">
  import { t } from "$lib/i18n";
  import { groupBlockers, type SleepBlocker } from "$lib/washmybelly/power";
  import Icon from "./ui/Icon.svelte";

  /// What keeps the machine (or the display) awake right now.
  let { blockers, needsAdmin }: { blockers: SleepBlocker[]; needsAdmin: boolean } = $props();
  let rows = $derived(groupBlockers(blockers));
</script>

<section class="card">
  <h3>{$t("washmybelly.power.sleep.title")}</h3>
  {#if needsAdmin}
    <p class="dim"><Icon name="lock" size={12} /> {$t("washmybelly.power.sleep.needs_admin")}</p>
  {:else if !rows.length}
    <p class="dim">{$t("washmybelly.power.sleep.none")}</p>
  {:else}
    <ul>
      {#each rows as r (r.process)}
        <li>
          <div class="name">{r.process}{#if r.count > 1}<span class="dim"> ×{r.count}</span>{/if}</div>
          <div class="why dim">{r.reasons.join(" · ") || $t("washmybelly.power.sleep.no_reason")}</div>
          <span class="tag">{r.display ? $t("washmybelly.power.sleep.display") : $t("washmybelly.power.sleep.system")}</span>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .card { border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator)); padding: 12px 16px; }
  h3 { margin: 0 0 6px; font-size: 13px; font-weight: 600; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { display: grid; grid-template-columns: minmax(0, 1fr) auto; grid-template-rows: auto auto; column-gap: 12px; padding: 7px 0; }
  li + li { border-top: var(--hairline, 1px) solid var(--separator); }
  .name { font-size: 13px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .why { grid-column: 1; font-size: 11.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tag { grid-column: 2; grid-row: 1 / span 2; align-self: center; font-size: 11px; padding: 2px 8px; border-radius: 999px; background: var(--surface-mut, var(--separator)); color: var(--text-dim); }
  .dim { margin: 0; font-size: 12px; color: var(--text-dim); }
</style>
