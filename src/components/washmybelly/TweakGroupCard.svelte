<script lang="ts">
  import { t } from "$lib/i18n";
  import { canPick, reasonKey, type TweakGroup, type TweakItem } from "$lib/washmybelly/tweaks";
  import Icon from "./ui/Icon.svelte";

  /// card of radio rows. Ported from "Radio Group in Card with Separators"
  let {
    group,
    items,
    picked,
    busy = false,
    onPick,
    onClear,
  }: {
    group: TweakGroup;
    items: TweakItem[];
    picked: Set<string>;
    busy?: boolean;
    onPick: (it: TweakItem) => void;
    onClear: (g: TweakGroup) => void;
  } = $props();

  let chosen = $derived(items.find((i) => picked.has(i.id))?.id ?? group.active ?? null);
  let name = $derived(`tw-group-${group.id}`);
</script>

<div class="tw-group" role="listitem">
  <div class="tw-ghead">
    <span class="group-row-title">{group.label}</span>
    {#if group.description}<span class="group-row-sub">{group.description}</span>{/if}
  </div>
  <div class="tw-options" role="radiogroup" aria-label={group.label}>
    {#each items as it (it.id)}
      {@const current = group.active === it.id}
      {@const reason = reasonKey(it.reason)}
      <label class="tw-opt" class:disabled={!canPick(it) && !current}>
        <input
          type="radio"
          {name}
          checked={chosen === it.id}
          disabled={busy || (!canPick(it) && !current)}
          onchange={() => {
            if (current) onClear(group);
            else onPick(it);
          }}
        />
        <span class="tw-opt-text">
          <span class="tw-opt-label">
            {it.label}
            {#if current}<span class="tw-current">{$t("washmybelly.tweaks.current")}</span>{/if}
            {#if group.default === it.id}<span class="tw-default">{$t("washmybelly.tweaks.windows_default")}</span>{/if}
          </span>
          {#if it.description}<span class="tw-opt-desc">{it.description}</span>{/if}
          {#if it.warning}<span class="tw-opt-warn"><Icon name="warning" size={11} />{it.warning}</span>{/if}
          {#if reason}<span class="tw-opt-desc"><Icon name="lock" size={11} />{$t(reason)}</span>{/if}
        </span>
        {#if it.needs_restart || it.admin}
          <span class="tw-opt-icons">
            {#if it.needs_restart}<span title={$t("washmybelly.tweaks.restart") as string}><Icon name="refresh" size={12} /></span>{/if}
            {#if it.admin}<span title={$t("washmybelly.tweaks.admin") as string}><Icon name="lock" size={12} /></span>{/if}
          </span>
        {/if}
      </label>
    {/each}
  </div>
</div>

<style>
  .tw-group {
    padding: var(--space-3) var(--space-3) var(--space-2);
  }
  .tw-ghead {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-bottom: var(--space-2);
  }
  .tw-options {
    border-radius: var(--radius-md);
    background: var(--fill-1, var(--fill-2));
    overflow: hidden;
  }
  .tw-opt {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    cursor: pointer;
  }
  .tw-opt + .tw-opt {
    border-top: var(--hairline) solid var(--separator, var(--content-border));
  }
  .tw-opt.disabled {
    cursor: default;
    opacity: 0.6;
  }
  .tw-opt input {
    margin-top: 3px;
    flex-shrink: 0;
  }
  .tw-opt-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .tw-opt-label {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    font-weight: 500;
  }
  .tw-opt-desc,
  .tw-opt-warn {
    display: flex;
    gap: 4px;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .tw-opt-warn {
    color: var(--warning, #ff9f0a);
  }
  .tw-current,
  .tw-default {
    padding: 1px 6px;
    border-radius: var(--radius-full);
    font-size: var(--text-caption);
    font-weight: 600;
  }
  .tw-current {
    background: color-mix(in srgb, var(--success, #30d158) 15%, transparent);
    color: var(--success, #30d158);
  }
  .tw-default {
    background: var(--fill-2);
    color: var(--text-muted);
  }
  .tw-opt-icons {
    display: inline-flex;
    gap: 4px;
    color: var(--text-dim);
    margin-top: 2px;
  }
</style>
