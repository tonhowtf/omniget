<script lang="ts">
  import { t } from "$lib/i18n";
</script>

<p class="roadmap-intro">{$t('about.roadmap_intro')}</p>

<section class="card">
  <h5 class="card-title">{$t('about.roadmap_platforms_title')}</h5>
  <div class="checklist">
    <label class="check-item done"><span class="checkbox">✓</span> YouTube</label>
    <label class="check-item done"><span class="checkbox">✓</span> Instagram</label>
    <label class="check-item done"><span class="checkbox">✓</span> TikTok</label>
    <label class="check-item done"><span class="checkbox">✓</span> Twitter / X</label>
    <label class="check-item done"><span class="checkbox">✓</span> Reddit</label>
    <label class="check-item done"><span class="checkbox">✓</span> Twitch</label>
    <label class="check-item done"><span class="checkbox">✓</span> Pinterest</label>
    <label class="check-item done"><span class="checkbox">✓</span> Bluesky</label>
    <label class="check-item done"><span class="checkbox">✓</span> Hotmart</label>
    <label class="check-item"><span class="checkbox"></span> Hubla</label>
    <label class="check-item"><span class="checkbox"></span> Kiwify</label>
    <label class="check-item"><span class="checkbox"></span> Udemy</label>
  </div>
</section>

<section class="card">
  <h5 class="card-title">{$t('about.roadmap_features_title')}</h5>
  <div class="checklist">
    <label class="check-item done"><span class="checkbox">✓</span> {$t('about.roadmap_feature_themes')}</label>
    <label class="check-item done"><span class="checkbox">✓</span> {$t('about.roadmap_feature_i18n')}</label>
    <label class="check-item done"><span class="checkbox">✓</span> {$t('about.roadmap_feature_hotmart_login')}</label>
    <label class="check-item"><span class="checkbox"></span> {$t('about.roadmap_feature_instagram_login')}</label>
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
