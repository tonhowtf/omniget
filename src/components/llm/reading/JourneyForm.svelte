<script lang="ts">
  /**
   * Start a reading journey: typed by hand, or picked from the Study reader
   * (then the reader's percentage becomes the first progress, marked
   * "from the reader").
   */
  import { t } from "$lib/i18n";
  import {
    POSITION_KINDS,
    positionKey,
    readerBooks,
    readerPercent,
    recordProgress,
    splitList,
    startJourney,
    type PositionKind,
    type ReaderBook,
  } from "$lib/stores/assist-reading-store.svelte";

  let { botId, onDone }: { botId: string; onDone: () => void } = $props();

  let title = $state("");
  let author = $state("");
  let edition = $state("");
  let language = $state("");
  let spoilers = $state("");
  let kind = $state<PositionKind>("chapter");
  let value = $state("");
  let externalId = $state<string | null>(null);
  let fromReader = $state(false);
  let busy = $state(false);
  let errorKey = $state<string | null>(null);
  let detail = $state("");

  let readerOpen = $state(false);
  let search = $state("");
  let books = $state<ReaderBook[] | null>(null);
  let readerError = $state<string | null>(null);

  async function searchReader(e?: SubmitEvent) {
    e?.preventDefault();
    readerError = null;
    const out = await readerBooks(search);
    if (out.ok) books = out.value;
    else {
      books = null;
      readerError = out.errorKey;
    }
  }

  function pick(b: ReaderBook) {
    title = b.title;
    author = b.author ?? "";
    externalId = `study:book:${b.id}`;
    kind = "percent";
    value = String(readerPercent(b.reading_pct));
    fromReader = true;
    readerOpen = false;
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (busy || !title.trim()) return;
    busy = true;
    errorKey = null;
    const out = await startJourney(botId, {
      title: title.trim(),
      author: author.trim() || undefined,
      edition: edition.trim() || undefined,
      language: language.trim() || undefined,
      external_id: externalId ?? undefined,
      spoiler_terms: splitList(spoilers),
    });
    if (!out.ok) {
      busy = false;
      errorKey = out.errorKey;
      detail = out.detail;
      return;
    }
    if (value.trim()) {
      const p = await recordProgress(botId, out.value.id, kind, value.trim(), undefined, fromReader ? "reader" : "manual");
      if (!p.ok) {
        busy = false;
        errorKey = p.errorKey;
        detail = p.detail;
        return;
      }
    }
    busy = false;
    onDone();
  }
</script>

<form class="journey-form" onsubmit={submit}>
  <div class="reader">
    {#if !readerOpen}
      <button type="button" class="link" onclick={() => (readerOpen = true)}>{$t("assist.reading.from_reader")}</button>
    {:else}
      <div class="reader-search" role="search">
        <label>
          <span>{$t("assist.reading.reader_search")}</span>
          <input type="search" bind:value={search} onkeydown={(e) => e.key === "Enter" && (e.preventDefault(), searchReader())} />
        </label>
        <button type="button" class="button small" onclick={() => searchReader()}>{$t("assist.reading.search")}</button>
        <button type="button" class="button small" onclick={() => (readerOpen = false)}>{$t("assist.reading.cancel")}</button>
      </div>
      {#if readerError}<p class="notice" role="status">{$t(readerError)}</p>{/if}
      {#if books && books.length === 0}<p class="dim">{$t("assist.reading.reader_empty")}</p>{/if}
      {#if books && books.length > 0}
        <ul class="books">
          {#each books as b (b.id)}
            <li>
              <button type="button" class="book" onclick={() => pick(b)}>
                <span>{b.title}</span>
                <span class="dim">{b.author ?? ""} · {readerPercent(b.reading_pct)}%</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </div>

  <label>
    <span>{$t("assist.reading.book_title")}</span>
    <input type="text" bind:value={title} required />
  </label>
  <div class="two">
    <label>
      <span>{$t("assist.reading.book_author")}</span>
      <input type="text" bind:value={author} />
    </label>
    <label>
      <span>{$t("assist.reading.book_edition")}</span>
      <input type="text" bind:value={edition} placeholder={$t("assist.reading.book_edition_hint")} />
    </label>
  </div>
  <label>
    <span>{$t("assist.reading.book_language")}</span>
    <input type="text" bind:value={language} />
  </label>
  <fieldset>
    <legend>{$t("assist.reading.where_are_you")}</legend>
    <div class="two">
      <label>
        <span>{$t("assist.reading.position_kind")}</span>
        <select bind:value={kind}>
          {#each POSITION_KINDS as k (k)}<option value={k}>{$t(positionKey(k))}</option>{/each}
        </select>
      </label>
      <label>
        <span>{$t("assist.reading.position_value")}</span>
        <input type="text" bind:value={value} inputmode={kind === "chapter" || kind === "locator" ? "text" : "decimal"} />
      </label>
    </div>
    {#if fromReader}<p class="dim">{$t("assist.reading.from_reader_note")}</p>{/if}
  </fieldset>
  <label>
    <span>{$t("assist.reading.spoiler_terms")}</span>
    <input type="text" bind:value={spoilers} />
    <small class="dim">{$t("assist.reading.spoiler_terms_hint")}</small>
  </label>
  {#if errorKey}
    <p class="notice error" role="alert">{$t(errorKey)}</p>
    {#if detail}<details><summary>{$t("assist.reading.advanced")}</summary><p class="dim">{detail}</p></details>{/if}
  {/if}
  <div class="actions">
    <button type="submit" class="button" disabled={busy || !title.trim()}>{$t("assist.reading.start")}</button>
    <button type="button" class="button" onclick={onDone}>{$t("assist.reading.cancel")}</button>
  </div>
</form>

<style>
  .journey-form { display: grid; gap: 10px; padding: 12px; border: 1px solid var(--separator); border-radius: var(--radius-sm); max-width: 640px; }
  label { display: grid; gap: 4px; font-size: 13px; }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  fieldset { border: 1px solid var(--separator); border-radius: var(--radius-sm); padding: 8px 12px; display: grid; gap: 6px; }
  legend { font-size: 13px; color: var(--text-muted); padding: 0 4px; }
  .dim { margin: 0; color: var(--text-muted); font-size: 12px; }
  .actions { display: flex; gap: 8px; flex-wrap: wrap; }
  .reader { display: grid; gap: 6px; }
  .reader-search { display: flex; flex-wrap: wrap; gap: 8px; align-items: end; }
  .books { list-style: none; margin: 0; padding: 0; display: grid; gap: 4px; max-height: 240px; overflow: auto; }
  .book { width: 100%; display: grid; gap: 2px; text-align: left; background: none; border: 1px solid var(--separator); border-radius: var(--radius-sm); padding: 6px 10px; color: var(--text); cursor: pointer; font-size: 13px; }
  .book:hover { border-color: var(--accent-text); }
  .link { background: none; border: 0; padding: 4px 0; color: var(--accent-text); font-size: 13px; cursor: pointer; border-radius: var(--radius-sm); justify-self: start; }
  .small { min-height: 32px; padding: 4px 10px; font-size: 13px; }
  .link:focus-visible, .button:focus-visible, .book:focus-visible, input:focus-visible, select:focus-visible, summary:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
  @media (max-width: 560px) { .two { grid-template-columns: 1fr; } }
</style>
