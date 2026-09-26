<script lang="ts">
  /**
   * One recommended film: role, connection, mood and pace, where to watch in
   * Brazil with availability and PT subtitles shown separately (date and
   * source each), "Por que esta indicação?", and "watched / dropped / not
   * now" with an optional short reaction.
   */
  import { t } from "$lib/i18n";
  import {
    accessKey,
    forgetReaction,
    moodKey,
    recordViewing,
    roleKey,
    safeHref,
    statusKey,
    viewingKey,
    type ItemView,
  } from "$lib/stores/assist-reading-store.svelte";

  let { botId, journeyId, view }: { botId: string; journeyId: string; view: ItemView } = $props();

  const date = (ms: number) => new Date(ms).toLocaleDateString();
  let it = $derived(view.item);
  let a = $derived(view.availability);
  let s = $derived(view.subtitle);
  let changed = $derived(view.current.status !== view.recorded_status);

  let why = $state(false);
  let marking = $state<null | "watched" | "abandoned" | "declined">(null);
  let reaction = $state("");
  let busy = $state(false);
  let errorKey = $state<string | null>(null);

  async function mark(e?: SubmitEvent) {
    e?.preventDefault();
    if (!marking || busy) return;
    busy = true;
    const out = await recordViewing(botId, journeyId, it.movie_id, marking, reaction);
    busy = false;
    if (out.ok) {
      marking = null;
      reaction = "";
      errorKey = null;
    } else errorKey = out.errorKey; // the reaction stays in the box
  }

  async function forget(eventId: string) {
    const out = await forgetReaction(botId, eventId);
    if (!out.ok) errorKey = out.errorKey;
  }
</script>

<li class="item">
  <div class="top">
    <p class="name">
      <strong>{view.movie.title}</strong> ({view.movie.year}){#if view.movie.director} · {view.movie.director}{/if}
    </p>
    <span class="badge role">{$t(roleKey(it.role))}</span>
    {#if it.resumes_item_id}<span class="badge">{$t("assist.reading.resumed")}</span>{/if}
    {#if view.state && view.state !== "recommended"}<span class="badge">{$t(viewingKey(view.state))}</span>{/if}
  </div>
  <p class="text">{it.connection}</p>
  <p class="text">{it.why}</p>
  <p class="meta">
    {it.moods.map((m) => $t(moodKey(m))).join(", ")} · {$t(`assist.reading.pace_${it.pace}`)}
  </p>

  <dl class="access">
    <dt>{$t("assist.reading.where")}</dt>
    <dd>
      {#if a}
        {a.platform} · {$t(accessKey(a.access_kind))} · {a.region}
        {#if safeHref(a.url)}· <a href={safeHref(a.url)} target="_blank" rel="noopener noreferrer">{$t("assist.reading.open_page")}</a>{/if}
      {:else}—{/if}
    </dd>
    <dt>{$t("assist.reading.availability")}</dt>
    <dd>
      {#if a}
        {$t(statusKey(a.status))} <span class="dim">· {date(a.checked_ms)} · {$t(`assist.reading.source_${a.source_kind}`)}</span>
      {:else}—{/if}
    </dd>
    <dt>{$t("assist.reading.subtitles_pt")}</dt>
    <dd>
      {#if s}
        {$t(statusKey(s.status))}{#if s.login_required} · {$t("assist.reading.login_wall")}{/if}
        <span class="dim">· {date(s.checked_ms)} · {$t(`assist.reading.source_${s.source_kind}`)}</span>
        {#if safeHref(s.url) && s.url !== a?.url}· <a href={safeHref(s.url)} target="_blank" rel="noopener noreferrer">{$t("assist.reading.open_page")}</a>{/if}
      {:else}
        {$t("assist.reading.not_checked")}
      {/if}
    </dd>
    <dt>{$t("assist.reading.final_status")}</dt>
    <dd>
      <span class="badge status-{view.recorded_status}">{$t(statusKey(view.recorded_status))}</span>
      {#if it.missing.length}<span class="dim"> · {it.missing.join("; ")}</span>{/if}
    </dd>
  </dl>
  {#if changed}
    <p class="notice" role="status">
      {$t("assist.reading.status_changed", { status: $t(statusKey(view.current.status)) })}
      {#if view.current.reasons.length}<span class="dim"> {view.current.reasons.join("; ")}</span>{/if}
    </p>
  {/if}

  {#if it.questions.length}
    <ul class="questions">{#each it.questions as q, i (i)}<li>{q}</li>{/each}</ul>
  {/if}

  <button type="button" class="link" aria-expanded={why} onclick={() => (why = !why)}>{$t("assist.reading.why")}</button>
  {#if why}
    <dl class="why">
      <dt>{$t("assist.reading.why_role")}</dt>
      <dd>{$t(`assist.reading.role_${it.role}_explain`)}</dd>
      {#each view.why_evidence as ev (ev.event_id)}
        <dt>{$t("assist.reading.why_you_said")}</dt>
        <dd>
          “{ev.reaction ?? ev.reasons.join(", ")}” — {ev.movie.title} ({$t(viewingKey(ev.kind))}, {date(ev.created_ms)})
          <button type="button" class="link small-link" onclick={() => forget(ev.event_id)}>{$t("assist.reading.forget_reaction")}</button>
        </dd>
      {/each}
      {#if a?.quote}
        <dt>{$t("assist.reading.why_availability_source")}</dt>
        <dd>“{a.quote}”</dd>
      {/if}
      {#if s?.quote}
        <dt>{$t("assist.reading.why_subtitle_source")}</dt>
        <dd>“{s.quote}”</dd>
      {/if}
      {#if s?.note || a?.note}
        <dt>{$t("assist.reading.why_notes")}</dt>
        <dd>{[a?.note, s?.note].filter(Boolean).join(" · ")}</dd>
      {/if}
    </dl>
  {/if}

  {#if marking}
    <form class="mark" onsubmit={mark}>
      <label class="grow">
        <span>{$t("assist.reading.reaction_optional")}</span>
        <input type="text" bind:value={reaction} maxlength="280" />
      </label>
      <button type="submit" class="button small" disabled={busy}>{$t(viewingKey(marking))}</button>
      <button type="button" class="button small" onclick={() => (marking = null)}>{$t("assist.reading.cancel")}</button>
    </form>
  {:else}
    <div class="actions" role="group" aria-label={$t("assist.reading.mark_as")}>
      <button type="button" class="button small" onclick={() => (marking = "watched")}>{$t("assist.reading.mark_watched")}</button>
      <button type="button" class="button small" onclick={() => (marking = "abandoned")}>{$t("assist.reading.mark_abandoned")}</button>
      <button type="button" class="button small" onclick={() => (marking = "declined")}>{$t("assist.reading.mark_declined")}</button>
    </div>
  {/if}
  {#if errorKey}<p class="notice error" role="alert">{$t(errorKey)}</p>{/if}
</li>

<style>
  .item { display: grid; gap: 6px; padding: 10px 12px; border: 1px solid var(--separator); border-radius: var(--radius-sm); }
  .top { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  .name { margin: 0; font-size: 14px; color: var(--text); overflow-wrap: anywhere; }
  .text { margin: 0; font-size: 14px; line-height: 1.5; color: var(--text); overflow-wrap: anywhere; }
  .meta, .dim { margin: 0; color: var(--text-muted); font-size: 12px; }
  .badge { font-size: 11px; padding: 2px 8px; border-radius: 999px; border: 1px solid var(--separator); color: var(--text-muted); }
  .badge.role { color: var(--accent-text); }
  .status-confirmed { color: var(--success, #2e7d32); }
  .status-probable { color: var(--warning, #b26a00); }
  .access, .why { display: grid; grid-template-columns: max-content 1fr; gap: 4px 12px; margin: 0; font-size: 13px; }
  .why { padding: 8px 12px; border-left: 2px solid var(--separator); }
  .access dt, .why dt { color: var(--text-muted); }
  .access dd, .why dd { margin: 0; overflow-wrap: anywhere; }
  .questions { margin: 0; padding-left: 18px; font-size: 13px; color: var(--text); }
  .actions, .mark { display: flex; flex-wrap: wrap; gap: 6px; align-items: end; }
  .grow { flex: 1 1 220px; display: grid; gap: 4px; font-size: 12px; }
  .link { background: none; border: 0; padding: 4px 0; color: var(--accent-text); font-size: 13px; cursor: pointer; border-radius: var(--radius-sm); justify-self: start; }
  .small-link { font-size: 12px; margin-left: 6px; }
  .small { min-height: 32px; padding: 4px 10px; font-size: 13px; }
  a { color: var(--accent-text); }
  .link:focus-visible, .button:focus-visible, input:focus-visible, a:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
  @media (max-width: 560px) { .access, .why { grid-template-columns: 1fr; } }
</style>
