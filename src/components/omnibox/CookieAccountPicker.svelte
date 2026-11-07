<script lang="ts">
  import { t } from "$lib/i18n";

  type Account = {
    slug: string;
    alias: string;
    captured_at_ms: number;
    cookie_count: number;
    last_used_at_ms: number | null;
  };

  let {
    accounts = [] as Account[],
    selectedSlug = $bindable<string | null>(null),
  } = $props();

  let open = $state(false);

  let selectedLabel = $derived.by(() => {
    if (!accounts.length) return null;
    const slug = selectedSlug ?? accounts[0]?.slug;
    const acc = accounts.find(a => a.slug === slug);
    return acc?.alias ?? slug;
  });

  function pick(slug: string) {
    selectedSlug = slug;
    open = false;
  }
</script>

{#if accounts.length > 1}
  <div class="cookie-picker">
    <button
      type="button"
      class="cookie-picker-trigger"
      onclick={() => { open = !open; }}
      aria-haspopup="listbox"
      aria-expanded={open}
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <circle cx="12" cy="12" r="10" />
        <circle cx="9" cy="10" r="1.5" />
        <circle cx="15" cy="10" r="1.5" />
        <circle cx="12" cy="15" r="1.5" />
      </svg>
      <span class="cookie-picker-label">{$t('omnibox.cookie_account')}:</span>
      <span class="cookie-picker-value">{selectedLabel}</span>
      <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <polyline points="6 9 12 15 18 9" />
      </svg>
    </button>

    {#if open}
      <ul class="cookie-picker-menu" role="listbox">
        {#each accounts as acc (acc.slug)}
          <li>
            <button
              type="button"
              class="cookie-picker-option"
              class:active={(selectedSlug ?? accounts[0]?.slug) === acc.slug}
              role="option"
              aria-selected={(selectedSlug ?? accounts[0]?.slug) === acc.slug}
              onclick={() => pick(acc.slug)}
            >
              <span class="cookie-picker-option-alias">{acc.alias}</span>
              <span class="cookie-picker-option-meta">
                {acc.cookie_count} {$t('omnibox.cookie_account_cookies')}
              </span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .cookie-picker {
    position: relative;
    display: inline-flex;
  }

  .cookie-picker-trigger {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--gray);
    background: var(--button);
    border: none;
    border-radius: calc(var(--border-radius) - 2px);
    cursor: pointer;
    box-shadow: var(--button-box-shadow);
  }

  .cookie-picker-trigger:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  @media (hover: hover) {
    .cookie-picker-trigger:hover {
      background: var(--button-hover);
      color: var(--secondary);
    }
  }

  .cookie-picker-label {
    color: var(--gray);
  }

  .cookie-picker-value {
    color: var(--secondary);
    font-weight: 600;
  }

  .cookie-picker-menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 30;
    min-width: 220px;
    margin: 0;
    padding: 4px;
    list-style: none;
    background: var(--popup-bg);
    border-radius: var(--border-radius);
    box-shadow: var(--elev-2);
  }

  .cookie-picker-option {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    width: 100%;
    padding: 8px 10px;
    font-size: var(--text-sm);
    color: var(--secondary);
    background: transparent;
    border: none;
    border-radius: calc(var(--border-radius) - 4px);
    text-align: left;
    cursor: pointer;
  }

  @media (hover: hover) {
    .cookie-picker-option:hover {
      background: var(--button-hover);
    }
  }

  .cookie-picker-option.active {
    background: var(--button-elevated);
  }

  .cookie-picker-option-alias {
    color: var(--secondary);
    font-weight: 500;
  }

  .cookie-picker-option-meta {
    color: var(--gray);
    font-size: 11px;
  }
</style>
