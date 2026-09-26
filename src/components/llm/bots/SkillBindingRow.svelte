<script lang="ts">
  /** One skill bound to a bot: its real state this turn, what it depends on, and the fixes. */
  import { t } from "$lib/i18n";
  import {
    bindingStateKey, bindingTone, fixPlan, reasonKey, versionLabel, type BindingStatus, type Fix,
  } from "$lib/stores/assist-bots-store.svelte";

  let { status, busy = false, onfix, ongrants, onunbind }: {
    status: BindingStatus;
    busy?: boolean;
    onfix: (fix: Fix) => void;
    ongrants: (allowRead: boolean, allowScripts: boolean) => void;
    onunbind: () => void;
  } = $props();

  let tone = $derived(bindingTone(status.state));
  let fixes = $derived(status.fixes.filter((f, i, all) => all.findIndex((g) => g.action === f.action && g.capability === f.capability && g.target === f.target) === i));
</script>

<li class="skill">
  <div class="head">
    <strong class="name">{status.skill}</strong>
    <span class="badge {tone}">{$t(bindingStateKey(status.state))}</span>
    <span class="version">{versionLabel(status)}</span>
  </div>
  {#if status.description}<p class="desc">{status.description}</p>{/if}

  {#if status.deps.length}
    <ul class="deps" aria-label={$t("assist.bots.skills.deps")}>
      {#each status.deps as dep (dep.raw)}
        <li class:missing={dep.satisfied === false}>
          <span class="mono">{dep.raw}</span>
          <span>{dep.satisfied === true ? $t("assist.bots.skills.dep_ok") : dep.satisfied === false ? $t("assist.bots.skills.dep_missing") : $t("assist.bots.skills.dep_unchecked")}</span>
          {#if dep.satisfied === false && reasonKey(dep.reason)}<span class="reason">{$t(reasonKey(dep.reason)!)}</span>{/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if status.compatibility}<p class="desc">{$t("assist.bots.skills.compatibility")}: {status.compatibility}</p>{/if}

  <div class="grants">
    <label><input type="checkbox" checked={status.allow_read} disabled={busy} onchange={(e) => ongrants(e.currentTarget.checked, status.allow_scripts)} /> {$t("assist.bots.skills.allow_read")}</label>
    <label><input type="checkbox" checked={status.allow_scripts} disabled={busy} onchange={(e) => ongrants(status.allow_read, e.currentTarget.checked)} /> {$t("assist.bots.skills.allow_scripts")}</label>
  </div>

  <div class="actions">
    {#each fixes as fix (fix.action + (fix.capability ?? "") + (fix.target ?? ""))}
      {#if fix.action !== "unbind"}
        <button type="button" class="button small" disabled={busy} onclick={() => onfix(fix)}>{$t(fixPlan(fix).labelKey)}</button>
      {/if}
    {/each}
    <button type="button" class="button small" disabled={busy} onclick={onunbind}>{$t("assist.bots.fix.unbind")}</button>
  </div>
</li>

<style>
  .skill { border: 1px solid var(--separator); border-radius: var(--radius-sm); padding: 12px; display: flex; flex-direction: column; gap: 8px; list-style: none; }
  .head { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .name { font-family: var(--font-mono); font-size: 13px; }
  .version { color: var(--text-muted); font-size: 12px; font-family: var(--font-mono); }
  .badge { font-size: 12px; padding: 2px 8px; border-radius: 999px; background: var(--fill-tertiary); }
  .badge.good { color: var(--success, #067647); }
  .badge.warn { color: var(--warning, #b54708); }
  .badge.bad { color: var(--error, #b42318); }
  .desc { margin: 0; color: var(--text-muted); font-size: 13px; }
  .deps { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 4px; font-size: 13px; }
  .deps li { display: flex; gap: 8px; flex-wrap: wrap; }
  .deps li.missing { color: var(--warning, #b54708); }
  .reason { color: var(--text-muted); }
  .mono { font-family: var(--font-mono); }
  .grants { display: flex; gap: 16px; flex-wrap: wrap; font-size: 13px; }
  .actions { display: flex; gap: 6px; flex-wrap: wrap; }
  .button:focus-visible, input:focus-visible { outline: var(--focus-ring, 2px solid var(--accent-text)); outline-offset: 2px; }
</style>
