<script lang="ts">
  import { open } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";

  const GITHUB_URL = "https://github.com/tonhowtf/omniget";
  const APP_VERSION = "0.2.6";

  async function openGitHub() {
    await open(GITHUB_URL);
  }
</script>

<section class="star-section">
  <p class="star-description">{$t('about.star_description')}</p>
  <div class="star-iframe-wrapper">
    <iframe
      src="https://ghbtns.com/github-btn.html?user=tonhowtf&repo=omniget&type=star&count=true&size=large"
      frameborder="0"
      scrolling="0"
      width="170"
      height="30"
      title="GitHub Stars"
    ></iframe>
  </div>
  <button class="star-button" onclick={openGitHub}>
    <svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor" stroke="none">
      <path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
    </svg>
    {$t('about.star_button')}
  </button>
</section>

<section class="project-header">
  <img src="/loop.png" alt="Loop" class="project-logo" width="64" height="64" />
  <p class="project-description">{$t('about.description')}</p>
</section>

<section class="card">
  <h5 class="card-title">{$t('about.features_title')}</h5>
  <div class="features-list">
    <p class="feature-item">{$t('about.feature_platforms')}</p>
    <p class="feature-item">{$t('about.feature_hotmart')}</p>
    <p class="feature-item">{$t('about.feature_progress')}</p>
    <p class="feature-item">{$t('about.feature_mascot')}</p>
    <p class="feature-item">{$t('about.feature_themes')}</p>
    <p class="feature-item">{$t('about.feature_i18n')}</p>
    <p class="feature-item">{$t('about.feature_tech')}</p>
  </div>
</section>

<p class="version">{$t('about.version')} {APP_VERSION}</p>

<style>
  .star-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    width: 100%;
  }

  .star-description {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--secondary);
  }

  .star-iframe-wrapper {
    display: flex;
    justify-content: center;
  }

  .star-button {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: calc(var(--padding) / 2);
    width: 100%;
    padding: calc(var(--padding) + 2px) calc(var(--padding) * 2);
    font-size: 14.5px;
    font-weight: 500;
    color: #fff;
    background: var(--orange);
    border: none;
    border-radius: var(--border-radius);
    cursor: pointer;
    transition: transform 0.15s;
  }

  .star-button svg {
    pointer-events: none;
    flex-shrink: 0;
  }

  @media (hover: hover) {
    .star-button:hover {
      transform: scale(1.05);
    }
  }

  .star-button:active {
    transform: scale(0.98);
  }

  .star-button:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .project-header {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--padding) / 2);
    text-align: center;
  }

  .project-logo {
    border-radius: calc(var(--border-radius) + 4px);
    pointer-events: none;
  }

  .project-description {
    font-size: 14.5px;
    font-weight: 400;
    line-height: 1.8;
    color: var(--gray);
    max-width: 320px;
  }

  .card {
    width: 100%;
    background: var(--button);
    box-shadow: var(--button-box-shadow);
    border-radius: var(--border-radius);
    padding: calc(var(--padding) + 4px);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 2);
  }

  .card-title {
    color: var(--secondary);
  }

  .features-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .feature-item {
    font-size: 13px;
    font-weight: 400;
    line-height: 1.6;
    color: var(--gray);
  }

  .feature-item::before {
    content: "• ";
    color: var(--blue);
  }

  .version {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    text-align: center;
  }
</style>
