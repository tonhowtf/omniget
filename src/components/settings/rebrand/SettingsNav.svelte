<script lang="ts">
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { SETTINGS_CATEGORIES, CATEGORY_LABEL_KEYS, settingsHref, type SettingsCategory } from "$lib/settings-navigation";
  import SettingsIcon from "./SettingsIcon.svelte";

  let {
    active,
    searching = false,
    onNavigate,
  }: {
    active: SettingsCategory;
    searching?: boolean;
    onNavigate?: () => void;
  } = $props();

  function onSelect(e: Event) {
    const tab = (e.target as HTMLSelectElement).value as SettingsCategory;
    onNavigate?.();
    void goto(settingsHref(tab), { noScroll: false, keepFocus: true });
  }
</script>

<nav class="settings-nav" aria-label={$t("settings.rebrand.title")}>
  <ul>
    {#each SETTINGS_CATEGORIES as tab (tab)}
      {@const current = !searching && active === tab}
      <li>
        <a class="settings-nav-link" class:active={current} href={settingsHref(tab)} aria-current={current ? "page" : undefined} onclick={() => onNavigate?.()}>
          <SettingsIcon name={tab} size={18} />
          <span class="settings-nav-label">{$t(CATEGORY_LABEL_KEYS[tab])}</span>
        </a>
      </li>
    {/each}
  </ul>
</nav>
<div class="settings-nav-select">
  <label for="settings-nav-select">{$t("settings.rebrand.title")}</label>
  <select id="settings-nav-select" class="select" value={active} onchange={onSelect}>
    {#each SETTINGS_CATEGORIES as tab (tab)}
      <option value={tab}>{$t(CATEGORY_LABEL_KEYS[tab])}</option>
    {/each}
  </select>
</div>

<style>
  .settings-nav ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .settings-nav-link {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-block-size: 36px;
    padding: var(--space-1) var(--space-2);
    border-radius: 6px;
    color: var(--text);
    text-decoration: none;
    font-size: 13px;
    line-height: 18px;
  }
  .settings-nav-link:hover { background: var(--surface-hi); }
  .settings-nav-link.active { background: var(--accent-soft); color: var(--accent-text, var(--accent)); font-weight: 600; }
  .settings-nav-link:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .settings-nav-label { min-inline-size: 0; overflow-wrap: anywhere; white-space: normal; }
  .settings-nav-select { display: none; }
  .settings-nav-select label {
    position: absolute; inline-size: 1px; block-size: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap;
  }
  .settings-nav-select select { inline-size: 100%; min-block-size: 32px; }
  @media (max-width: 799px) {
    .settings-nav { display: none; }
    .settings-nav-select { display: block; }
  }
</style>
