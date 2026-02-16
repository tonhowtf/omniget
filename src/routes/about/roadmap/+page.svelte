<script lang="ts">
  import { t } from "$lib/i18n";
</script>

<p class="roadmap-intro">{$t('about.roadmap_intro')}</p>

<section class="card">
  <h5 class="card-title">{$t('about.roadmap_platforms_title')}</h5>
  <div class="checklist">
    <div class="check-item done"><span class="checkbox">✓</span> YouTube</div>
    <div class="check-item done"><span class="checkbox">✓</span> Instagram</div>
    <div class="check-item done"><span class="checkbox">✓</span> TikTok</div>
    <div class="check-item done"><span class="checkbox">✓</span> Twitter / X</div>
    <div class="check-item done"><span class="checkbox">✓</span> Reddit</div>
    <div class="check-item done"><span class="checkbox">✓</span> Twitch</div>
    <div class="check-item done"><span class="checkbox">✓</span> Pinterest</div>
    <div class="check-item done"><span class="checkbox">✓</span> Bluesky</div>
    <div class="check-item done"><span class="checkbox">✓</span> Hotmart</div>
    <div class="check-item"><span class="checkbox"></span> Hubla</div>
    <div class="check-item"><span class="checkbox"></span> Kiwify</div>
    <div class="check-item"><span class="checkbox"></span> Udemy</div>
  </div>
</section>

<section class="card">
  <h5 class="card-title">{$t('about.roadmap_features_title')}</h5>
  <div class="checklist">
    <div class="check-item done"><span class="checkbox">✓</span> {$t('about.roadmap_feature_themes')}</div>
    <div class="check-item done"><span class="checkbox">✓</span> {$t('about.roadmap_feature_i18n')}</div>
    <div class="check-item done"><span class="checkbox">✓</span> {$t('about.roadmap_feature_hotmart_login')}</div>
    <div class="check-item"><span class="checkbox"></span> {$t('about.roadmap_feature_instagram_login')}</div>
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

  .check-item.done {
    color: var(--secondary);
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

  .check-item.done .checkbox {
    background: var(--green);
    border-color: var(--green);
    color: #000;
  }
</style>
