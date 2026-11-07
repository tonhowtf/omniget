<script lang="ts">
  import { t } from "$lib/i18n";
</script>

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

<style>
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
</style>
