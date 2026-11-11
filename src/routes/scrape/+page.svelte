<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/devtools/api";
  import { removedSummary } from "$lib/devtools/helpers";
  import type { DevtoolsError, ScrapeFormat, ScrapePage, ScrapeSelection } from "$lib/devtools/types";
  import DevShell from "$components/devtools/DevShell.svelte";
  import ErrorNotice from "$components/devtools/ErrorNotice.svelte";
  import CopyButton from "$components/palette/CopyButton.svelte";

  type Mode = "page" | "select";
  const MODES: Mode[] = ["page", "select"];
  const FORMATS: ScrapeFormat[] = ["markdown", "text", "html"];
  const FORMAT_LABELS: Record<ScrapeFormat, string> = { markdown: "Markdown", text: "", html: "HTML" };

  let mode = $state<Mode>("page");
  let url = $state("");
  let format = $state<ScrapeFormat>("markdown");
  let maxTokens = $state(4000);
  let selector = $state("");
  let attr = $state("");
  let busy = $state(false);
  let pageResult = $state<ScrapePage | null>(null);
  let selection = $state<ScrapeSelection | null>(null);
  let error = $state<DevtoolsError | null>(null);

  let removed = $derived(pageResult ? removedSummary(pageResult.removed) : "");
  let ready = $derived(url.trim() !== "" && (mode === "page" || selector.trim() !== ""));

  function count(n: number): string {
    return n.toLocaleString();
  }

  async function run() {
    if (!ready || busy) return;
    busy = true;
    error = null;
    try {
      if (mode === "page") {
        pageResult = null;
        pageResult = await api.scrapeFetch({ url: url.trim(), format, maxTokens: maxTokens > 0 ? maxTokens : undefined });
      } else {
        selection = null;
        selection = await api.scrapeSelect({ url: url.trim(), selector: selector.trim(), attr: attr.trim() || undefined });
      }
    } catch (e) {
      error = api.parseError(e);
    } finally {
      busy = false;
    }
  }

  function retry(candidate: string) {
    selector = candidate;
    void run();
  }

  function pick(next: Mode) {
    mode = next;
    error = null;
  }
</script>

{#snippet browserLink()}
  <a class="btn btn-secondary btn-sm" href="/global?url={encodeURIComponent(url.trim())}">{$t("scrape.open_browser")}</a>
{/snippet}

<DevShell icon="scraper" title={$t("scrape.title") as string} subtitle={$t("scrape.subtitle") as string} {busy}>
  <div class="segmented dv-tabs" role="tablist" aria-label={$t("scrape.title") as string}>
    {#each MODES as id (id)}
      <button type="button" role="tab" class="segmented-btn" class:active={mode === id} aria-selected={mode === id} onclick={() => pick(id)}>
        {$t(`scrape.mode_${id}`)}
      </button>
    {/each}
  </div>

  <form
    class="dv-panel"
    onsubmit={(e) => {
      e.preventDefault();
      void run();
    }}
  >
    <div class="dv-row">
      <label class="dv-field wide">
        {$t("scrape.url")}
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input input-lg dv-mono" type="text" inputmode="url" bind:value={url} placeholder="https://example.com" spellcheck="false" autocomplete="off" autofocus />
      </label>
      <button type="submit" class="btn btn-primary btn-lg" disabled={!ready || busy}>
        {#if busy}<span class="spinner" aria-hidden="true"></span>{/if}
        {mode === "page" ? $t("scrape.fetch") : $t("scrape.find")}
      </button>
    </div>
    {#if mode === "page"}
      <div class="dv-row">
        <div class="dv-field fixed">
          {$t("scrape.format")}
          <div class="segmented" role="radiogroup" aria-label={$t("scrape.format") as string}>
            {#each FORMATS as id (id)}
              <button type="button" role="radio" class="segmented-btn" class:active={format === id} aria-checked={format === id} onclick={() => (format = id)}>
                {FORMAT_LABELS[id] || $t("scrape.format_text")}
              </button>
            {/each}
          </div>
        </div>
        <label class="dv-field sc-limit">
          {$t("scrape.max_tokens")}
          <input class="input dv-mono" type="number" min="100" step="100" bind:value={maxTokens} />
        </label>
      </div>
    {:else}
      <div class="dv-row">
        <label class="dv-field wide">
          {$t("scrape.selector")}
          <input class="input dv-mono" bind:value={selector} placeholder="h1, .price, a[href]" spellcheck="false" autocomplete="off" />
        </label>
        <label class="dv-field">
          {$t("scrape.attr")}
          <input class="input dv-mono" bind:value={attr} placeholder="href" spellcheck="false" autocomplete="off" />
        </label>
      </div>
    {/if}
  </form>

  {#if error}
    <ErrorNotice {error}>
      {#if error.code === "BLOCKED"}
        {@render browserLink()}
      {:else if error.candidates.length > 0}
        <p>{$t("scrape.candidates")}</p>
        <div class="dv-chips">
          {#each error.candidates as candidate (candidate)}
            <button type="button" class="btn btn-secondary btn-sm dv-mono" onclick={() => retry(candidate)}>{candidate}</button>
          {/each}
        </div>
      {/if}
    </ErrorNotice>
  {/if}

  {#if mode === "page" && pageResult}
    <div class="dv-panel">
      <div class="dv-bar">
        <div class="sc-head">
          <h2 class="dv-title">{pageResult.title ?? pageResult.finalUrl}</h2>
          <p class="dv-meta">{$t("devtools.tokens", { count: count(pageResult.tokens) })} · HTTP {pageResult.status}</p>
        </div>
        <CopyButton text={pageResult.content} label={$t("scrape.copy") as string} named />
      </div>
      {#if pageResult.truncated}
        <p class="dv-warn">{$t("scrape.truncated", { tokens: count(pageResult.tokens), total: count(pageResult.total) })}</p>
      {/if}
      {#if pageResult.note}
        <p class="dv-warn">{$t("scrape.no_text")}</p>
        <div>{@render browserLink()}</div>
      {/if}
      {#if pageResult.content}
        <pre class="dv-code">{pageResult.content}</pre>
      {/if}
      {#if removed}
        <p class="dv-meta">{$t("scrape.removed", { list: removed })}</p>
      {/if}
      {#if pageResult.structured.length > 0}
        <details class="dv-details">
          <summary>{$t("scrape.structured", { count: pageResult.structured.length })}</summary>
          <div class="dv-stack">
            {#each pageResult.structured as block, i (i)}
              <div class="sc-block">
                <p class="dv-meta dv-mono">{block.kind}{block.id ? ` #${block.id}` : ""}{block.truncated ? " …" : ""}</p>
                <pre class="dv-code">{block.preview ?? JSON.stringify(block.data, null, 2)}</pre>
              </div>
            {/each}
          </div>
        </details>
      {/if}
    </div>
  {:else if mode === "select" && selection}
    <div class="dv-panel">
      <div class="dv-bar">
        <h2 class="dv-title">{$t("scrape.matches", { shown: count(selection.matches.length), total: count(selection.total) })}</h2>
        <CopyButton text={selection.matches.join("\n")} label={$t("scrape.copy") as string} named />
      </div>
      <ol class="sc-matches">
        {#each selection.matches as match, i (i)}
          <li class="dv-mono">{match}</li>
        {/each}
      </ol>
    </div>
  {:else if !error && !busy}
    <p class="dv-empty">{mode === "page" ? $t("scrape.empty_page") : $t("scrape.empty_select")}</p>
  {/if}
</DevShell>

<style>
  .dv-row > .sc-limit {
    flex: 0 1 140px;
  }

  .sc-head {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .sc-block {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .sc-matches {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0 0 0 var(--space-6);
    color: var(--text-dim);
  }

  .sc-matches li {
    padding: var(--space-2) 0;
    color: var(--text);
    overflow-wrap: anywhere;
    user-select: text;
  }

  .sc-matches li + li {
    border-top: var(--hairline) solid var(--content-border);
  }
</style>
