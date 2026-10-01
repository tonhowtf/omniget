<script lang="ts">
  import { t } from "$lib/i18n";
  import { formatDay, shortenPath, tildify } from "$lib/washmybelly/format";
  import { factTags, revealPath, stateKey, type PersistenceItem } from "$lib/washmybelly/security";
  import Icon from "./ui/Icon.svelte";
  import SecuritySeal from "./SecuritySeal.svelte";

  let {
    item,
    onReveal,
    onManage,
  }: {
    item: PersistenceItem;
    onReveal: (path: string) => void;
    onManage?: (it: PersistenceItem) => void;
  } = $props();

  let open = $state(false);
  let reveal = $derived(revealPath(item));
  let tags = $derived(factTags(item));
  let stateText = $derived(stateKey(item.state) ? ($t(stateKey(item.state)!) as string) : item.state);
  let where = $derived(item.checked ?? item.path ?? item.program ?? "");
</script>

<div class="group-row sec-row" role="listitem">
  <div class="group-row-content">
    <div class="sec-title">
      <span class="group-row-title sec-name" title={item.name}>{item.name}</span>
      <SecuritySeal cls={item.class} title={item.summary} />
      {#if item.xprotect_matches.length}
        <span class="sec-tag sec-tag-look">{$t("washmybelly.security.xprotect_match")}</span>
      {/if}
    </div>
    <span class="group-row-sub sec-sub">
      {#if item.signer || item.team_id}
        <span>{item.signer ?? ""}{#if item.team_id}{item.signer ? " · " : ""}<span class="mono">{item.team_id}</span>{/if}</span>
      {/if}
      {#if stateText}<span>{stateText}</span>{/if}
      {#if item.installed}<span>{$t("washmybelly.security.since", { date: formatDay(item.installed) })}</span>{/if}
    </span>
    {#if where}
      <span class="sec-path mono" title={where}>{shortenPath(tildify(where), 70)}</span>
    {/if}
    {#if tags.length}
      <span class="sec-tags">
        {#each tags as n (n)}
          <span class="sec-tag">{$t(`washmybelly.security.note.${n}`)}</span>
        {/each}
      </span>
    {/if}
    {#if open}
      <div class="sec-more">
        <p>{item.summary}</p>
        {#if item.command}<p class="mono">{item.command}</p>{/if}
        {#if item.detail}<p>{item.detail}</p>{/if}
        {#if item.quarantine}
          <p>
            {$t("washmybelly.security.quarantine_from", { name: item.quarantine.agent ?? "?" })}
            {#if item.quarantine.at}· {formatDay(item.quarantine.at)}{/if}
          </p>
        {/if}
        {#if item.signed_at}<p>{$t("washmybelly.security.signed_at", { date: item.signed_at })}</p>{/if}
        {#if item.xprotect_matches.length}
          <p>{$t("washmybelly.security.xprotect_rules", { name: item.xprotect_matches.join(", ") })}</p>
        {/if}
      </div>
    {/if}
  </div>
  <div class="group-row-trailing sec-actions">
    {#if reveal}
      <button class="btn btn-secondary btn-sm" type="button" onclick={() => onReveal(reveal!)}>{$t("washmybelly.security.reveal")}</button>
    {/if}
    {#if item.startup_id && onManage}
      <button class="btn btn-secondary btn-sm" type="button" onclick={() => onManage(item)}>{$t("washmybelly.security.manage")}</button>
    {/if}
    <button
      class="sec-disclose"
      class:open
      type="button"
      aria-expanded={open}
      aria-label={$t("washmybelly.security.details") as string}
      onclick={() => (open = !open)}
    >
      <Icon name="chevron" size={12} />
    </button>
  </div>
</div>

<style>
  .sec-row {
    align-items: flex-start;
  }
  .sec-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .sec-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .sec-sub {
    display: flex;
    flex-wrap: wrap;
    gap: 2px var(--space-2);
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }
  .sec-path {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-dim);
  }
  .sec-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
  }
  .sec-tag {
    padding: 1px 6px;
    border-radius: var(--radius-full);
    font-size: var(--text-caption);
    font-weight: 600;
    background: var(--fill-2);
    color: var(--text-muted);
  }
  .sec-tag-look {
    background: color-mix(in srgb, var(--warning, #ff9f0a) 17%, transparent);
    color: var(--warning, #ff9f0a);
  }
  .sec-more {
    margin-top: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .sec-more p {
    margin: 0 0 4px;
    word-break: break-word;
  }
  .sec-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: 2px;
  }
  .sec-disclose {
    display: inline-grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    transition: transform 0.15s ease;
  }
  .sec-disclose.open {
    transform: rotate(90deg);
  }
  @media (prefers-reduced-motion: reduce) {
    .sec-disclose {
      transition: none;
    }
  }
</style>
