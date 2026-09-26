<script lang="ts">
  /**
   * Version state of one installed skill: the hash on disk, whether it still
   * matches what OmniGet recorded at install (drift = edited outside
   * OmniGet: bots stop using it until repaired), what it declares it needs,
   * and the two repairs.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getBusySkill, getSkillStatus, loadSkillStatus, repairSkill } from "$lib/stores/llm-skills-store.svelte";

  let { name }: { name: string } = $props();
  let status = $derived(getSkillStatus(name));
  let busy = $derived(getBusySkill() === name);

  onMount(() => { void loadSkillStatus(); });
</script>

{#if status}
  <div class="version" class:drift={status.state === "drift" || status.state === "invalid"}>
    <span class="mono">{status.version ? `${status.version} · ` : ""}{(status.hash ?? "").slice(0, 8)}</span>
    <span>{$t(`assist.bots.skill_version.${status.state}`)}</span>
    {#if status.state === "drift" || status.state === "invalid"}
      <span class="repairs">
        {#if status.state === "drift"}<button type="button" class="button small" disabled={busy} onclick={() => repairSkill(name, "accept")}>{$t("assist.bots.fix.accept_version")}</button>{/if}
        <button type="button" class="button small" disabled={busy} onclick={() => repairSkill(name, "reinstall")}>{$t("assist.bots.fix.reinstall")}</button>
      </span>
    {/if}
    {#if status.dependencies?.length}
      <span class="deps">{$t("assist.bots.skills.needs")}: <span class="mono">{status.dependencies.map((d) => d.raw).join(", ")}</span></span>
    {/if}
  </div>
{/if}

<style>
  .version { display: flex; flex-wrap: wrap; gap: 6px 10px; align-items: center; font-size: 12px; color: var(--text-muted); }
  .version.drift { color: var(--warning, #b54708); }
  .mono { font-family: var(--font-mono); overflow-wrap: anywhere; }
  .repairs { display: flex; gap: 6px; flex-wrap: wrap; }
  .deps { flex-basis: 100%; }
  .button:focus-visible { outline: var(--focus-ring, 2px solid var(--accent-text)); outline-offset: 2px; }
</style>
