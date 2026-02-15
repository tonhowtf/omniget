<script lang="ts">
  type CourseCardProps = {
    name: string;
    price: string;
    imageUrl?: string;
    onDownload: () => void;
  };

  let { name, price, imageUrl, onDownload }: CourseCardProps = $props();
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
    <span class="card-price">{price}</span>
    <button class="button elevated card-download" onclick={onDownload}>
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
      Baixar
    </button>
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

  .card-price {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .card-download {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: calc(var(--padding) / 2);
    margin-top: calc(var(--padding) / 2);
  }

  .card-download svg {
    pointer-events: none;
  }
</style>
