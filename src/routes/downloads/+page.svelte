<div class="downloads">
  <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 3v12m0 0l-4-4m4 4l4-4" />
    <path d="M4 17v2a1 1 0 001 1h14a1 1 0 001-1v-2" />
  </svg>
  <p class="empty-text">No downloads in progress</p>
</div>

<style>
  .downloads {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: calc(100vh - var(--padding) * 4);
    gap: calc(var(--padding) * 1.5);
    color: var(--gray);
  }

  .empty-text {
    font-size: 14.5px;
  }
</style>
