<script lang="ts">
  import { t } from "$lib/i18n";
  import PageHero from "$lib/study-components/PageHero.svelte";

  interface Props {
    title: string;
    summary: string;
    bullets?: string[];
  }

  let { title, summary, bullets = [] }: Props = $props();
</script>

<section class="study-page">
  <PageHero {title} subtitle={$t("study.anki.stub.under_construction")} />

  <div class="card">
    <div class="card-body">
      <p class="lede">{summary}</p>
      {#if bullets.length > 0}
        <ul>
          {#each bullets as item (item)}
            <li>{item}</li>
          {/each}
        </ul>
      {/if}
    </div>
    <footer class="card-foot">
      <a class="back-link" href="/study/anki">← {$t("study.anki.sidebar.dashboard")}</a>
    </footer>
  </div>
</section>

<style>
  .study-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
    width: 100%;
    max-width: 900px;
    margin-inline: auto;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding: calc(var(--padding) * 2);
    border: 1px dashed color-mix(in oklab, var(--accent) 35%, var(--input-border));
    border-radius: var(--border-radius);
    background: color-mix(in oklab, var(--accent) 4%, transparent);
  }
  .card-body {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .lede {
    margin: 0;
    color: var(--secondary);
    font-size: 14px;
    line-height: 1.5;
  }
  ul {
    margin: 0;
    padding-left: 20px;
    color: var(--tertiary);
    font-size: 13px;
    line-height: 1.7;
  }
  .card-foot {
    display: flex;
    justify-content: flex-end;
    padding-top: var(--padding);
    border-top: none;
  }
  .back-link {
    color: var(--accent);
    text-decoration: none;
    font-size: 13px;
    padding: 4px 10px;
    border-radius: var(--border-radius);
    transition: background 150ms ease;
  }
  .back-link:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .back-link:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }

  @media (prefers-reduced-motion: reduce) {
    .back-link {
      transition: none;
    }
  }
</style>
