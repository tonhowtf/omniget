<script lang="ts">
  /**
   * One company: header (name, mission, status with pause/resume) and the
   * inner tabs. Each tab loads its own data; the shell only keeps the
   * company and the inbox count fresh.
   */
  import type { Snippet } from "svelte";
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { errorText, onCompanyChange, rpc, type Company } from "$lib/company/rpc";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { avatarColor } from "$components/company/company-ui";

  let { children }: { children: Snippet } = $props();

  let id = $derived(page.params.id ?? "");
  let base = $derived(`/llm/company/${id}`);
  let path = $derived(page.url.pathname.replace(/\/$/, ""));

  let company = $state<Company | null>(null);
  let missing = $state(false);
  let inboxCount = $state(0);
  let busy = $state(false);

  const TABS = [
    { seg: "", key: "overview" },
    { seg: "issues", key: "issues" },
    { seg: "inbox", key: "inbox" },
    { seg: "team", key: "team" },
    { seg: "goals", key: "goals" },
    { seg: "routines", key: "routines" },
    { seg: "costs", key: "costs" },
    { seg: "activity", key: "activity" },
    { seg: "settings", key: "settings" },
  ] as const;

  function href(seg: string) {
    return seg ? `${base}/${seg}` : base;
  }

  function current(seg: string) {
    const h = href(seg);
    return seg ? path === h || path.startsWith(h + "/") : path === h;
  }

  async function load(cid: string) {
    try {
      const c = await rpc<Company>("company.get", { company: cid });
      if (cid !== id) return;
      company = c;
      missing = false;
    } catch (e) {
      if (cid !== id) return;
      company = null;
      missing = true;
      console.warn(errorText(e));
    }
    try {
      const inbox = await rpc<{ issues: unknown[]; approvals: unknown[]; incidents: unknown[] }>("inbox.list", { company: cid });
      if (cid === id) inboxCount = inbox.issues.length + inbox.approvals.length + inbox.incidents.length;
    } catch {
      /* the count is decoration */
    }
  }

  $effect(() => {
    const cid = id;
    if (!cid) return;
    company = null;
    missing = false;
    inboxCount = 0;
    void load(cid);
    const un = onCompanyChange(cid, () => void load(cid));
    return () => void un.then((f) => f());
  });

  async function setStatus(status: "active" | "paused") {
    if (!company) return;
    busy = true;
    try {
      company = await rpc<Company>("company.status", { company: company.id, status });
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>{company?.name ?? $t("company.nav")}</title></svelte:head>

<div class="company-shell">
  {#if missing}
    <div class="empty-state">
      <img class="empty-state-art" src="/emoji/package.png" alt="" width="72" height="72" draggable="false" />
      <div class="empty-state-title">{$t("company.common.not_found")}</div>
      <a class="button" href="/llm/company">{$t("company.common.back_to_list")}</a>
    </div>
  {:else}
    <header class="company-head">
      <a class="back" href="/llm/company" aria-label={$t("company.common.back_to_list")}>‹</a>
      {#if company}
        <span class="tile" style:background={avatarColor(company.name)} aria-hidden="true">{company.issue_prefix}</span>
        <div class="titles">
          <div class="name-line">
            <h1 class="company-name">{company.name}</h1>
            <span class="tag {company.status === 'active' ? 'tag-success' : company.status === 'paused' ? 'tag-warning' : ''}">
              {$t(`company.common.company_status.${company.status}`)}
            </span>
          </div>
          {#if company.mission}<p class="mission" title={company.mission}>{company.mission}</p>{/if}
          {#if company.status === "paused" && company.pause_reason}<p class="reason">{company.pause_reason}</p>{/if}
        </div>
        <div class="head-actions">
          {#if company.status === "active"}
            <button type="button" class="button" disabled={busy} onclick={() => setStatus("paused")}>{$t("company.shell.pause")}</button>
          {:else if company.status === "paused"}
            <button type="button" class="button active" disabled={busy} onclick={() => setStatus("active")}>{$t("company.shell.resume")}</button>
          {/if}
        </div>
      {:else}
        <div class="skeleton" style="height: 36px; flex: 1; max-width: 360px"></div>
      {/if}
    </header>

    <nav class="company-tabs" aria-label={$t("company.shell.tabs")}>
      {#each TABS as tab (tab.key)}
        <a href={href(tab.seg)} aria-current={current(tab.seg) ? "page" : undefined}>
          {$t(`company.shell.tab.${tab.key}`)}
          {#if tab.key === "inbox" && inboxCount > 0}<span class="badge">{inboxCount}</span>{/if}
        </a>
      {/each}
    </nav>

    <div class="company-content">
      {#key id}{@render children()}{/key}
    </div>
  {/if}
</div>

<style>
  .company-shell { flex: 1; min-height: 0; display: flex; flex-direction: column; overflow: hidden; }
  .company-head { display: flex; align-items: center; gap: var(--space-3); padding: var(--space-4) var(--space-5) var(--space-3); flex-shrink: 0; }
  .back { font-size: 22px; line-height: 1; color: var(--text-dim); text-decoration: none; padding: 2px 6px; border-radius: var(--radius-sm); }
  .back:hover { background: var(--fill-1); color: var(--text); }
  .tile { display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0; width: 40px; height: 40px; border-radius: 10px; color: #fff; font-size: 11px; font-weight: 700; font-family: var(--font-mono); }
  .titles { flex: 1; min-width: 0; }
  .name-line { display: flex; align-items: center; gap: var(--space-2); min-width: 0; }
  .company-name { margin: 0; font-family: var(--font-display); font-size: var(--text-lg); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mission { margin: 2px 0 0; font-size: var(--text-sm); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .reason { margin: 2px 0 0; font-size: var(--text-xs); color: var(--warning); }
  .head-actions { display: flex; gap: var(--space-2); flex-shrink: 0; }
  .company-tabs { display: flex; gap: 4px 18px; flex-wrap: wrap; padding: 0 var(--space-5); border-bottom: var(--hairline) solid var(--separator); flex-shrink: 0; }
  .company-tabs a { display: inline-flex; align-items: center; gap: 6px; padding: 8px 0; font-size: var(--text-sm); color: var(--text-muted); text-decoration: none; border-bottom: 2px solid transparent; margin-bottom: -1px; }
  .company-tabs a:hover { color: var(--text); }
  .company-tabs a[aria-current="page"] { color: var(--text); font-weight: 600; border-bottom-color: var(--accent); }
  .company-tabs a:focus-visible { outline: var(--focus-ring); border-radius: var(--radius-xs); }
  .company-content { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; }
  @media (max-width: 600px) { .company-head { padding-inline: var(--space-3); } .company-tabs { padding-inline: var(--space-3); } }
</style>
