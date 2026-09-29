<script lang="ts">
  /**
   * Blocked by / blocking / related / subtasks of an issue, with removal, a
   * form to add a relation by identifier, and "new subtask".
   */
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { rpc, errorText, type Issue, type IssueDetail, type IssueLink } from "$lib/company/rpc";
  import IssueStatusBadge from "./IssueStatusBadge.svelte";

  let {
    company,
    detail,
    issues,
    hrefOf,
    onsubtask,
  }: {
    company: string;
    detail: IssueDetail;
    issues: Issue[];
    hrefOf: (identifier: string) => string;
    onsubtask: () => void;
  } = $props();

  type Kind = "blocked_by" | "blocking" | "related" | "duplicate";
  let kind = $state<Kind>("blocked_by");
  let target = $state("");
  let busy = $state(false);

  let me = $derived(detail.issue);
  let sections = $derived([
    { key: "blocked_by", items: detail.blocked_by },
    { key: "blocking", items: detail.blocking },
    { key: "related", items: detail.related },
    { key: "children", items: detail.children },
  ]);
  let candidates = $derived(issues.filter((i) => i.id !== me.id));

  function pair(k: Kind, other: string): [string, string, string] {
    if (k === "blocked_by") return [other, me.id, "blocks"];
    if (k === "blocking") return [me.id, other, "blocks"];
    return [me.id, other, k];
  }

  async function add(e: Event) {
    e.preventDefault();
    const other = target.trim();
    if (!other || busy) return;
    busy = true;
    const [from, to, rel] = pair(kind, other);
    try {
      await rpc("issue.relate", { company, from, to, kind: rel });
      target = "";
    } catch (err) {
      showToast("error", errorText(err));
    } finally {
      busy = false;
    }
  }

  async function remove(section: string, link: IssueLink) {
    try {
      if (section === "blocked_by") await rpc("issue.unrelate", { company, from: link.id, to: me.id, kind: "blocks" });
      else if (section === "blocking") await rpc("issue.unrelate", { company, from: me.id, to: link.id, kind: "blocks" });
      else {
        const rel = link.kind === "duplicate" ? "duplicate" : "related";
        await rpc("issue.unrelate", { company, from: me.id, to: link.id, kind: rel });
        await rpc("issue.unrelate", { company, from: link.id, to: me.id, kind: rel });
      }
    } catch (err) {
      showToast("error", errorText(err));
    }
  }
</script>

<div class="relations">
  {#each sections as s (s.key)}
    {#if s.items.length > 0}
      <div class="group-label">{$t(`company.issue.rel.${s.key}`)}</div>
      <div class="group">
        {#each s.items as l (l.id + l.kind)}
          <div class="list-row compact">
            <IssueStatusBadge status={l.status} label={false} />
            <a class="ident" href={hrefOf(l.identifier)}>{l.identifier}</a>
            <a class="title" href={hrefOf(l.identifier)}>{l.title}</a>
            {#if s.key !== "children"}
              <button type="button" class="btn btn-ghost btn-sm" onclick={() => remove(s.key, l)} aria-label={$t("company.issue.rel.remove")}>
                {$t("company.issue.rel.remove")}
              </button>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  {/each}

  <form class="add" onsubmit={add}>
    <select class="input" bind:value={kind} aria-label={$t("company.issue.rel.kind")}>
      <option value="blocked_by">{$t("company.issue.rel.blocked_by")}</option>
      <option value="blocking">{$t("company.issue.rel.blocking")}</option>
      <option value="related">{$t("company.issue.rel.related")}</option>
      <option value="duplicate">{$t("company.issue.rel.duplicate")}</option>
    </select>
    <input
      class="input grow"
      type="text"
      list="company-issue-targets"
      bind:value={target}
      placeholder={$t("company.issue.rel.target_placeholder")}
      aria-label={$t("company.issue.rel.target")}
    />
    <datalist id="company-issue-targets">
      {#each candidates as i (i.id)}<option value={i.identifier}>{i.title}</option>{/each}
    </datalist>
    <button type="submit" class="btn btn-secondary btn-sm" disabled={!target.trim() || busy}>{$t("company.issue.rel.add")}</button>
    <button type="button" class="btn btn-ghost btn-sm" onclick={onsubtask}>{$t("company.issue.new_subtask")}</button>
  </form>
</div>

<style>
  .relations {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .list-row {
    border-radius: 0;
    gap: var(--space-2);
  }
  .ident {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
    text-decoration: none;
    flex-shrink: 0;
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
    text-decoration: none;
  }
  .title:hover {
    text-decoration: underline;
  }
  .add {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .add select {
    width: auto;
  }
  .grow {
    flex: 1;
    min-width: 140px;
  }
</style>
