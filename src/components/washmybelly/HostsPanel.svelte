<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { errorText, formatBytes, formatCount } from "$lib/washmybelly/format";
  import { addDomains, blockLineFlags, removeDomain, toggleList, type HostsPlan, type HostsPreview, type HostsPrefs, type HostsStatus } from "$lib/washmybelly/hosts";
  import EmptyState from "./ui/EmptyState.svelte";
  import Icon from "./ui/Icon.svelte";
  import PlanReview from "./ui/PlanReview.svelte";
  import PrivacyCode from "./PrivacyCode.svelte";

  let status = $state<HostsStatus | null>(null);
  let prefs = $state<HostsPrefs>({ lists: [], whitelist: [], extra: [] });
  let loading = $state(true);
  let error = $state<string | null>(null);
  let preview = $state<HostsPreview | null>(null);
  let previewing = $state(false);
  let planning = $state(false);
  let planned = $state<HostsPlan | null>(null);
  let note = $state<string | null>(null);
  let wlText = $state("");
  let wlBad = $state<string[]>([]);
  let exText = $state("");
  let exBad = $state<string[]>([]);
  let reviewIds = $state<string[]>([]);
  let reviewOpen = $state(false);
  let reviewTitle = $state<string | undefined>(undefined);

  let flags = $derived(preview ? blockLineFlags(preview.preview) : []);

  async function load() {
    loading = true;
    error = null;
    try {
      status = await api.ui<HostsStatus>("hosts.status");
      prefs = { ...status.prefs };
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load();
  });

  function changed() {
    preview = null;
    planned = null;
    note = null;
  }

  function setList(id: string, on: boolean) {
    prefs = { ...prefs, lists: toggleList(prefs.lists, id, on) };
    changed();
  }

  function addWhitelist() {
    const r = addDomains(prefs.whitelist, wlText);
    wlBad = r.bad;
    if (r.list.length !== prefs.whitelist.length) {
      prefs = { ...prefs, whitelist: r.list };
      changed();
    }
    if (!r.bad.length) wlText = "";
  }

  function addExtra() {
    const r = addDomains(prefs.extra, exText);
    exBad = r.bad;
    if (r.list.length !== prefs.extra.length) {
      prefs = { ...prefs, extra: r.list };
      changed();
    }
    if (!r.bad.length) exText = "";
  }

  async function doPreview() {
    previewing = true;
    note = null;
    try {
      preview = await api.ui<HostsPreview>("hosts.preview", { mode: "apply", ...prefs });
    } catch (e) {
      note = errorText(e);
    } finally {
      previewing = false;
    }
  }

  async function makePlan(mode: "apply" | "remove") {
    planning = true;
    note = null;
    planned = null;
    try {
      const p = await api.ui<HostsPlan>("hosts.plan", { mode, ...prefs });
      planned = p;
      if (mode === "apply") preview = p;
      if (!p.plan_id || !p.item_ids.length) {
        note = $t("washmybelly.privacy.hosts.unchanged") as string;
        return;
      }
      reviewIds = p.item_ids;
      reviewTitle = $t(mode === "remove" ? "washmybelly.privacy.hosts.review_remove" : "washmybelly.privacy.hosts.review_apply", { count: p.domains }) as string;
      reviewOpen = true;
    } catch (e) {
      note = errorText(e);
    } finally {
      planning = false;
    }
  }

  function afterDone() {
    planned = null;
    preview = null;
    void load();
  }
</script>

{#if loading && !status}
  <div class="hp-loading" aria-busy="true">
    {#each [0, 1, 2] as i (i)}<span class="skeleton" style:height="48px"></span>{/each}
  </div>
{:else if error && !status}
  <EmptyState art="worried" title={$t("washmybelly.error_title") as string} body={error}>
    {#snippet action()}
      <button class="btn btn-primary" type="button" onclick={() => void load()}>{$t("washmybelly.retry")}</button>
    {/snippet}
  </EmptyState>
{:else if status}
  <p class="hp-intro">{$t("washmybelly.privacy.hosts.intro")}</p>

  <span class="group-label">{$t("washmybelly.privacy.hosts.status_title")}</span>
  <div class="group" role="list">
    <div class="group-row" role="listitem">
      <div class="group-row-content">
        <span class="group-row-title">
          {#if status.block_present}
            {$t("washmybelly.privacy.hosts.blocking", { count: formatCount(status.blocked) })}
          {:else}
            {$t("washmybelly.privacy.hosts.not_blocking")}
          {/if}
        </span>
        <span class="group-row-sub hp-sub">
          <span class="hp-mono">{status.path}</span>
          {#if status.updated_at}<span>{$t("washmybelly.privacy.hosts.updated", { date: status.updated_at })}</span>{/if}
          <span>{$t("washmybelly.privacy.hosts.your_lines", { count: status.user_lines })}</span>
        </span>
      </div>
      <div class="group-row-trailing hp-trail">
        {#if !status.writable}<span class="tag" title={$t("washmybelly.privacy.hosts.needs_admin") as string}><Icon name="lock" size={10} />{$t("washmybelly.privacy.admin")}</span>{/if}
        {#if status.block_present}
          <button class="btn btn-sm btn-secondary" type="button" disabled={planning || reviewOpen} onclick={() => void makePlan("remove")}>{$t("washmybelly.privacy.hosts.remove")}</button>
        {/if}
      </div>
    </div>
  </div>
  {#if status.foreign_lines > 0}
    <p class="group-footer">{$t("washmybelly.privacy.hosts.foreign", { count: status.foreign_lines })}</p>
  {/if}

  <span class="group-label">{$t("washmybelly.privacy.hosts.lists_title")}</span>
  <div class="group" role="list">
    {#each status.lists as l (l.id)}
      {@const on = prefs.lists.includes(l.id)}
      <div class="group-row" role="listitem">
        <div class="group-row-content">
          <span class="group-row-title">{l.name}</span>
          <span class="group-row-sub">{$t(`washmybelly.privacy.hosts.list.${l.id}`)} · {$t("washmybelly.privacy.hosts.domains", { count: formatCount(l.domains) })} · {l.license}</span>
        </div>
        <div class="group-row-trailing">
          <button type="button" class="toggle" class:on role="switch" aria-checked={on} aria-label={l.name} onclick={() => setList(l.id, !on)}>
            <span class="toggle-knob"></span>
          </button>
        </div>
      </div>
    {/each}
  </div>
  <p class="group-footer">{$t("washmybelly.privacy.hosts.lists_footer")}</p>

  <span class="group-label">{$t("washmybelly.privacy.hosts.whitelist_title")}</span>
  <div class="hp-box">
    <form class="hp-add" onsubmit={(e) => { e.preventDefault(); addWhitelist(); }}>
      <input class="input" type="text" autocomplete="off" spellcheck="false" placeholder={$t("washmybelly.privacy.hosts.whitelist_placeholder") as string} bind:value={wlText} />
      <button class="btn btn-secondary" type="submit" disabled={!wlText.trim()}>{$t("washmybelly.privacy.hosts.add")}</button>
    </form>
    {#if wlBad.length}<p class="hp-bad">{$t("washmybelly.privacy.hosts.bad_domains", { list: wlBad.join(", ") })}</p>{/if}
    {#if prefs.whitelist.length}
      <div class="hp-chips">
        {#each prefs.whitelist as d (d)}
          <span class="tag hp-chip">{d}<button type="button" class="hp-x" aria-label={$t("washmybelly.privacy.hosts.remove_domain", { name: d }) as string} onclick={() => { prefs = { ...prefs, whitelist: removeDomain(prefs.whitelist, d) }; changed(); }}>×</button></span>
        {/each}
      </div>
    {:else}
      <p class="hp-dim">{$t("washmybelly.privacy.hosts.whitelist_empty")}</p>
    {/if}
  </div>

  <span class="group-label">{$t("washmybelly.privacy.hosts.extra_title")}</span>
  <div class="hp-box">
    <form class="hp-add" onsubmit={(e) => { e.preventDefault(); addExtra(); }}>
      <input class="input" type="text" autocomplete="off" spellcheck="false" placeholder={$t("washmybelly.privacy.hosts.extra_placeholder") as string} bind:value={exText} />
      <button class="btn btn-secondary" type="submit" disabled={!exText.trim()}>{$t("washmybelly.privacy.hosts.add")}</button>
    </form>
    {#if exBad.length}<p class="hp-bad">{$t("washmybelly.privacy.hosts.bad_domains", { list: exBad.join(", ") })}</p>{/if}
    {#if prefs.extra.length}
      <div class="hp-chips">
        {#each prefs.extra as d (d)}
          <span class="tag hp-chip">{d}<button type="button" class="hp-x" aria-label={$t("washmybelly.privacy.hosts.remove_domain", { name: d }) as string} onclick={() => { prefs = { ...prefs, extra: removeDomain(prefs.extra, d) }; changed(); }}>×</button></span>
        {/each}
      </div>
    {/if}
  </div>

  <div class="hp-actions">
    <button class="btn btn-secondary" type="button" disabled={previewing || planning} onclick={() => void doPreview()}>{$t("washmybelly.privacy.hosts.preview")}</button>
    <button class="btn btn-primary" type="button" disabled={planning || reviewOpen || (!prefs.lists.length && !prefs.extra.length && !status.block_present)} onclick={() => void makePlan("apply")}>
      {$t("washmybelly.privacy.hosts.apply")}
    </button>
  </div>
  {#if note}<p class="hp-note">{note}</p>{/if}

  {#if preview}
    <div class="hp-preview">
      <p class="hp-stats">
        {$t("washmybelly.privacy.hosts.preview_stats", { count: formatCount(preview.domains), added: formatCount(preview.added), removed: formatCount(preview.removed), size: formatBytes(preview.bytes) })}
        {#if preview.skipped_whitelist}· {$t("washmybelly.privacy.hosts.preview_whitelisted", { count: preview.skipped_whitelist })}{/if}
      </p>
      <pre class="hp-file" aria-label={$t("washmybelly.privacy.hosts.preview_label") as string}>{#each preview.preview as line, i (i)}<span class="ln" class:ours={flags[i]}>{line || " "}</span>{/each}{#if preview.preview_hidden}<span class="ln more">{$t("washmybelly.privacy.hosts.preview_more", { count: formatCount(preview.preview_hidden) })}</span>{/if}</pre>
    </div>
  {/if}

  {#if planned?.admin && planned.commands?.length}
    <div class="hp-cmd">
      <p class="hp-dim">{$t("washmybelly.privacy.hosts.terminal_hint")}</p>
      <PrivacyCode code={planned.commands.join("\n")} label={$t("washmybelly.privacy.hosts.terminal_label") as string} />
    </div>
  {/if}

  <p class="group-footer">{$t("washmybelly.privacy.hosts.footer")}</p>
{/if}

<PlanReview kind="hosts" itemIds={reviewIds} bind:open={reviewOpen} title={reviewTitle} onDone={afterDone} onRestored={afterDone} />

<style>
  .hp-intro {
    margin: 0 0 var(--space-4);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .group {
    margin-bottom: var(--space-2);
  }
  .group + .group-label,
  .hp-box + .group-label {
    margin-top: var(--space-4);
  }
  .hp-sub {
    display: flex;
    flex-wrap: wrap;
    gap: 0 var(--space-2);
  }
  .hp-mono,
  .hp-file {
    font-family: var(--font-mono);
  }
  .hp-mono {
    font-size: 11px;
  }
  .hp-trail {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .hp-box {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-lg, 10px);
    background: var(--bg-grouped-secondary, var(--fill-1));
  }
  .hp-add {
    display: flex;
    gap: var(--space-2);
  }
  .hp-add .input {
    flex: 1;
    min-width: 0;
  }
  .hp-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .hp-chip {
    padding-right: 2px;
  }
  .hp-x {
    width: 16px;
    height: 16px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: none;
    color: var(--text-dim);
    font: inherit;
    line-height: 1;
    cursor: pointer;
  }
  .hp-x:hover {
    background: var(--fill-2);
    color: var(--text);
  }
  .hp-bad {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--danger);
  }
  .hp-dim {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .hp-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-2);
    margin: var(--space-4) 0 var(--space-2);
  }
  .hp-note {
    margin: 0 0 var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
    text-align: right;
  }
  .hp-preview {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
  }
  .hp-stats {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .hp-file {
    margin: 0;
    max-height: 320px;
    overflow: auto;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
    font-size: 11.5px;
    line-height: 1.5;
    white-space: pre;
    color: var(--text-muted);
  }
  .hp-file .ln {
    display: block;
  }
  .hp-file .ours {
    color: var(--text);
    background: color-mix(in srgb, var(--success) 9%, transparent);
  }
  .hp-file .more {
    color: var(--text-dim);
    font-style: italic;
  }
  .hp-cmd {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
  }
  .hp-loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .hp-loading .skeleton {
    display: block;
    border-radius: var(--radius-md);
  }
</style>
