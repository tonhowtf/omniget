<script lang="ts">
  import { t } from "$lib/i18n";

  type CardDownloadStatus = "idle" | "downloading" | "complete" | "error";

  type CourseCardProps = {
    name: string;
    price: string;
    imageUrl?: string;
    externalPlatform?: boolean;
    downloadStatus?: CardDownloadStatus;
    downloadPercent?: number;
    onDownload: () => void;
  };

  let {
    name,
    price,
    imageUrl,
    externalPlatform = false,
    downloadStatus = "idle",
    downloadPercent = 0,
    onDownload,
  }: CourseCardProps = $props();

  let isDisabled = $derived(
    downloadStatus === "downloading" || downloadStatus === "complete"
  );

  function handleClick() {
    if (isDisabled) return;
    onDownload();
  }
</script>

<div class="course-card">
  <div class="card-image">
    {#if imageUrl}
      <img src={imageUrl} alt={name} loading="lazy" />
    {:else}
      <div class="card-placeholder">
        <svg
          width="48"
          height="48"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M22 9l-10 -4l-10 4l10 4l10 -4v6" />
          <path d="M6 10.6v5.4a6 3 0 0 0 12 0v-5.4" />
        </svg>
      </div>
    {/if}
  </div>

  <div class="card-body">
    <h4 class="card-title" title={name}>{name}</h4>
    <div class="card-meta">
      <span class="card-price">{price}</span>
      {#if externalPlatform}
        <span class="card-badge external">{$t("hotmart.external_platform")}</span>
      {/if}
    </div>
    {#if externalPlatform}
      <button class="button elevated card-download" disabled>
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M18 6L6 18M6 6l12 12" />
        </svg>
        {$t("hotmart.unavailable")}
      </button>
    {:else if downloadStatus === "downloading"}
      <button class="button elevated card-download status-downloading" disabled>
        <span class="btn-spinner"></span>
        {$t("hotmart.downloading")}
      </button>
      <div class="mini-progress-track">
        <div
          class="mini-progress-fill"
          style="width: {downloadPercent.toFixed(1)}%"
        ></div>
      </div>
    {:else if downloadStatus === "complete"}
      <button class="button elevated card-download status-complete" disabled>
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M5 12l5 5l10 -10" />
        </svg>
        {$t("hotmart.downloaded")}
      </button>
    {:else if downloadStatus === "error"}
      <button class="button elevated card-download status-error" onclick={handleClick}>
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M12 9v4" />
          <path d="M12 17h.01" />
          <path d="M3 12a9 9 0 1 0 18 0a9 9 0 0 0 -18 0" />
        </svg>
        {$t("hotmart.download_retry")}
      </button>
    {:else}
      <button class="button elevated card-download" onclick={handleClick}>
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2 -2v-2" />
          <polyline points="7 11 12 16 17 11" />
          <line x1="12" y1="4" x2="12" y2="16" />
        </svg>
        {$t("hotmart.download_btn")}
      </button>
    {/if}
  </div>
</div>

<style>
  .course-card {
    background: var(--button);
    border-radius: var(--border-radius);
    box-shadow: var(--button-box-shadow);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .card-image {
    width: 100%;
    aspect-ratio: 16 / 9;
    overflow: hidden;
    border-radius: var(--border-radius) var(--border-radius) 0 0;
    background: var(--button-elevated);
  }

  .card-image img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    pointer-events: none;
  }

  .card-placeholder {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .card-placeholder svg {
    stroke: var(--gray);
    opacity: 0.5;
    pointer-events: none;
  }

  .card-body {
    padding: var(--padding);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 2);
  }

  .card-title {
    font-size: 14.5px;
    font-weight: 500;
    margin-block: 0;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    text-overflow: ellipsis;
    line-height: 1.3;
    color: var(--secondary);
    user-select: text;
  }

  .card-meta {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
    flex-wrap: wrap;
  }

  .card-price {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .card-badge {
    font-size: 10px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 2px calc(var(--padding) / 2);
    border-radius: calc(var(--border-radius) / 2);
  }

  .card-badge.external {
    background: var(--orange);
    color: #000;
  }

  .card-download {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: calc(var(--padding) / 2);
    margin-top: calc(var(--padding) / 2);
  }

  .card-download:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .card-download svg {
    pointer-events: none;
  }

  .status-downloading {
    opacity: 0.8;
    cursor: default;
  }

  .status-downloading:disabled {
    opacity: 0.8;
  }

  .status-complete {
    background: var(--green);
    color: #000;
  }

  .status-complete:disabled {
    opacity: 1;
  }

  .status-error {
    background: var(--red);
    color: #fff;
  }

  .btn-spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--input-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .mini-progress-track {
    width: 100%;
    height: 4px;
    background: var(--button-elevated);
    border-radius: 2px;
    overflow: hidden;
  }

  .mini-progress-fill {
    height: 100%;
    background: var(--blue);
    border-radius: 2px;
    transition: width 0.3s ease-out;
  }
</style>
