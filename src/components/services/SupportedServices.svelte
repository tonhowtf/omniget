<script lang="ts">
  import { t } from "$lib/i18n";

  let expanded = $state(false);

  const services = [
    "YouTube", "Instagram", "TikTok", "Twitter / X",
    "Reddit", "Twitch", "Pinterest", "Vimeo",
    "Bluesky", "Hotmart", "Telegram",
    "SoundCloud", "Facebook", "Dailymotion", "Bilibili",
    "Snapchat", "Tumblr", "Rutube", "VK",
    "Streamable", "Loom", "Newgrounds", "Xiaohongshu",
  ];

  function toggle() {
    expanded = !expanded;
  }
</script>

<div class="supported-services">
  <button
    class="toggle-btn"
    onclick={toggle}
    aria-expanded={expanded}
    aria-label={expanded ? $t('services.title_hide') : $t('services.title_show')}
  >
    <span class="icon-circle" class:expanded>
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M12 5v14M5 12h14" />
      </svg>
    </span>
    <span class="toggle-label">{$t('services.title')}</span>
  </button>

  {#if expanded}
    <div class="popover">
      <div class="pills">
        {#each services as service}
          <span class="pill">{service}</span>
        {/each}
      </div>
      <p class="disclaimer">{$t('services.disclaimer')}</p>
    </div>
  {/if}
</div>

<style>
  .supported-services {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--padding) / 2);
    width: 100%;
    max-width: 560px;
  }

  .toggle-btn {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 7px 13px 7px 10px;
    border-radius: 18px;
    background: none;
    border: none;
    color: var(--gray);
    cursor: pointer;
    font-size: 12.5px;
    font-weight: 500;
    user-select: none;
  }

  @media (hover: hover) {
    .toggle-btn:hover {
      color: var(--secondary);
    }

    .toggle-btn:hover .icon-circle {
      background: var(--button-elevated-hover);
    }
  }

  .toggle-btn:active .icon-circle {
    background: var(--button-elevated-press);
  }

  .toggle-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .icon-circle {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--button-elevated);
    border-radius: 18px;
    flex-shrink: 0;
    transition: transform 0.2s cubic-bezier(0.53, 0.05, 0.02, 1.2);
  }

  .icon-circle.expanded {
    transform: rotate(45deg);
  }

  .icon-circle svg {
    pointer-events: none;
  }

  .toggle-label {
    pointer-events: none;
  }

  .popover {
    width: 100%;
    background: var(--button);
    box-shadow: var(--button-box-shadow);
    border-radius: var(--border-radius);
    padding: var(--padding);
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    animation: popoverEnter 150ms ease-out;
  }

  @keyframes popoverEnter {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .pills {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .pill {
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) / 2);
    padding: 4px 8px;
    font-size: 12.5px;
    font-weight: 500;
    color: var(--secondary);
    user-select: none;
  }

  .disclaimer {
    font-size: 11px;
    font-weight: 400;
    color: var(--gray);
    opacity: 0.7;
  }

  @media (prefers-reduced-motion: reduce) {
    .popover {
      animation: none;
    }

    .icon-circle {
      transition: none;
    }
  }
</style>
