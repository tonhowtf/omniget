<script lang="ts">
  import { t } from "$lib/i18n";

  let { downloadMode = $bindable("auto" as "auto" | "audio" | "mute"), onChange = undefined } = $props();
</script>

<div class="mode-group">
  <button
    class="mode-btn"
    class:active={downloadMode === "auto"}
    onclick={() => { downloadMode = "auto"; onChange?.(); }}
  >
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M12 3l1.912 5.813a2 2 0 001.272 1.272L21 12l-5.813 1.912a2 2 0 00-1.272 1.272L12 21l-1.912-5.813a2 2 0 00-1.272-1.272L3 12l5.813-1.912a2 2 0 001.272-1.272z" />
    </svg>
    {$t('omnibox.mode_auto')}
  </button>
  <button
    class="mode-btn"
    class:active={downloadMode === "audio"}
    onclick={() => { downloadMode = "audio"; onChange?.(); }}
  >
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M9 18V5l12-2v13" />
      <circle cx="6" cy="18" r="3" />
      <circle cx="18" cy="16" r="3" />
    </svg>
    {$t('omnibox.mode_audio')}
  </button>
  <button
    class="mode-btn"
    class:active={downloadMode === "mute"}
    onclick={() => { downloadMode = "mute"; onChange?.(); }}
  >
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M11 5L6 9H2v6h4l5 4V5z" />
      <line x1="23" y1="9" x2="17" y2="15" />
      <line x1="17" y1="9" x2="23" y2="15" />
    </svg>
    {$t('omnibox.mode_mute')}
  </button>
</div>

<style>
  .mode-group {
    display: flex;
    background: var(--fill-2);
    border-radius: var(--radius-md);
    padding: 2px;
    gap: 1px;
  }

  .mode-btn {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 26px;
    padding: 0 var(--space-3);
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-muted);
    background: none;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out);
  }

  .mode-btn.active {
    background: var(--surface-hi);
    color: var(--text);
    box-shadow: 0 1px 2px rgba(var(--shadow-ink), var(--elev-alpha-1)), inset 0 0 0 var(--hairline) var(--content-border);
  }

  .mode-btn.active svg {
    color: var(--accent-hi);
  }

  @media (hover: hover) {
    .mode-btn:not(.active):hover {
      color: var(--text);
    }
  }

  .mode-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -1px;
  }

  .mode-btn svg {
    flex-shrink: 0;
    pointer-events: none;
  }
</style>
