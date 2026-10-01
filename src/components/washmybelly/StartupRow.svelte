<script lang="ts">
  import { t } from "$lib/i18n";
  import { formatDay, shortenPath, tildify } from "$lib/washmybelly/format";
  import { canToggle, hueOf, initial, isSoftReason, reasonKey, type StartupItem } from "$lib/washmybelly/startup";
  import Icon from "./ui/Icon.svelte";

  let {
    item,
    selected = false,
    busy = false,
    onToggle,
    onSelect,
  }: {
    item: StartupItem;
    selected?: boolean;
    busy?: boolean;
    onToggle: (it: StartupItem) => void;
    onSelect: (it: StartupItem, on: boolean) => void;
  } = $props();

  let toggleable = $derived(canToggle(item));
  let reason = $derived(reasonKey(item.reason));
  let locked = $derived(!toggleable);
  let signer = $derived(item.signer || (item.third_party ? null : ($t("washmybelly.startup.system") as string)));
</script>

<div class="group-row st-row" class:off={!item.enabled} role="listitem">
  <input
    class="st-check"
    type="checkbox"
    checked={selected}
    disabled={!(item.enabled && item.can_disable)}
    aria-label={$t("washmybelly.startup.select", { name: item.name }) as string}
    onchange={(e) => onSelect(item, (e.currentTarget as HTMLInputElement).checked)}
  />
  <span class="st-tile" style:--h={hueOf(item.name)} aria-hidden="true">{initial(item.name)}</span>
  <div class="group-row-content">
    <div class="st-title">
      <span class="group-row-title st-name" title={item.name}>{item.name}</span>
      {#if item.recommended && item.enabled}
        <span class="st-tag st-tag-tip">{$t("washmybelly.startup.suggested")}</span>
      {/if}
      {#if item.impact}
        <span class="st-tag st-impact-{item.impact.level}" title={$t("washmybelly.startup.impact_ms", { count: item.impact.ms }) as string}>
          {$t(`washmybelly.startup.impact.${item.impact.level}`)}
        </span>
      {/if}
      {#if item.running}
        <span class="st-dot" title={$t("washmybelly.startup.running") as string}></span>
      {/if}
    </div>
    <span class="group-row-sub st-sub">
      {#if signer}
        <span class="st-signer" class:verified={item.signer_verified}>
          {#if item.signer_verified}<Icon name="check" size={11} />{/if}{signer}{#if item.team_id}&nbsp;· {item.team_id}{/if}
        </span>
      {:else}
        <span class="st-signer unsigned">{$t("washmybelly.startup.unsigned")}</span>
      {/if}
      <span class="st-loc" title={item.location}>{shortenPath(tildify(item.location), 60)}</span>
      {#if item.installed}
        <span class="st-date">{$t("washmybelly.startup.since", { date: formatDay(item.installed) })}</span>
      {/if}
    </span>
    {#if reason}
      <span class="st-reason" class:soft={isSoftReason(item.reason)}>
        {#if !isSoftReason(item.reason)}<Icon name="lock" size={11} />{:else}<Icon name="info" size={11} />{/if}
        {$t(reason)}
      </span>
    {/if}
  </div>
  <div class="group-row-trailing">
    {#if item.needs_admin && toggleable}
      <span class="st-admin" title={$t("washmybelly.startup.needs_admin") as string}><Icon name="lock" size={12} /></span>
    {/if}
    <button
      type="button"
      class="toggle"
      class:on={item.enabled}
      role="switch"
      aria-checked={item.enabled}
      aria-label={$t(item.enabled ? "washmybelly.startup.turn_off" : "washmybelly.startup.turn_on", { name: item.name }) as string}
      disabled={locked || busy}
      title={locked ? ((reason ? $t(reason) : $t("washmybelly.startup.locked")) as string) : undefined}
      onclick={() => onToggle(item)}
    >
      <span class="toggle-knob"></span>
    </button>
  </div>
</div>

<style>
  .st-row {
    align-items: flex-start;
  }
  .st-row.off .st-name {
    color: var(--text-dim);
  }
  .st-check {
    margin-top: 11px;
    flex-shrink: 0;
  }
  .st-tile {
    flex-shrink: 0;
    width: 30px;
    height: 30px;
    margin-top: 2px;
    display: grid;
    place-items: center;
    border-radius: 8px;
    font-size: 14px;
    font-weight: 700;
    color: #fff;
    background: linear-gradient(180deg, hsl(var(--h) 70% 62%), hsl(var(--h) 62% 46%));
  }
  .st-row.off .st-tile {
    filter: grayscale(0.85);
    opacity: 0.7;
  }
  .st-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .st-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .st-sub {
    display: flex;
    flex-wrap: wrap;
    gap: 2px var(--space-2);
    min-width: 0;
  }
  .st-signer {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .st-signer.verified {
    color: var(--text-muted);
  }
  .st-signer.verified :global(.wmb-icon) {
    color: var(--success, #30d158);
  }
  .st-signer.unsigned {
    color: var(--warning, #ff9f0a);
  }
  .st-loc {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }
  .st-tag {
    flex-shrink: 0;
    padding: 1px 6px;
    border-radius: var(--radius-full);
    font-size: var(--text-caption);
    font-weight: 600;
    background: var(--fill-2);
    color: var(--text-muted);
  }
  .st-tag-tip {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .st-impact-high {
    background: color-mix(in srgb, var(--danger) 16%, transparent);
    color: var(--danger);
  }
  .st-impact-medium {
    background: color-mix(in srgb, var(--warning, #ff9f0a) 18%, transparent);
    color: var(--warning, #ff9f0a);
  }
  .st-dot {
    flex-shrink: 0;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--success, #30d158);
  }
  .st-reason {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 2px;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .st-reason.soft {
    color: var(--text-muted);
  }
  .st-admin {
    color: var(--text-dim);
    display: inline-flex;
  }
  .group-row-trailing {
    margin-top: 6px;
  }
</style>
