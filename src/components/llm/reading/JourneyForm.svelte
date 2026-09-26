<script lang="ts">
  /** Start a reading journey, typed by hand. */
  import { t } from "$lib/i18n";
  import {
    POSITION_KINDS,
    positionKey,
    recordProgress,
    splitList,
    startJourney,
    type PositionKind,
  } from "$lib/stores/assist-reading-store.svelte";

  let { botId, onDone }: { botId: string; onDone: () => void } = $props();

  let title = $state("");
  let author = $state("");
  let edition = $state("");
  let language = $state("");
  let spoilers = $state("");
  let kind = $state<PositionKind>("chapter");
  let value = $state("");
  let busy = $state(false);
  let errorKey = $state<string | null>(null);
  let detail = $state("");


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
      spoiler_terms: splitList(spoilers),
    });
    if (!out.ok) {
      busy = false;
      errorKey = out.errorKey;
      detail = out.detail;
      return;
    }
    if (value.trim()) {
      const p = await recordProgress(botId, out.value.id, kind, value.trim(), undefined, "manual");
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
  .small { min-height: 32px; padding: 4px 10px; font-size: 13px; }
  .button:focus-visible, input:focus-visible, select:focus-visible, summary:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
  @media (max-width: 560px) { .two { grid-template-columns: 1fr; } }
</style>
