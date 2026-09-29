<script lang="ts">
  /** Deliverables attached to an issue; pending ones can be accepted or rejected. */
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { rpc, errorText, type WorkProduct } from "$lib/company/rpc";
  import { relTime } from "./company-ui";

  let { company, items }: { company: string; items: WorkProduct[] } = $props();

  async function review(wp: WorkProduct, verdict: "accepted" | "rejected") {
    try {
      await rpc("issue.work_product_review", { company, work_product: wp.id, verdict });
    } catch (e) {
      showToast("error", errorText(e));
    }
  }

  function tone(r: string): string {
    return r === "accepted" ? "tag-success" : r === "rejected" ? "tag-danger" : r === "pending" ? "tag-warning" : "";
  }
</script>

<div class="group">
  {#each items as wp (wp.id)}
    <div class="group-row">
      <span class="kind">{wp.kind}</span>
      <div class="group-row-content">
        <span class="group-row-title">
          {#if wp.url}
            <button type="button" class="link" onclick={() => wp.url && void openExternal(wp.url)}>{wp.title || wp.url}</button>
          {:else}
            {wp.title || wp.path || wp.kind}
          {/if}
        </span>
        {#if wp.summary || wp.path}
          <span class="group-row-sub">{wp.summary ?? ""}{wp.summary && wp.path ? " · " : ""}{#if wp.path}<code>{wp.path}</code>{/if}</span>
        {/if}
      </div>
      <div class="group-row-trailing">
        <span class="hint">{relTime(wp.created_ms, $t)}</span>
        {#if wp.review === "pending"}
          <button type="button" class="btn btn-secondary btn-sm" onclick={() => review(wp, "rejected")}>{$t("company.issue.wp_reject")}</button>
          <button type="button" class="btn btn-primary btn-sm" onclick={() => review(wp, "accepted")}>{$t("company.issue.wp_accept")}</button>
        {:else if wp.review !== "none"}
          <span class="tag {tone(wp.review)}">{$t(`company.issue.wp_review.${wp.review}`)}</span>
        {/if}
      </div>
    </div>
  {:else}
    <p class="hint pad">{$t("company.issue.no_work_products")}</p>
  {/each}
</div>

<style>
  .kind {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
    padding: 0 6px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
    text-align: left;
  }
  .link:hover {
    text-decoration: underline;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .pad {
    padding: var(--space-3);
  }
</style>
