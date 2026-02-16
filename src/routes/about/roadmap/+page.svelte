<script lang="ts">
  import { t } from "$lib/i18n";
</script>

<div class="roadmap">
  <p class="roadmap-description">{$t('about.roadmap_description')}</p>
</div>

<style>
  .roadmap {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: calc(var(--padding) * 2) 0;
  }

  .roadmap-description {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--gray);
    text-align: center;
  }
</style>
