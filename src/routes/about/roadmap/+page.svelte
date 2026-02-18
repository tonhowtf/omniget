<script lang="ts">
  import { open } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";

  const GITHUB_ISSUES_URL = "https://github.com/tonhowtf/omniget/issues";

  async function openIssues() {
    await open(GITHUB_ISSUES_URL);
  }
</script>

<p class="roadmap-intro">{$t('about.roadmap_intro')}</p>

<section class="card">
  <h5 class="card-title">{$t('about.roadmap_platforms_title')}</h5>
  <div class="checklist">
    <div class="check-item"><span class="checkbox"></span> Hubla</div>
    <div class="check-item"><span class="checkbox"></span> Kiwify</div>
  </div>
</section>

<section class="card">
  <h5 class="card-title">{$t('about.roadmap_features_title')}</h5>
  <div class="checklist">
    <div class="check-item"><span class="checkbox"></span> {$t('about.roadmap_feature_telegram_multi_account')}</div>
  </div>
</section>

<section class="card community-card">
  <h5 class="card-title">{$t('about.roadmap_community_title')}</h5>
  <p class="card-text">{$t('about.roadmap_community_text')}</p>
  <div class="community-links">
    <div class="community-row">
      <span class="community-label">{$t('about.roadmap_community_discord')}</span>
      <span class="community-value">@tonhowtf</span>
    </div>
    <div class="community-row">
      <span class="community-label">{$t('about.roadmap_community_issue')}</span>
      <button class="button community-btn" onclick={openIssues}>GitHub Issues</button>
    </div>
  </div>
</section>

<style>
  .roadmap-intro {
    font-size: 13px;
    font-weight: 400;
    line-height: 1.8;
    color: var(--gray);
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

  .card-text {
    font-size: 13px;
    font-weight: 400;
    line-height: 1.8;
    color: var(--gray);
    user-select: text;
  }

  .checklist {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .check-item {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
    font-size: 13px;
    font-weight: 400;
    color: var(--gray);
    cursor: default;
  }

  .checkbox {
    width: 18px;
    height: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    font-size: 11px;
    font-weight: 600;
    flex-shrink: 0;
    border: 1.5px solid var(--input-border);
    color: transparent;
  }

  .community-links {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 0.75);
  }

  .community-row {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
    flex-wrap: wrap;
  }

  .community-label {
    font-size: 13px;
    font-weight: 400;
    color: var(--gray);
  }

  .community-value {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
    user-select: text;
  }

  .community-btn {
    font-size: 12.5px;
    padding: calc(var(--padding) / 3) calc(var(--padding) * 0.75);
  }
</style>
