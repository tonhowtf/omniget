<script lang="ts">
  import { t } from "$lib/i18n";
  import { isActionable, reasonKey, type PrivacyItem } from "$lib/washmybelly/privacy";
  import Icon from "./ui/Icon.svelte";
  import RiskBadge from "./ui/RiskBadge.svelte";
  import PrivacyCode from "./PrivacyCode.svelte";

  let {
    item,
    selected = false,
    onSelect,
    onApply,
  }: {
    item: PrivacyItem;
    selected?: boolean;
    onSelect: (it: PrivacyItem, on: boolean) => void;
    onApply: (it: PrivacyItem) => void;
  } = $props();

  let open = $state(false);
  let actionable = $derived(isActionable(item));
  let reason = $derived(item.state === "unknown" ? (reasonKey(item.reason) ? ($t(reasonKey(item.reason)!) as string) : item.reason || ($t("washmybelly.privacy.state_unknown_hint") as string)) : null);
  let hasMore = $derived(!!item.docs || (item.code_only && !!item.code));
</script>

<div class="group-row pr" class:unknown={item.state === "unknown"} role="listitem">
  <input
    class="pr-check"
    type="checkbox"
    checked={selected}
    disabled={!actionable}
    aria-label={$t("washmybelly.privacy.select", { name: item.name }) as string}
    title={reason ?? undefined}
    onchange={(e) => onSelect(item, (e.currentTarget as HTMLInputElement).checked)}
  />
  <div class="group-row-content">
    <div class="pr-title">
      {#if hasMore}
        <button type="button" class="pr-name pr-toggle" aria-expanded={open} onclick={() => (open = !open)}>
          <span class="pr-chev" class:open><Icon name="chevron" size={10} /></span>{item.name}
        </button>
      {:else}
        <span class="pr-name">{item.name}</span>
      {/if}
    </div>
    <span class="group-row-sub pr-sub">
      {#if item.recommend === "standard"}<span class="tag tag-accent">{$t("washmybelly.privacy.rec_standard")}</span>{/if}
      {#if item.recommend === "strict"}<span class="tag tag-info">{$t("washmybelly.privacy.rec_strict")}</span>{/if}
      {#if item.risk === "extreme" || item.risk === "destructive"}<RiskBadge risk={item.risk} compact />{/if}
      {#if item.admin}<span class="tag" title={$t("washmybelly.privacy.needs_admin") as string}><Icon name="lock" size={10} />{$t("washmybelly.privacy.admin")}</span>{/if}
      {#if item.restart}<span class="tag">{$t("washmybelly.privacy.restart")}</span>{/if}
      {#if item.code_only}<span class="tag">{$t("washmybelly.privacy.code_only")}</span>{/if}
    </span>
    {#if reason}
      <span class="pr-reason"><Icon name="info" size={11} />{reason}</span>
    {/if}
    {#if open}
      <div class="pr-more">
        {#if item.docs}<p class="pr-docs">{item.docs}</p>{/if}
        {#if item.code_only && item.code}
          <p class="pr-docs dim">{$t("washmybelly.privacy.code_only_hint")}</p>
          <PrivacyCode code={item.code} />
        {/if}
      </div>
    {/if}
  </div>
  <div class="group-row-trailing pr-trail">
    {#if item.state === "applied"}
      <span class="tag tag-success"><Icon name="check" size={10} />{$t("washmybelly.privacy.state.applied")}</span>
    {:else if item.state === "unknown"}
      <span class="tag">{$t("washmybelly.privacy.state.unknown")}</span>
    {:else}
      <span class="tag tag-warning">{$t("washmybelly.privacy.state.not_applied")}</span>
      {#if actionable}
        <button type="button" class="btn btn-sm btn-secondary" onclick={() => onApply(item)}>{$t("washmybelly.privacy.apply_one")}</button>
      {/if}
    {/if}
  </div>
</div>

<style>
  .pr {
    align-items: flex-start;
  }
  .pr.unknown .pr-name {
    color: var(--text-dim);
  }
  .pr-check {
    margin-top: 4px;
    flex-shrink: 0;
  }
  .pr-title {
    display: flex;
    align-items: center;
    min-width: 0;
  }
  .pr-name {
    font-size: var(--text-base);
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .pr-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .pr-chev {
    display: inline-flex;
    color: var(--text-dim);
    transition: transform 0.15s ease;
  }
  .pr-chev.open {
    transform: rotate(90deg);
  }
  .pr-sub {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 3px;
  }
  .pr-sub:empty {
    display: none;
  }
  .pr-reason {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 3px;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .pr-more {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .pr-docs {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    white-space: pre-line;
  }
  .dim {
    color: var(--text-dim);
  }
  .pr-trail {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }
  @media (prefers-reduced-motion: reduce) {
    .pr-chev {
      transition: none;
    }
  }
</style>
