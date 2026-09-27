<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { errorText } from "$lib/washmybelly/format";
  import { planItemId, sortTasks, taskKey, type MaintenanceList, type MaintenanceTask } from "$lib/washmybelly/maintenance";
  import { moduleById, type Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import PlanReview from "./ui/PlanReview.svelte";
  import MaintenanceCard from "./MaintenanceCard.svelte";

  /// Maintenance: repairs with a real effect for this system (DNS cache,
  /// "Open With" list, Spotlight, Quick Look, disk check; sfc and DISM on
  /// Windows; fstrim on Linux). Each "Run" goes through a one-item plan of
  /// kind `maintenance`; the tool's last lines land in the receipt.
  let { caps }: { caps: Caps } = $props();

  const mod = moduleById("maintenance");

  let tasks = $state<MaintenanceTask[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let reviewIds = $state<string[]>([]);
  let reviewOpen = $state(false);
  let reviewTitle = $state<string | undefined>(undefined);

  let sorted = $derived(sortTasks(tasks));

  async function load() {
    error = null;
    loading = true;
    try {
      const r = await api.ui<MaintenanceList>("maintenance.list");
      tasks = r.tasks ?? [];
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load();
  });

  function title(task: MaintenanceTask): string {
    const k = taskKey(task.id, "title");
    const s = $t(k) as string;
    return s === k ? task.title : s;
  }

  async function run(task: MaintenanceTask) {
    error = null;
    try {
      // Registers the task as a plan candidate; PlanReview builds the plan from its id.
      await api.ui("maintenance.plan", { ids: [task.id] });
      reviewIds = [planItemId(task.id)];
      reviewTitle = $t("washmybelly.maintenance.review", { name: title(task) }) as string;
      reviewOpen = true;
    } catch (e) {
      error = errorText(e);
    }
  }

  function groupLabel(key: string): string {
    const k = `washmybelly.maintenance.group.${key}`;
    const s = $t(k) as string;
    return s === k ? key : s;
  }
</script>

<ModuleHeader title={$t("washmybelly.modules.maintenance") as string} subtitle={$t("washmybelly.maintenance.subtitle") as string} tile={mod} />

{#if loading && tasks.length === 0}
  <div class="mt-grid" aria-busy="true">
    {#each [0, 1, 2, 3] as i (i)}<span class="skeleton" style:height="180px"></span>{/each}
  </div>
{:else if error && tasks.length === 0}
  <EmptyState art="worried" title={$t("washmybelly.error_title") as string} body={error}>
    {#snippet action()}
      <button class="btn btn-primary" type="button" onclick={() => void load()}>{$t("washmybelly.retry")}</button>
    {/snippet}
  </EmptyState>
{:else if tasks.length === 0}
  <EmptyState art="sleeping" title={$t("washmybelly.maintenance.empty_title") as string} />
{:else}
  {#if error}<p class="mt-error">{error}</p>{/if}
  <div class="mt-grid">
    {#each sorted as task (task.id)}
      <MaintenanceCard {task} busy={reviewOpen} onRun={(tk) => void run(tk)} />
    {/each}
  </div>
  <p class="group-footer">{$t(caps.os === "windows" ? "washmybelly.maintenance.footer_windows" : "washmybelly.maintenance.footer")}</p>
{/if}

<PlanReview kind="maintenance" itemIds={reviewIds} bind:open={reviewOpen} title={reviewTitle} confirmLabel={$t("washmybelly.maintenance.run") as string} {groupLabel} />

<style>
  .mt-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }
  .mt-grid .skeleton {
    display: block;
    border-radius: var(--radius-lg, 12px);
  }
  .mt-error {
    margin: 0 0 var(--space-3);
    font-size: var(--text-sm);
    color: var(--warning, #ff9f0a);
  }
</style>
