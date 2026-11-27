<script lang="ts">
  import { open } from "@tauri-apps/plugin-shell";
  import { getVersion } from "@tauri-apps/api/app";
  import { onMount } from "svelte";
  import { t, locales } from "$lib/i18n";
  import { WMB_CREDITS } from "$lib/washmybelly/credits";

  const GITHUB_URL = "https://github.com/tonhowtf/omniget";
  let version = $state("");

  onMount(() => {
    void getVersion().then((value) => { version = value; }).catch(() => {});
  });

  async function openGitHub() {
    await open(GITHUB_URL);
  }
</script>

<div class="project-page">
  <section class="surface-card project-hero">
    <img src="/loop.png" alt="Loop" class="project-logo" width="64" height="64" draggable="false" />
    <div class="project-hero-copy">
      <div class="project-title-row">
        <h1>OmniGet</h1>
        {#if version}<span class="tag">{$t("about.version")} {version}</span>{/if}
      </div>
      <p class="project-description">{$t("about.description")}</p>
      <div class="project-actions">
        <button type="button" class="btn btn-primary btn-sm" onclick={openGitHub}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" stroke="none" aria-hidden="true">
            <path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
          </svg>
          {$t("about.star_button")}
        </button>
        <span class="star-hint">{$t("about.star_description")}</span>
      </div>
    </div>
  </section>

  <section class="surface-card section-card">
    <h2 class="section-title">{$t("about.features_title")}</h2>
    <ul class="features-grid">
      <li>{$t("about.feature_platforms")}</li>
      <li>{$t("about.feature_progress")}</li>
      <li>{$t("about.feature_mascot")}</li>
      <li>{$t("about.feature_themes")}</li>
      <li>{$t("about.feature_i18n_count", { count: $locales.length })}</li>
      <li>{$t("about.feature_tech")}</li>
    </ul>
  </section>

  <section class="surface-card section-card">
    <h2 class="section-title">{$t("about.wmb_credits_title")}</h2>
    <p class="section-body">{$t("about.wmb_credits_body")}</p>
    <ul class="credits-list">
      {#each WMB_CREDITS as c (c.name)}
        <li class="credit-row">
          <button type="button" class="credit-link" onclick={() => open(c.url)}>{c.name}</button>
          <span class="credit-license">{c.license}</span>
          <span class="credit-use">{$t(`about.wmb_credit_${c.use}`)}</span>
        </li>
      {/each}
    </ul>
  </section>
</div>

<style>
  .project-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .project-hero {
    display: flex;
    align-items: flex-start;
    gap: var(--space-4);
    padding: var(--space-5);
  }

  .project-logo {
    border-radius: var(--radius-lg);
    pointer-events: none;
    flex-shrink: 0;
  }

  .project-hero-copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .project-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .project-hero-copy h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
  }

  .project-description {
    margin: 0;
    font-size: var(--text-base);
    line-height: 1.65;
    color: var(--text-muted);
    max-width: 58ch;
  }

  .project-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-3);
    padding-top: var(--space-1);
  }

  .star-hint {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .section-card {
    padding: var(--space-4) var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .section-title {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
    color: var(--text);
  }

  .section-body {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.6;
    color: var(--text-muted);
  }

  .features-grid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: var(--space-2) var(--space-4);
  }

  .features-grid li {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    font-size: var(--text-base);
    line-height: 1.55;
    color: var(--text-muted);
  }

  .features-grid li::before {
    content: "";
    flex: 0 0 auto;
    width: 5px;
    height: 5px;
    margin-top: calc(0.775em - 2.5px);
    border-radius: 50%;
    background: var(--accent);
  }

  .credits-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }

  .credit-row {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-2);
    padding: var(--space-2) 0;
    border-top: var(--hairline) solid var(--separator);
    font-size: var(--text-sm);
  }

  .credit-row:first-child {
    border-top: none;
    padding-top: 0;
  }

  .credit-link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-weight: 600;
    text-decoration: underline;
    text-underline-offset: 2px;
    cursor: pointer;
  }

  .credit-link:hover {
    color: var(--accent-text, var(--accent));
  }

  .credit-license {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
    background: var(--surface-hi);
    padding: 1px 6px;
    border-radius: var(--radius-xs);
  }

  .credit-use {
    color: var(--text-muted);
  }
</style>
