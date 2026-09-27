<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { errorText, formatDay } from "$lib/washmybelly/format";
  import { hueOf, initial } from "$lib/washmybelly/startup";
  import { appName, mergeTargets, orderServices, serviceKey, type TccApp, type TccList, type TccService } from "$lib/washmybelly/tcc";
  import EmptyState from "./ui/EmptyState.svelte";
  import Icon from "./ui/Icon.svelte";
  import PlanReview from "./ui/PlanReview.svelte";

  /// Permissions tab (macOS): who may use the camera, the microphone, the
  /// screen, the whole disk... read from TCC, grouped by service. "Reset"
  /// forgets one app's answer through a one-item plan; the app asks again
  /// the next time it needs it. Ported from "Item Action List" (sean0205,
  /// 21st.dev): icon, title and description with one trailing action per
  /// row, inside the app's grouped inset list.
  let list = $state<TccList | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let allowedOnly = $state(true);
  let reviewIds = $state<string[]>([]);
  let reviewOpen = $state(false);
  let reviewTitle = $state<string | undefined>(undefined);
  let fdaError = $state<string | null>(null);

  let services = $derived(list?.available ? orderServices(list.services, allowedOnly) : []);

  async function load() {
    loading = true;
    error = null;
    try {
      list = await api.ui<TccList>("privacy.tcc_list");
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load();
  });

  async function openFda() {
    fdaError = null;
    try {
      await api.ui("security.open_fda_settings");
    } catch (e) {
      fdaError = errorText(e);
    }
  }

  function label(s: TccService): string {
    const k = serviceKey(s);
    const v = $t(k) as string;
    return v === k ? s.service : v;
  }

  function reset(s: TccService, a: TccApp) {
    if (!a.item_id) return;
    reviewIds = [a.item_id];
    reviewTitle = $t("washmybelly.privacy.tcc.review", { name: appName(a.client), service: label(s) }) as string;
    reviewOpen = true;
  }
</script>

{#if loading && !list}
  <div class="tc-loading" aria-busy="true">
    {#each [0, 1, 2] as i (i)}<span class="skeleton" style:height="48px"></span>{/each}
  </div>
{:else if error && !list}
  <EmptyState art="worried" title={$t("washmybelly.error_title") as string} body={error}>
    {#snippet action()}
      <button class="btn btn-primary" type="button" onclick={() => void load()}>{$t("washmybelly.retry")}</button>
    {/snippet}
  </EmptyState>
{:else if list && !list.available}
  {#if list.reason === "full_disk_access"}
    <EmptyState art="searching" title={$t("washmybelly.privacy.tcc.fda_title") as string} body={$t("washmybelly.privacy.tcc.fda_body") as string}>
      {#snippet action()}
        <div class="tc-fda">
          <button class="btn btn-primary" type="button" onclick={() => void openFda()}>{$t("washmybelly.privacy.tcc.fda_open")}</button>
          <button class="btn btn-secondary" type="button" onclick={() => void load()}>{$t("washmybelly.privacy.tcc.fda_check")}</button>
        </div>
        {#if fdaError}<p class="tc-err">{fdaError}</p>{/if}
      {/snippet}
    </EmptyState>
  {:else}
    <EmptyState art="worried" title={$t("washmybelly.privacy.tcc.unavailable") as string} body={$t(`washmybelly.privacy.tcc.reason.${list.reason}`) as string} />
  {/if}
{:else}
  <div class="tc-bar">
    <p class="tc-intro">{$t("washmybelly.privacy.tcc.intro")}</p>
    <label class="tc-filter">
      <button type="button" class="toggle" class:on={allowedOnly} role="switch" aria-checked={allowedOnly} aria-label={$t("washmybelly.privacy.tcc.allowed_only") as string} onclick={() => (allowedOnly = !allowedOnly)}>
        <span class="toggle-knob"></span>
      </button>
      <span>{$t("washmybelly.privacy.tcc.allowed_only")}</span>
    </label>
    <button class="btn btn-ghost btn-icon" type="button" disabled={loading} title={$t("washmybelly.privacy.refresh") as string} aria-label={$t("washmybelly.privacy.refresh") as string} onclick={() => void load()}>
      <Icon name="refresh" size={13} />
    </button>
  </div>

  {#if services.length === 0}
    <EmptyState art="happy" title={$t("washmybelly.privacy.tcc.empty_title") as string} body={$t("washmybelly.privacy.tcc.empty_body") as string} />
  {/if}

  {#each services as s (s.service)}
    <span class="group-label">
      {label(s)}
      <span class="tc-count">{$t("washmybelly.privacy.tcc.count", { count: s.allowed })}</span>
    </span>
    <div class="group" role="list">
      {#each mergeTargets(s.apps) as a (a.db + a.client)}
        <div class="group-row tc-row" role="listitem">
          <span class="tc-tile" style:--h={hueOf(appName(a.client))} aria-hidden="true">{initial(appName(a.client))}</span>
          <div class="group-row-content">
            <span class="group-row-title">{appName(a.client)}</span>
            <span class="group-row-sub tc-sub">
              <span class="tc-id" title={a.client}>{a.client}</span>
              {#if a.targets.length}<span>{$t("washmybelly.privacy.tcc.controls", { apps: a.targets.map(appName).join(", ") })}</span>{/if}
              {#if a.last_modified}<span>{$t("washmybelly.privacy.tcc.since", { date: formatDay(a.last_modified) })}</span>{/if}
            </span>
          </div>
          <div class="group-row-trailing tc-trail">
            {#if a.db === "system"}<span class="tag" title={$t("washmybelly.privacy.tcc.system_hint") as string}>{$t("washmybelly.privacy.tcc.system")}</span>{/if}
            <span class="tag" class:tag-success={a.auth === "allowed"} class:tag-info={a.auth === "limited"}>{$t(`washmybelly.privacy.tcc.auth.${a.auth}`)}</span>
            {#if a.can_reset && a.item_id}
              <button class="btn btn-sm btn-secondary" type="button" disabled={reviewOpen} onclick={() => reset(s, a)}>{$t("washmybelly.privacy.tcc.reset")}</button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/each}
  <p class="group-footer">{$t("washmybelly.privacy.tcc.footer")}</p>
{/if}

<PlanReview kind="privacy" itemIds={reviewIds} bind:open={reviewOpen} title={reviewTitle} onDone={() => void load()} />

<style>
  .tc-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }
  .tc-intro {
    flex: 1 1 260px;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .tc-filter {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
    cursor: pointer;
  }
  .tc-count {
    font-weight: 400;
    margin-left: var(--space-1);
  }
  .group {
    margin-bottom: var(--space-5);
  }
  .tc-tile {
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: 7px;
    font-size: 13px;
    font-weight: 700;
    color: #fff;
    background: linear-gradient(180deg, hsl(var(--h) 70% 62%), hsl(var(--h) 62% 46%));
  }
  .tc-sub {
    display: flex;
    flex-wrap: wrap;
    gap: 0 var(--space-2);
  }
  .tc-id {
    font-family: var(--font-mono);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 280px;
  }
  .tc-trail {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }
  .tc-fda {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-2);
  }
  .tc-err {
    margin: var(--space-2) 0 0;
    font-size: var(--text-xs);
    color: var(--danger);
  }
  .tc-loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .tc-loading .skeleton {
    display: block;
    border-radius: var(--radius-md);
  }
</style>
