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
    const out: string[] = [];
    let inFence = false;
    let fenceLines: string[] = [];
    for (const line of md.split("\n")) {
      if (line.trim().startsWith("```")) {
        if (inFence) {
          out.push(`<pre><code>${escapeHtml(fenceLines.join("\n"))}</code></pre>`);
          fenceLines = [];
        }
        inFence = !inFence;
        continue;
      }
      if (inFence) {
        fenceLines.push(line);
        continue;
      }
      if (/^\s*(---|\*\*\*|___)\s*$/.test(line)) {
        out.push("<hr />");
      } else if (line.startsWith("### ")) {
        out.push(`<h4>${escapeHtml(line.slice(4))}</h4>`);
      } else if (line.startsWith("## ")) {
        out.push(`<h3>${escapeHtml(line.slice(3))}</h3>`);
      } else if (line.startsWith("# ")) {
        out.push(`<h2>${escapeHtml(line.slice(2))}</h2>`);
      } else if (line.startsWith("- ") || line.startsWith("* ")) {
        out.push(`<li>${formatInline(line.slice(2))}</li>`);
      } else if (line.trim() === "") {
        out.push("<br />");
      } else {
        out.push(`<p>${formatInline(line)}</p>`);
      }
    }
    if (inFence && fenceLines.length) {
      out.push(`<pre><code>${escapeHtml(fenceLines.join("\n"))}</code></pre>`);
    }
    return out.join("");
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
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--gray);
  }

  .version-value {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--accent);
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
    border-top-color: var(--accent);
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
    font-size: var(--text-base);
    font-weight: 500;
    color: var(--gray);
  }

  .markdown-content {
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.7;
    color: var(--secondary);
  }

  .markdown-content :global(h2) {
    font-size: var(--text-lg);
    font-weight: 600;
    margin: 0 0 calc(var(--padding) / 2);
    letter-spacing: var(--track-snug);
  }

  .markdown-content :global(h3) {
    font-size: var(--text-md);
    font-weight: 600;
    margin: var(--padding) 0 calc(var(--padding) / 2);
  }

  .markdown-content :global(h4) {
    font-size: var(--text-base);
    font-weight: 600;
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
    color: var(--accent);
    margin-right: 6px;
  }

  .markdown-content :global(strong) {
    font-weight: 600;
  }

  .markdown-content :global(pre) {
    background: var(--fill-1);
    border-radius: var(--radius-sm);
    padding: var(--space-3);
    overflow-x: auto;
    margin: var(--space-2) 0;
  }

  .markdown-content :global(pre code) {
    background: transparent;
    padding: 0;
  }

  .markdown-content :global(hr) {
    border: none;
    height: 1px;
    background: var(--border);
    margin: var(--space-4) 0;
  }

  .markdown-content :global(code) {
    font-size: 12px;
    padding: 1px 5px;
    background: var(--button-elevated);
    border-radius: 4px;
  }

  .markdown-content :global(a) {
    color: var(--accent);
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
