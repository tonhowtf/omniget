<script lang="ts">
  import { t } from "$lib/i18n";
  import SettingsIcon from "./SettingsIcon.svelte";

  let { href, titleKey, hintKey }: { href: string; titleKey: string; hintKey?: string } = $props();
</script>

<a class="manager-link" {href}>
  <span class="manager-text">
    <span class="manager-title">{$t(titleKey)}</span>
    {#if hintKey}<span class="manager-hint">{$t(hintKey)}</span>{/if}
  </span>
  <SettingsIcon name="chevron" size={18} />
</a>

<style>
  .manager-link {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-block-size: 40px;
    padding-block: var(--space-2);
    color: var(--text);
    text-decoration: none;
    border-radius: 6px;
  }
  .manager-link:hover .manager-title { color: var(--accent-text, var(--accent)); }
  .manager-link:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .manager-text { display: flex; flex-direction: column; min-inline-size: 0; }
  .manager-title { font-size: 13px; line-height: 20px; }
  .manager-hint { font-size: 12px; line-height: 18px; color: var(--text-muted); overflow-wrap: anywhere; }
</style>
