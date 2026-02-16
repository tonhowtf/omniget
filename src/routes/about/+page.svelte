<script lang="ts">
  import { open } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";

  const APP_VERSION = "0.1.0";
  const GITHUB_URL = "https://github.com/tonhowtf/omniget";

  async function openGitHub() {
    await open(GITHUB_URL);
  }
</script>

<div class="about">
  <div class="header">
    <img src="/loop.png" alt="OmniGet" class="logo" width="80" height="80" />
    <h2>OmniGet</h2>
    <p class="description">{$t('about.description')}</p>
  </div>

  <section class="card">
    <h5 class="card-title">{$t('about.terms_title')}</h5>
    <p class="card-text">{$t('about.terms_text')}</p>
  </section>

  <section class="card">
    <h5 class="card-title">{$t('about.legal_title')}</h5>
    <p class="card-text">{$t('about.legal_text')}</p>
  </section>

  <section class="card">
    <h5 class="card-title">{$t('about.responsibility_title')}</h5>
    <p class="card-text">{$t('about.responsibility_text')}</p>
  </section>

  <section class="card">
    <h5 class="card-title">{$t('about.mascot_title')}</h5>
    <p class="card-text">{$t('about.mascot_text')}</p>
  </section>

  <section class="card">
    <h5 class="card-title">{$t('about.license_title')}</h5>
    <p class="card-text">{$t('about.license_text')}</p>
  </section>

  <section class="star-section">
    <p class="star-description">{$t('about.star_description')}</p>
    <button class="star-button" onclick={openGitHub}>
      <svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor" stroke="none">
        <path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
      </svg>
      {$t('about.star_button')}
    </button>
  </section>

  <p class="version">{$t('about.version')} {APP_VERSION}</p>
</div>

<style>
  .about {
    display: flex;
    flex-direction: column;
    align-items: center;
    min-height: calc(100vh - var(--padding) * 4);
    padding-top: calc(var(--padding) * 3);
    padding-left: calc(var(--padding) * 1.5);
    padding-right: calc(var(--padding) * 1.5);
    padding-bottom: calc(var(--padding) * 3);
    gap: calc(var(--padding) * 2);
    max-width: 480px;
    margin: 0 auto;
  }

  .header {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--padding) / 2);
    text-align: center;
  }

  .logo {
    border-radius: calc(var(--border-radius) + 4px);
    pointer-events: none;
  }

  .description {
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
    padding: calc(var(--padding) + 4px) calc(var(--padding) + 4px);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 2);
  }

  .card-title {
    color: var(--secondary);
  }

  .card-text {
    font-size: 13px;
    font-weight: 400;
    line-height: 1.8;
    color: var(--gray);
    user-select: text;
  }

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

  .version {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }
</style>
