<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import {
    getChangelogBody,
    getCurrentVersion,
    fetchChangelog,
  } from "$lib/stores/changelog-store.svelte";

  let body = $derived(getChangelogBody());
  let version = $derived(getCurrentVersion());
  let loading = $state(true);

  onMount(async () => {
    await fetchChangelog();
    loading = false;
  });

  function renderMarkdown(md: string): string {
    return md
      .split("\n")
      .map((line) => {
        if (line.startsWith("### ")) {
          return `<h4>${escapeHtml(line.slice(4))}</h4>`;
        }
        if (line.startsWith("## ")) {
          return `<h3>${escapeHtml(line.slice(3))}</h3>`;
        }
        if (line.startsWith("# ")) {
          return `<h2>${escapeHtml(line.slice(2))}</h2>`;
        }
        if (line.startsWith("- ") || line.startsWith("* ")) {
          return `<li>${formatInline(line.slice(2))}</li>`;
        }
        if (line.trim() === "") {
          return "<br />";
        }
        return `<p>${formatInline(line)}</p>`;
      })
      .join("");
  }

  function escapeHtml(str: string): string {
    return str
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function formatInline(str: string): string {
    let result = escapeHtml(str);
    result = result.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
    result = result.replace(/`(.+?)`/g, "<code>$1</code>");
    result = result.replace(
      /\[(.+?)\]\((.+?)\)/g,
      '<a href="$2" target="_blank" rel="noopener">$1</a>'
    );
    return result;
  }
</script>

<div class="changelog-page">
  {#if version}
    <div class="version-row">
      <span class="version-label">{$t("about.version")}</span>
      <span class="version-value">{version}</span>
    </div>
  {/if}

  {#if loading}
    <div class="loading">
      <span class="spinner"></span>
    </div>
  {:else if body}
    <div class="card">
      <div class="markdown-content">
        {@html renderMarkdown(body)}
      </div>
    </div>
  {:else}
    <div class="card empty-card">
      <p class="empty-text">{$t("changelog.empty")}</p>
    </div>
  {/if}
</div>

<style>
  .changelog-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
  }

  .version-row {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: calc(var(--padding) / 2);
  }

  .version-label {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .version-value {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--blue);
  }

  .loading {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: calc(var(--padding) * 3) 0;
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 2px solid var(--input-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .card {
    width: 100%;
    background: var(--button);
    box-shadow: var(--button-box-shadow);
    border-radius: var(--border-radius);
    padding: calc(var(--padding) + 4px);
  }

  .empty-card {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: calc(var(--padding) * 2);
  }

  .empty-text {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .markdown-content {
    font-size: 13px;
    font-weight: 400;
    line-height: 1.7;
    color: var(--secondary);
  }

  .markdown-content :global(h2) {
    font-size: 18px;
    font-weight: 500;
    margin: 0 0 calc(var(--padding) / 2);
    letter-spacing: -0.5px;
  }

  .markdown-content :global(h3) {
    font-size: 15px;
    font-weight: 500;
    margin: var(--padding) 0 calc(var(--padding) / 2);
  }

  .markdown-content :global(h4) {
    font-size: 13px;
    font-weight: 500;
    margin: var(--padding) 0 calc(var(--padding) / 4);
    color: var(--gray);
  }

  .markdown-content :global(p) {
    margin: 0 0 4px;
  }

  .markdown-content :global(li) {
    margin: 0 0 4px;
    padding-left: calc(var(--padding) / 2);
    list-style: none;
  }

  .markdown-content :global(li::before) {
    content: "•";
    color: var(--blue);
    margin-right: 6px;
  }

  .markdown-content :global(strong) {
    font-weight: 600;
  }

  .markdown-content :global(code) {
    font-size: 12px;
    padding: 1px 5px;
    background: var(--button-elevated);
    border-radius: 4px;
  }

  .markdown-content :global(a) {
    color: var(--blue);
    text-decoration: none;
  }

  @media (hover: hover) {
    .markdown-content :global(a:hover) {
      text-decoration: underline;
    }
  }

  .markdown-content :global(br) {
    display: block;
    content: "";
    margin-top: 4px;
  }
</style>
