<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";

  type AuditRow = {
    id: number;
    ts_ms: number;
    actor: string;
    channel: string;
    action: string;
    entity_kind: string;
    entity_id: string;
    details: Record<string, unknown> | null;
  };

  const SPANS = ["1h", "1d", "7d", "30d", "90d"] as const;
  const KINDS = ["", "job.", "ask.", "grant.", "token.", "agent.", "loop."] as const;
  const KIND_KEY: Record<string, string> = {
    "": "all", "job.": "job", "ask.": "ask", "grant.": "grant", "token.": "token", "agent.": "agent", "loop.": "loop",
  };

  let since = $state<(typeof SPANS)[number]>("1d");
  let action = $state<(typeof KINDS)[number]>("");
  let rows = $state<AuditRow[]>([]);
  let retention = $state(90);
  let retentionInput = $state(90);
  let loading = $state(false);
  let failed = $state(false);
  let saved = $state(false);

  async function load() {
    loading = true;
    failed = false;
    try {
      const r = await invoke<{ rows: AuditRow[]; retention_days: number | null }>("llm_audit_list", {
        since,
        action: action || null,
        limit: 1000,
      });
      rows = r.rows;
      if (r.retention_days) {
        retention = r.retention_days;
        retentionInput = r.retention_days;
      }
    } catch {
      failed = true;
    } finally {
      loading = false;
    }
  }

  async function saveRetention() {
    const days = Math.min(3650, Math.max(1, Math.round(Number(retentionInput))));
    if (!Number.isFinite(days)) return;
    try {
      await invoke("llm_audit_retention_set", { days });
      retention = days;
      saved = true;
      void load();
    } catch {
      failed = true;
    }
  }

  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }

  function csvCell(v: string): string {
    return /[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
  }

  function exportCsv() {
    const head = ["ts", "action", "actor", "channel", "entity_kind", "entity_id", "details"];
    const lines = [head.join(",")].concat(
      rows.map((r) =>
        [new Date(r.ts_ms).toISOString(), r.action, r.actor, r.channel, r.entity_kind, r.entity_id, JSON.stringify(r.details ?? {})]
          .map(csvCell)
          .join(","),
      ),
    );
    const url = URL.createObjectURL(new Blob([lines.join("\n") + "\n"], { type: "text/csv" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = `omniget-audit-${since}.csv`;
    a.click();
    URL.revokeObjectURL(url);
  }

  onMount(() => void load());
</script>

<div class="audit">
  <p class="lede">{$t("llm.audit.lede")}</p>
  <div class="bar">
    <label class="field">
      <span class="field-label">{$t("llm.audit.since")}</span>
      <select class="input" bind:value={since} onchange={() => void load()}>
        {#each SPANS as s (s)}<option value={s}>{$t(`llm.audit.since_${s}`)}</option>{/each}
      </select>
    </label>
    <label class="field">
      <span class="field-label">{$t("llm.audit.kind")}</span>
      <select class="input" bind:value={action} onchange={() => void load()}>
        {#each KINDS as k (k)}<option value={k}>{$t(`llm.audit.kind_${KIND_KEY[k]}`)}</option>{/each}
      </select>
    </label>
    <button type="button" class="button" disabled={loading} onclick={() => void load()}>{$t("llm.audit.refresh")}</button>
    <button type="button" class="button" disabled={rows.length === 0} onclick={exportCsv}>{$t("llm.audit.export")}</button>
  </div>

  {#if failed}
    <p class="note" role="alert">{$t("llm.audit.error")}</p>
  {:else if rows.length === 0 && !loading}
    <p class="note">{$t("llm.audit.empty")}</p>
  {:else}
    <ol class="rows" aria-label={$t("llm.audit.title")}>
      {#each rows as r (r.id)}
        <li class="row">
          <span class="when">{when(r.ts_ms)}</span>
          <span class="action">{r.action}</span>
          <span class="actor" title={$t("llm.audit.who") as string}>{r.actor}</span>
          <span class="channel" title={$t("llm.audit.through") as string}>{r.channel}</span>
          <span class="entity" title={$t("llm.audit.on") as string}>{r.entity_kind}:{r.entity_id}</span>
        </li>
      {/each}
    </ol>
  {/if}

  <div class="retention">
    <label class="field">
      <span class="field-label">{$t("llm.audit.retention")}</span>
      <input class="input" type="number" min="1" max="3650" bind:value={retentionInput} oninput={() => (saved = false)} />
    </label>
    <span class="hint">{$t("llm.audit.retention_days", { days: retention })}</span>
    <button type="button" class="button" disabled={retentionInput === retention} onclick={() => void saveRetention()}>{$t("llm.audit.retention_save")}</button>
    {#if saved}<span class="hint" role="status">{$t("llm.audit.retention_saved")}</span>{/if}
  </div>
  <p class="hint"><code>omniget audit --since 1d</code> · {$t("llm.audit.cli_hint")}</p>
</div>

<style>
  .audit { display: flex; flex-direction: column; gap: var(--space-3); padding: var(--space-4); }
  .lede, .note, .hint { font-size: var(--text-sm); color: var(--text-dim); margin: 0; }
  .bar, .retention { display: flex; flex-wrap: wrap; align-items: flex-end; gap: var(--space-2); }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .rows { list-style: none; margin: 0; padding: 0; max-height: 420px; overflow-y: auto; border-top: var(--hairline) solid var(--separator); }
  .row {
    display: grid; grid-template-columns: 11rem 9rem minmax(8rem, 1fr) 5rem minmax(8rem, 1.2fr);
    gap: var(--space-2); padding: 6px 0; font-size: var(--text-caption);
    border-bottom: var(--hairline) solid var(--separator); align-items: baseline;
  }
  .row span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .when, .channel { color: var(--text-dim); }
  .action { font-weight: 600; }
  .actor, .entity { font-family: var(--font-mono, monospace); }
  @media (max-width: 720px) { .row { grid-template-columns: 1fr 1fr; } }
</style>
