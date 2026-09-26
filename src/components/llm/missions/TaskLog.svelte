<script lang="ts">
  /** A task's log, redacted by the backend and read one page at a time. */
  import { t } from "$lib/i18n";
  import { errorText, missionLog, type LogPage } from "$lib/stores/assist-missions-store.svelte";

  let { missionId, taskId }: { missionId: string; taskId: string } = $props();

  const PAGE = 200;
  let lines = $state<string[]>([]);
  let page = $state<LogPage | null>(null);
  let loading = $state(false);
  let error = $state("");

  async function load(offset: number, append: boolean) {
    loading = true;
    error = "";
    try {
      const p = await missionLog(missionId, taskId, offset, PAGE);
      page = p;
      lines = append ? [...lines, ...p.lines] : p.lines;
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void missionId;
    void taskId;
    void load(0, false);
  });
</script>

<div class="log">
  {#if error}
    <p class="error" role="alert">{error}</p>
  {:else if loading && !page}
    <p class="hint" role="status">{$t("mission.log.loading")}</p>
  {:else if page}
    <p class="hint">{$t("mission.log.redacted")} · {$t("mission.log.count", { shown: lines.length, total: page.total })}</p>
    <pre class="pre">{lines.join("\n") || $t("mission.log.empty")}</pre>
    {#if page.clipped}<p class="hint">{$t("mission.log.clipped")}</p>{/if}
    <div class="actions">
      {#if page.next != null}
        <button type="button" class="button" disabled={loading} onclick={() => void load(page!.next!, true)}>{$t("mission.log.more")}</button>
      {/if}
      <button type="button" class="button" disabled={loading} onclick={() => void load(0, false)}>{$t("mission.log.reload")}</button>
    </div>
  {/if}
</div>

<style>
  .log { display: flex; flex-direction: column; gap: var(--space-2); min-width: 0; }
  .hint { margin: 0; font-size: var(--text-xs); color: var(--text-dim); }
  .error { margin: 0; font-size: var(--text-sm); color: var(--red); overflow-wrap: anywhere; }
  .pre { margin: 0; padding: var(--space-2); max-height: 320px; overflow: auto; border-radius: var(--radius-sm); background: var(--fill-2); font-family: var(--font-mono); font-size: var(--text-xs); white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
  .actions { display: flex; gap: var(--space-2); flex-wrap: wrap; }
</style>
