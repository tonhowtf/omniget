<script lang="ts">
  import { t } from "$lib/i18n";

  const sections = [
    { title: "about.terms_title", body: "about.terms_text" },
    { title: "about.legal_title", body: "about.legal_text" },
    { title: "about.responsibility_title", body: "about.responsibility_text" },
    { title: "about.mascot_title", body: "about.mascot_text" },
    { title: "about.license_title", body: "about.license_text" },
  ] as const;
</script>

<div class="terms-page">
  <header class="terms-head">
    <h1>{$t("about.tab.terms")}</h1>
  </header>

  <div class="surface-card terms-list">
    {#each sections as sec (sec.title)}
      <section class="term-block">
        <h2>{$t(sec.title)}</h2>
        <p>{$t(sec.body)}</p>
      </section>
    {/each}
  </div>
</div>

<style>
  .terms-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .terms-head h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    line-height: var(--leading-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
  }

  .terms-list {
    padding: 0;
    overflow: hidden;
  }

  .term-block {
    padding: var(--space-4) var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .term-block + .term-block {
    border-top: var(--hairline) solid var(--separator);
  }

  .term-block h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
    color: var(--text);
  }

  .term-block p {
    margin: 0;
    font-size: var(--text-base);
    line-height: 1.7;
    color: var(--text-muted);
    max-width: 66ch;
    user-select: text;
  }
</style>
