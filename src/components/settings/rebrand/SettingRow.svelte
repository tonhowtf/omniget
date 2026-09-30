<script lang="ts">
  import type { Snippet } from "svelte";
  import SettingStatus from "./SettingStatus.svelte";

  let {
    id,
    label,
    help,
    settingKey,
    status,
    disabledReason,
    control,
  }: {
    id: string;
    label: string;
    help?: string;
    settingKey?: string;
    status?: string;
    disabledReason?: string;
    control: Snippet<[{ id: string; describedBy: string | undefined }]>;
  } = $props();

  let helpId = $derived(`${id}-help`);
  let reasonId = $derived(`${id}-reason`);
  let describedBy = $derived([help ? helpId : null, disabledReason ? reasonId : null].filter(Boolean).join(" ") || undefined);
</script>

<div class="setting-row" data-setting-key={settingKey} class:is-disabled={!!disabledReason}>
  <div class="setting-row-text">
    <label class="setting-row-label" id={`${id}-label`} for={id}>{label}</label>
    {#if help}<p class="setting-row-help" id={helpId}><bdi>{help}</bdi></p>{/if}
    {#if disabledReason}<p class="setting-row-reason" id={reasonId}><bdi>{disabledReason}</bdi></p>{/if}
    {#if status}<SettingStatus path={status} />{/if}
  </div>
  <div class="setting-row-control">
    {@render control({ id, describedBy })}
  </div>
</div>

<style>
  .setting-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-block-size: 40px;
    padding-block: var(--space-2);
    min-inline-size: 0;
  }
  .setting-row + :global(.setting-row) { border-block-start: 1px solid var(--separator); }
  .setting-row-text { flex: 1 1 auto; min-inline-size: 0; max-inline-size: 76ch; }
  .setting-row-label { display: block; font-size: 13px; line-height: 20px; color: var(--text); overflow-wrap: anywhere; }
  .setting-row-help, .setting-row-reason { margin: 2px 0 0; font-size: 12px; line-height: 18px; color: var(--text-muted); overflow-wrap: anywhere; }
  .setting-row-reason { color: var(--text); }
  .setting-row-control {
    flex: 0 1 clamp(160px, 40%, 260px);
    display: flex;
    justify-content: flex-end;
    align-items: center;
    /* Never narrower than 160 px beside the label; stacked rows use the full width. */
    min-inline-size: min(160px, 100%);
  }
  .setting-row-control :global(:is(input:not([type="checkbox"]):not([type="radio"]):not([type="range"]), select, textarea)) {
    inline-size: 100%;
    min-inline-size: 0;
    box-sizing: border-box;
  }
  @container (max-width: 620px) {
    .setting-row { flex-wrap: wrap; align-items: stretch; }
    .setting-row-text { flex-basis: 100%; }
    .setting-row-control { flex: 1 1 100%; justify-content: flex-start; }
    /* A lone switch stays beside its label while the label fits. */
    .setting-row:has(> .setting-row-control > :global([role="switch"]:only-child)) { flex-wrap: nowrap; align-items: center; }
    .setting-row:has(> .setting-row-control > :global([role="switch"]:only-child)) .setting-row-text { flex: 1 1 auto; }
    .setting-row:has(> .setting-row-control > :global([role="switch"]:only-child)) .setting-row-control { flex: none; justify-content: flex-end; }
  }
</style>
