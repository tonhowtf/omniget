<script lang="ts">
  import { t } from "$lib/i18n";
  import ContextHint from "$components/hints/ContextHint.svelte";

  let { url = $bindable(""), onInput } = $props();
</script>

<div class="omnibox-wrapper">
  <input
    class="omnibox"
    type="text"
    placeholder={$t('omnibox.placeholder')}
    bind:value={url}
    oninput={onInput}
  />
  <ContextHint text={$t('hints.omnibox')} dismissKey="omnibox" />
</div>

<style>
  .omnibox-wrapper {
    width: 100%;
  }

  .omnibox {
    width: 100%;
    padding: var(--padding) calc(var(--padding) + 4px);
    font-size: 14.5px;
    background: var(--button);
    border-radius: var(--border-radius);
    color: var(--secondary);
    border: 1px solid var(--input-border);
  }

  .omnibox::placeholder {
    color: var(--gray);
  }

  .omnibox:focus-visible {
    border-color: var(--secondary);
    outline: none;
  }
</style>
