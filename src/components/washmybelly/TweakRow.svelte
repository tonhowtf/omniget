<script lang="ts">
  import { t } from "$lib/i18n";
  import { canPick, canRevert, reasonKey, sourceName, type TweakItem } from "$lib/washmybelly/tweaks";
  import Icon from "./ui/Icon.svelte";
  import RiskBadge from "./ui/RiskBadge.svelte";
  import TweakStateBadge from "./TweakStateBadge.svelte";

  /// One tweak: pick box, name, state read from Windows, tags (restart,
  /// administrator, source), the warning, and the reason when it cannot be
  /// changed here. Ported from "Styled Toggle Switches" (ai2, 21st.dev):
  /// a hairline settings row with the control leading and badges inline;
  /// here the control is a pick box, because nothing changes until the
  /// plan is reviewed.
  let {
    item,
    picked = false,
    busy = false,
    onPick,
    onRevert,
  }: {
    item: TweakItem;
    picked?: boolean;
    busy?: boolean;
    onPick: (it: TweakItem) => void;
    onRevert: (it: TweakItem) => void;
  } = $props();

  let pickable = $derived(canPick(item));
  let reason = $derived(reasonKey(item.reason));
  let label = $derived(item.label_key ? ($t(item.label_key) as string) : item.label);
</script>

<div class="group-row tw-row" class:off={!item.available} role="listitem">
  <input
    class="tw-check"
    type="checkbox"
    checked={picked}
    disabled={!pickable || busy}
    aria-label={$t("washmybelly.tweaks.pick", { name: label }) as string}
    onchange={() => onPick(item)}
  />
  <div class="group-row-content">
    <div class="tw-title">
      <span class="group-row-title tw-name">{label}</span>
      <TweakStateBadge state={item.state} />
      {#if item.risk === "extreme"}<RiskBadge risk="extreme" />{/if}
    </div>
    {#if item.description}<span class="group-row-sub tw-desc">{item.description}</span>{/if}
    <span class="tw-tags">
      {#if item.needs_restart}<span class="tw-tag"><Icon name="refresh" size={10} />{$t("washmybelly.tweaks.restart")}</span>{/if}
      {#if item.admin}<span class="tw-tag"><Icon name="lock" size={10} />{$t("washmybelly.tweaks.admin")}</span>{/if}
      <span class="tw-tag tw-src" title={item.license}>{sourceName(item.source)}</span>
    </span>
    {#if item.warning}
      <span class="tw-warn" class:danger={item.risk === "extreme"}><Icon name="warning" size={11} />{item.warning}</span>
    {/if}
    {#if reason}
      <span class="tw-reason"><Icon name="lock" size={11} />{$t(reason)}</span>
    {/if}
  </div>
  {#if canRevert(item)}
    <div class="group-row-trailing">
      <button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={() => onRevert(item)}>
        <Icon name="undo" size={12} />{$t("washmybelly.tweaks.to_default")}
      </button>
    </div>
  {/if}
</div>

<style>
  .tw-row {
    align-items: flex-start;
  }
  .tw-row.off .tw-name {
    color: var(--text-dim);
  }
  .tw-check {
    margin-top: 3px;
    flex-shrink: 0;
  }
  .tw-title {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .tw-name {
    font-weight: 500;
  }
  .tw-desc {
    display: block;
  }
  .tw-tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin-top: 4px;
  }
  .tw-tag {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 1px 6px;
    border-radius: var(--radius-full);
    font-size: var(--text-caption);
    font-weight: 600;
    background: var(--fill-2);
    color: var(--text-muted);
  }
  .tw-src {
    font-weight: 500;
    color: var(--text-dim);
  }
  .tw-warn,
  .tw-reason {
    display: flex;
    align-items: flex-start;
    gap: 4px;
    margin-top: 4px;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .tw-warn {
    color: var(--warning, #ff9f0a);
  }
  .tw-warn.danger {
    color: var(--danger);
  }
  .tw-reason {
    color: var(--text-dim);
  }
  .tw-warn :global(.wmb-icon),
  .tw-reason :global(.wmb-icon) {
    margin-top: 2px;
    flex-shrink: 0;
  }
</style>
