<script lang="ts">
  type SearchResult = {
    id: string;
    title: string;
    author: string;
    duration: number | null;
    thumbnail_url: string | null;
    url: string;
    platform: string;
  };

  let { results = [], onSelect } = $props();

  function formatDuration(seconds: number | null): string {
    if (seconds === null) return "";
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = Math.floor(seconds % 60);
    if (h > 0) return `${h}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
    return `${m}:${s.toString().padStart(2, "0")}`;
  }
</script>

<div class="search-results feedback-enter">
  {#each results as result (result.id)}
    <button class="search-result-row" onclick={() => onSelect(result)}>
      {#if result.thumbnail_url}
        <img
          src={result.thumbnail_url}
          alt=""
          class="search-thumb"
          loading="lazy"
        />
      {:else}
        <div class="search-thumb search-thumb-placeholder">
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M23 7l-7 5 7 5V7z" />
            <rect x="1" y="5" width="15" height="14" rx="2" ry="2" />
          </svg>
        </div>
      {/if}
      <div class="search-result-info">
        <span class="search-result-title">{result.title}</span>
        <span class="search-result-meta">
          {#if result.author}{result.author}{/if}
          {#if result.author && result.duration !== null}
            <span class="feedback-sep">&middot;</span>
          {/if}
          {#if result.duration !== null}{formatDuration(result.duration)}{/if}
        </span>
      </div>
    </button>
  {/each}
</div>

<style>
  .search-results {
    width: 100%;
    display: flex;
    flex-direction: column;
    background: var(--button);
    border-radius: var(--border-radius);
    box-shadow: var(--button-box-shadow);
    overflow: hidden;
  }

  .search-result-row {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) * 0.75);
    padding: calc(var(--padding) * 0.75);
    background: transparent;
    border: none;
    border-bottom: 1px solid var(--button-stroke);
    cursor: pointer;
    text-align: left;
  }

  .search-result-row:last-child {
    border-bottom: none;
  }

  @media (hover: hover) {
    .search-result-row:hover {
      background: var(--button-hover);
    }
  }

  .search-result-row:active {
    background: var(--button-press);
  }

  .search-thumb {
    width: 48px;
    height: 48px;
    border-radius: 4px;
    object-fit: cover;
    flex-shrink: 0;
  }

  .search-thumb-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    border-radius: 4px;
    background: var(--button-elevated);
    color: var(--gray);
  }

  .search-thumb-placeholder svg {
    pointer-events: none;
  }

  .search-result-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .search-result-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .search-result-meta {
    font-size: 11.5px;
    font-weight: 500;
    color: var(--gray);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .feedback-sep {
    opacity: 0.5;
    margin: 0 2px;
  }
</style>
