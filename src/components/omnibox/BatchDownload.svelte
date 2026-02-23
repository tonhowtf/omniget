<script lang="ts">
  import { t } from "$lib/i18n";

  let { count, onDownload } = $props();
</script>

<div class="feedback feedback-enter" data-supported="true">
  <span class="feedback-text">
    {$t('omnibox.batch_detected', { count })}
  </span>
</div>

<button class="button action-btn" onclick={onDownload}>
  {$t('omnibox.batch_download_all')}
</button>

<style>
  .feedback {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
  }

  .feedback[data-supported="true"] {
    color: var(--green);
  }

  .feedback-text {
    font-size: 12.5px;
    font-weight: 500;
  }

  .action-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: var(--padding) calc(var(--padding) * 2);
    font-size: 14.5px;
    font-weight: 500;
    background: var(--button);
    border: none;
    border-radius: var(--border-radius);
    color: var(--button-text);
    cursor: pointer;
    box-shadow: var(--button-box-shadow);
  }

  @media (hover: hover) {
    .action-btn:hover {
      background: var(--button-hover);
    }
  }

  .action-btn:active {
    background: var(--button-press);
  }

  .action-btn:disabled {
    cursor: default;
  }
</style>
