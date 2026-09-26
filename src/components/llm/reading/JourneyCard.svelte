<script lang="ts">
  /** One book: status, progress per edition, today's wish, rounds and films. */
  import { t } from "$lib/i18n";
  import RoundItemCard from "./RoundItemCard.svelte";
  import {
    JOURNEY_STATUSES,
    POSITION_KINDS,
    deleteJourney,
    noteContext,
    positionKey,
    progressValue,
    recordProgress,
    splitList,
    statusKey,
    updateJourney,
    viewingKey,
    type JourneyDetail,
    type JourneyStatus,
    type PositionKind,
  } from "$lib/stores/assist-reading-store.svelte";

  let { botId, detail }: { botId: string; detail: JourneyDetail } = $props();

  let j = $derived(detail.journey);
  const date = (ms: number) => new Date(ms).toLocaleDateString();
  const dateTime = (ms: number) => new Date(ms).toLocaleString();

  let kind = $state<PositionKind>("chapter");
  let value = $state("");
  let edition = $state("");
  let progressNote = $state<string | null>(null);
  let wish = $state("");
  let editingSpoilers = $state(false);
  let spoilers = $state("");
  let confirmingDelete = $state(false);
  let showHistory = $state(false);
  let busy = $state(false);
  let errorKey = $state<string | null>(null);
  let detailText = $state("");

  function fail(out: { ok: false; errorKey: string; detail: string }) {
    errorKey = out.errorKey;
    detailText = out.detail;
  }

  async function saveProgress(e: SubmitEvent) {
    e.preventDefault();
    if (busy || !value.trim()) return;
    busy = true;
    errorKey = null;
    const out = await recordProgress(botId, j.id, kind, value.trim(), edition);
    busy = false;
    if (out.ok) {
      progressNote = out.value.not_comparable ? "assist.reading.progress_not_comparable" : null;
      value = "";
    } else fail(out);
  }

  async function setStatus(s: JourneyStatus) {
    const out = await updateJourney(botId, j.id, { status: s });
    if (!out.ok) fail(out);
  }

  async function saveWish(e: SubmitEvent) {
    e.preventDefault();
    if (!wish.trim()) return;
    const out = await noteContext(botId, j.id, wish.trim());
    if (out.ok) wish = "";
    else fail(out);
  }

  async function saveSpoilers(e: SubmitEvent) {
    e.preventDefault();
    const out = await updateJourney(botId, j.id, { spoiler_terms: splitList(spoilers) });
    if (out.ok) {
      editingSpoilers = false;
      spoilers = "";
    } else fail(out);
  }

  async function remove() {
    const out = await deleteJourney(botId, j.id);
    if (!out.ok) fail(out);
  }
</script>

<article class="journey" aria-labelledby="journey-{j.id}">
  <header class="head">
    <div>
      <h4 id="journey-{j.id}" class="title">{j.title}</h4>
      <p class="dim">
        {[j.author, j.edition ? $t("assist.reading.edition_label", { edition: j.edition }) : null, j.language].filter(Boolean).join(" · ")}
        {#if j.external_id?.startsWith("study:")}· {$t("assist.reading.linked_reader")}{/if}
      </p>
    </div>
    <label class="status">
      <span class="sr">{$t("assist.reading.journey_status")}</span>
      <select value={j.status} onchange={(e) => setStatus((e.currentTarget as HTMLSelectElement).value as JourneyStatus)}>
        {#each JOURNEY_STATUSES as s (s)}<option value={s}>{$t(`assist.reading.journey_${s}`)}</option>{/each}
      </select>
    </label>
  </header>

  <section class="block">
    <h5 class="label">{$t("assist.reading.progress_title")}</h5>
    {#if detail.progress}
      <p class="value">
        {$t(positionKey(detail.progress.position_kind))} {progressValue(detail.progress)}
        {#if detail.progress.edition}<span class="dim"> · {detail.progress.edition}</span>{/if}
        <span class="dim"> · {date(detail.progress.recorded_ms)} · {$t(`assist.reading.origin_${detail.progress.origin}`)}</span>
      </p>
    {:else}
      <p class="dim">{$t("assist.reading.progress_unknown")}</p>
    {/if}
    <form class="inline" onsubmit={saveProgress}>
      <label>
        <span class="sr">{$t("assist.reading.position_kind")}</span>
        <select bind:value={kind}>
          {#each POSITION_KINDS as k (k)}<option value={k}>{$t(positionKey(k))}</option>{/each}
        </select>
      </label>
      <label>
        <span class="sr">{$t("assist.reading.position_value")}</span>
        <input type="text" bind:value={value} placeholder={$t("assist.reading.position_value")} size="8" />
      </label>
      {#if kind === "page" || kind === "locator"}
        <label>
          <span class="sr">{$t("assist.reading.book_edition")}</span>
          <input type="text" bind:value={edition} placeholder={j.edition ?? $t("assist.reading.book_edition")} size="14" />
        </label>
      {/if}
      <button type="submit" class="button small" disabled={busy || !value.trim()}>{$t("assist.reading.update_progress")}</button>
    </form>
    {#if progressNote}<p class="notice" role="status">{$t(progressNote)}</p>{/if}
    {#if detail.progress_history.length > 1}
      <button type="button" class="link" aria-expanded={showHistory} onclick={() => (showHistory = !showHistory)}>{$t("assist.reading.progress_history")}</button>
      {#if showHistory}
        <ul class="history">
          {#each detail.progress_history as p (p.id)}
            <li>{$t(positionKey(p.position_kind))} {progressValue(p)}{p.edition ? ` · ${p.edition}` : ""} <span class="dim">· {dateTime(p.recorded_ms)} · {$t(`assist.reading.origin_${p.origin}`)}</span></li>
          {/each}
        </ul>
      {/if}
    {/if}
  </section>

  <section class="block">
    <h5 class="label">{$t("assist.reading.today_title")}</h5>
    {#each detail.contexts as c (c.id)}
      <p class="value">“{c.text}” <span class="dim">· {$t("assist.reading.expires", { when: dateTime(c.expires_ms) })}</span></p>
    {/each}
    <form class="inline" onsubmit={saveWish}>
      <label class="grow">
        <span class="sr">{$t("assist.reading.today_title")}</span>
        <input type="text" bind:value={wish} placeholder={$t("assist.reading.today_hint")} />
      </label>
      <button type="submit" class="button small" disabled={!wish.trim()}>{$t("assist.reading.save")}</button>
    </form>
  </section>

  <section class="block">
    <h5 class="label">{$t("assist.reading.spoilers_title")}</h5>
    <p class="dim">{$t("assist.reading.spoilers_count", { count: j.spoiler_terms.length })}</p>
    {#if editingSpoilers}
      <form class="inline" onsubmit={saveSpoilers}>
        <label class="grow">
          <span class="sr">{$t("assist.reading.spoiler_terms")}</span>
          <input type="text" bind:value={spoilers} placeholder={$t("assist.reading.spoiler_terms_hint")} />
        </label>
        <button type="submit" class="button small">{$t("assist.reading.save")}</button>
        <button type="button" class="button small" onclick={() => (editingSpoilers = false)}>{$t("assist.reading.cancel")}</button>
      </form>
      <p class="dim">{$t("assist.reading.spoilers_replace_note")}</p>
    {:else}
      <button type="button" class="link" onclick={() => { spoilers = ""; editingSpoilers = true; }}>{$t("assist.reading.spoilers_edit")}</button>
    {/if}
  </section>

  {#if errorKey}
    <p class="notice error" role="alert">{$t(errorKey)}</p>
    {#if detailText}<details><summary>{$t("assist.reading.advanced")}</summary><p class="dim">{detailText}</p></details>{/if}
  {/if}

  <section class="block">
    <h5 class="label">{$t("assist.reading.rounds_title")}</h5>
    {#if detail.rounds.length === 0}
      <p class="dim">{$t("assist.reading.rounds_empty")}</p>
    {/if}
    {#each detail.rounds as r (r.round.id)}
      <div class="round">
        <p class="dim">
          {$t("assist.reading.round_of", { when: dateTime(r.round.created_ms) })}
          {#if r.round.context_text}· “{r.round.context_text}”{/if}
          {#if r.round.only_confirmed}· {$t("assist.reading.only_confirmed_short")}{/if}
        </p>
        {#if r.round.reason}<p class="value">{r.round.reason}</p>{/if}
        {#if r.round.shortfall_reason}<p class="notice" role="note">{$t("assist.reading.shortfall", { reason: r.round.shortfall_reason })}</p>{/if}
        <ul class="items">
          {#each r.items as it (it.item.id)}
            <RoundItemCard {botId} journeyId={j.id} view={it} />
          {/each}
        </ul>
        {#if r.round.look_for.length > 0}
          <div class="look-for">
            <p class="label">{$t("assist.reading.look_for")}</p>
            <ul>
              {#each r.round.look_for as lf (lf.movie_id)}
                <li>{lf.title} ({lf.year}){lf.note ? ` — ${lf.note}` : ""} <span class="dim">· {$t(statusKey(lf.status))}</span></li>
              {/each}
            </ul>
          </div>
        {/if}
        {#if r.round.skill_name}
          <details>
            <summary>{$t("assist.reading.advanced")}</summary>
            <p class="dim">{$t("assist.reading.skill_used", { name: r.round.skill_name, hash: (r.round.skill_hash ?? "—").slice(0, 12) })}</p>
          </details>
        {/if}
      </div>
    {/each}
  </section>

  {#if detail.films.length > 0}
    <section class="block">
      <h5 class="label">{$t("assist.reading.films_title")}</h5>
      <ul class="films">
        {#each detail.films as f (f.movie.id)}
          <li>{f.movie.title} ({f.movie.year}) <span class="dim">· {$t(viewingKey(f.state))}</span></li>
        {/each}
      </ul>
    </section>
  {/if}

  <footer class="foot">
    {#if confirmingDelete}
      <span class="confirm">
        {$t("assist.reading.delete_confirm")}
        <button type="button" class="button small danger" onclick={remove}>{$t("assist.reading.delete")}</button>
        <button type="button" class="button small" onclick={() => (confirmingDelete = false)}>{$t("assist.reading.cancel")}</button>
      </span>
    {:else}
      <button type="button" class="link danger" onclick={() => (confirmingDelete = true)}>{$t("assist.reading.delete_journey")}</button>
    {/if}
  </footer>
</article>

<style>
  .journey { display: grid; gap: 14px; padding: 14px; border: 1px solid var(--separator); border-radius: var(--radius-sm); }
  .head { display: flex; flex-wrap: wrap; gap: 8px; justify-content: space-between; align-items: start; }
  .title { margin: 0; font-size: 16px; font-weight: 600; color: var(--text); overflow-wrap: anywhere; }
  .block { display: grid; gap: 6px; }
  .label { margin: 0; font-size: 12px; font-weight: 600; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em; }
  .value { margin: 0; font-size: 14px; color: var(--text); overflow-wrap: anywhere; }
  .dim { margin: 0; color: var(--text-muted); font-size: 12px; }
  .inline { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  .grow { flex: 1 1 220px; display: grid; }
  .history, .films { margin: 0; padding-left: 18px; display: grid; gap: 2px; font-size: 13px; }
  .items { list-style: none; margin: 0; padding: 0; display: grid; gap: 10px; }
  .round { display: grid; gap: 6px; padding-top: 8px; border-top: 1px solid var(--separator); }
  .look-for ul { margin: 4px 0 0; padding-left: 18px; font-size: 13px; }
  .foot { display: flex; justify-content: flex-end; }
  .confirm { display: inline-flex; flex-wrap: wrap; gap: 6px; align-items: center; font-size: 13px; }
  .link { background: none; border: 0; padding: 4px 0; color: var(--accent-text); font-size: 13px; cursor: pointer; border-radius: var(--radius-sm); justify-self: start; }
  .danger { color: var(--danger, #c0392b); }
  .small { min-height: 32px; padding: 4px 10px; font-size: 13px; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
  .link:focus-visible, .button:focus-visible, input:focus-visible, select:focus-visible, summary:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
  summary { cursor: pointer; font-size: 12px; color: var(--text-muted); }
</style>
