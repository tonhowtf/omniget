<script lang="ts">
  /** Where the person can watch: subscriptions, accepted ways, confirmed-only. */
  import { t } from "$lib/i18n";
  import {
    ACCESS_KINDS,
    accessKey,
    setPrefs,
    splitList,
    type AccessKind,
    type AccessPrefs,
  } from "$lib/stores/assist-reading-store.svelte";

  let { botId, prefs }: { botId: string; prefs: AccessPrefs } = $props();

  let editing = $state(false);
  let subs = $state("");
  let kinds = $state<AccessKind[]>([]);
  let onlyConfirmed = $state(false);
  let days = $state(7);
  let busy = $state(false);
  let errorKey = $state<string | null>(null);

  function start() {
    subs = prefs.subscriptions.join(", ");
    kinds = [...prefs.access_kinds];
    onlyConfirmed = prefs.only_confirmed;
    days = prefs.freshness_days;
    errorKey = null;
    editing = true;
  }

  function toggle(k: AccessKind, on: boolean) {
    kinds = on ? [...kinds.filter((x) => x !== k), k] : kinds.filter((x) => x !== k);
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    if (kinds.length === 0) {
      errorKey = "assist.reading.prefs_need_kind";
      return;
    }
    busy = true;
    const out = await setPrefs(botId, {
      subscriptions: splitList(subs),
      access_kinds: kinds,
      only_confirmed: onlyConfirmed,
      freshness_days: days,
    });
    busy = false;
    if (out.ok) editing = false;
    else errorKey = out.errorKey;
  }
</script>

<section class="prefs" aria-labelledby="reading-prefs-{botId}">
  <h3 id="reading-prefs-{botId}" class="section-title">{$t("assist.reading.prefs_title")}</h3>
  {#if !editing}
    <dl class="facts">
      <dt>{$t("assist.reading.prefs_region")}</dt>
      <dd>{prefs.region}</dd>
      <dt>{$t("assist.reading.prefs_subscriptions")}</dt>
      <dd>{prefs.subscriptions.length ? prefs.subscriptions.join(", ") : $t("assist.reading.prefs_none")}</dd>
      <dt>{$t("assist.reading.prefs_kinds")}</dt>
      <dd>{prefs.access_kinds.map((k) => $t(accessKey(k))).join(", ")}</dd>
      <dt>{$t("assist.reading.prefs_only_confirmed")}</dt>
      <dd>{prefs.only_confirmed ? $t("assist.reading.yes") : $t("assist.reading.no_probable_ok")}</dd>
    </dl>
    <button type="button" class="link" onclick={start}>{$t("assist.reading.edit")}</button>
  {:else}
    <form class="form" onsubmit={save}>
      <label>
        <span>{$t("assist.reading.prefs_subscriptions")}</span>
        <input type="text" bind:value={subs} placeholder={$t("assist.reading.prefs_subscriptions_hint")} />
      </label>
      <fieldset>
        <legend>{$t("assist.reading.prefs_kinds")}</legend>
        {#each ACCESS_KINDS as k (k)}
          <label class="check">
            <input type="checkbox" checked={kinds.includes(k)} onchange={(e) => toggle(k, (e.currentTarget as HTMLInputElement).checked)} />
            <span>{$t(accessKey(k))}</span>
          </label>
        {/each}
      </fieldset>
      <label class="check">
        <input type="checkbox" bind:checked={onlyConfirmed} />
        <span>{$t("assist.reading.prefs_only_confirmed_long")}</span>
      </label>
      <details>
        <summary>{$t("assist.reading.advanced")}</summary>
        <label>
          <span>{$t("assist.reading.prefs_days")}</span>
          <input type="number" min="1" max="90" bind:value={days} />
        </label>
      </details>
      {#if errorKey}<p class="notice error" role="alert">{$t(errorKey)}</p>{/if}
      <div class="actions">
        <button type="submit" class="button" disabled={busy}>{$t("assist.reading.save")}</button>
        <button type="button" class="button" onclick={() => (editing = false)}>{$t("assist.reading.cancel")}</button>
      </div>
    </form>
  {/if}
</section>

<style>
  .prefs { display: grid; gap: 8px; }
  .section-title { margin: 0; font-size: 15px; font-weight: 600; color: var(--text); }
  .facts { display: grid; grid-template-columns: max-content 1fr; gap: 4px 12px; margin: 0; font-size: 13px; }
  .facts dt { color: var(--text-muted); }
  .facts dd { margin: 0; overflow-wrap: anywhere; }
  .form { display: grid; gap: 10px; max-width: 520px; }
  .form label { display: grid; gap: 4px; font-size: 13px; }
  .form label.check { display: flex; gap: 8px; align-items: center; }
  fieldset { border: 1px solid var(--separator); border-radius: var(--radius-sm); padding: 8px 12px; display: flex; flex-wrap: wrap; gap: 12px; }
  legend { font-size: 13px; color: var(--text-muted); padding: 0 4px; }
  .actions { display: flex; gap: 8px; flex-wrap: wrap; }
  .link { background: none; border: 0; padding: 4px 0; color: var(--accent-text); font-size: 13px; cursor: pointer; border-radius: var(--radius-sm); justify-self: start; }
  .link:focus-visible, .button:focus-visible, input:focus-visible, summary:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
  summary { cursor: pointer; font-size: 13px; color: var(--text-muted); }
  @media (max-width: 560px) { .facts { grid-template-columns: 1fr; } }
</style>
