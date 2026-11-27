<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { renderMarkdown } from "$lib/changelog-markdown";
  import {
    getChangelogBody,
    getCurrentVersion,
    fetchChangelog,
  } from "$lib/stores/changelog-store.svelte";

  let body = $derived(getChangelogBody());
  let version = $derived(getCurrentVersion());
  let loading = $state(true);
  let failed = $state(false);

  onMount(async () => {
    try {
      await fetchChangelog();
    } catch {
      failed = true;
    } finally {
      loading = false;
    }
  });
</script>

<div class="changelog-page" data-agent-readiness={loading ? "loading" : failed ? "error" : "ready"}>
  <header class="changelog-head">
    <h1>{$t("about.tab.changelog")}</h1>
    {#if version}
      <span class="tag">{$t("about.version")} {version}</span>
    {/if}
  </header>

  {#if loading}
    <div class="surface-card loading" role="status">
      <span class="spinner" aria-hidden="true"></span>
      <span class="loading-text">{$t("common.loading")}</span>
    </div>
  {:else if body}
    <article class="surface-card changelog-card">
      <div class="markdown-content">
        {@html renderMarkdown(body)}
      </div>
    </article>
  {:else}
    <div class="surface-card empty-card">
      <p class="empty-text">{$t("changelog.empty")}</p>
    </div>
  {/if}
</div>

<style>
  .changelog-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .changelog-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .changelog-head h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    line-height: var(--leading-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
  }

  .loading,
  .empty-card {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-6);
  }

  .loading-text,
  .empty-text {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .spinner {
    width: 18px;
    height: 18px;
    border: 2px solid var(--content-border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner { animation: none; }
  }

  .changelog-card {
    padding: var(--space-5);
  }

  .markdown-content {
    font-size: var(--text-base);
    line-height: 1.7;
    color: var(--text-muted);
    max-width: 68ch;
  }

  .markdown-content :global(h2) {
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--text);
    margin: 0 0 var(--space-2);
    letter-spacing: var(--track-snug);
  }

  .markdown-content :global(h3) {
    font-size: var(--text-md);
    font-weight: 600;
    color: var(--text);
    margin: var(--space-4) 0 var(--space-2);
  }

  .markdown-content :global(h4) {
    font-size: var(--text-base);
    font-weight: 600;
    margin: var(--space-3) 0 var(--space-1);
    color: var(--text-muted);
  }

  .markdown-content :global(p) {
    margin: 0 0 var(--space-2);
  }

  .markdown-content :global(li) {
    margin: 0 0 6px;
    padding-inline-start: var(--space-2);
    list-style: none;
  }

  .markdown-content :global(li::before) {
    content: "•";
    color: var(--accent);
    margin-inline-end: 8px;
  }

  .markdown-content :global(strong) {
    font-weight: 600;
    color: var(--text);
  }

  .markdown-content :global(pre) {
    background: var(--fill-1);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    overflow-x: auto;
    margin: var(--space-3) 0;
  }

  .markdown-content :global(pre code) {
    background: transparent;
    padding: 0;
  }

  .markdown-content :global(hr) {
    border: none;
    height: var(--hairline);
    background: var(--separator);
    margin: var(--space-5) 0;
  }

  .markdown-content :global(code) {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    padding: 1px 5px;
    background: var(--surface-hi);
    border-radius: var(--radius-xs);
    color: var(--text);
  }

  .markdown-content :global(a) {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .markdown-content :global(br) {
    display: block;
    content: "";
    margin-top: 4px;
  }
</style>
