<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SocialIcon from "./SocialIcon.svelte";
  import { social, tool, type Platform } from "./api";
  import { toolError } from "./insights";
  import { QUICK_FILTERS, savable, type FilterList, type NamedFilter } from "./second";

  let {
    platform,
    current,
    active,
    onpick,
  }: { platform: Platform; current: Record<string, unknown>; active: string | null; onpick: (name: string | null) => void } = $props();

  let items = $state<NamedFilter[]>([]);
  let loading = $state(true);
  let failed = $state(false);
  let naming = $state(false);
  let name = $state("");
  let saving = $state(false);

  let fields = $derived(savable(current));

  function label(f: NamedFilter): string {
    return f.kind === "quick" && (QUICK_FILTERS as readonly string[]).includes(f.name) ? ($t(`social_audience.saved.quick.${f.name}`) as string) : f.name;
  }

  async function load() {
    loading = true;
    failed = false;
    try {
      const r = await social.call<FilterList>(tool(platform, "audience"), { view: "filters" });
      items = r.items;
    } catch {
      failed = true;
    } finally {
      loading = false;
    }
  }

  function failure(e: unknown): string {
    const err = toolError(e);
    if (err.code === "TOO_MANY_FILTERS") return $t("social_audience.saved.full") as string;
    if (err.code === "RESERVED_NAME") return $t("social_audience.saved.reserved") as string;
    return err.message;
  }

  async function save() {
    if (!fields || !name.trim() || saving) return;
    saving = true;
    try {
      const r = await social.call<{ name: string; filters: NamedFilter[] }>(tool(platform, "audience_mark"), { list: "filters", name: name.trim(), filter: fields });
      items = r.filters;
      naming = false;
      name = "";
      showToast("success", $t("social_audience.saved.saved_done") as string);
      onpick(r.name);
    } catch (e) {
      showToast("error", failure(e));
    } finally {
      saving = false;
    }
  }

  async function remove(f: NamedFilter) {
    try {
      const r = await social.call<{ filters: NamedFilter[] }>(tool(platform, "audience_mark"), { list: "filters", name: f.name, remove: true });
      items = r.filters;
      if (active?.toLowerCase() === f.name.toLowerCase()) onpick(null);
    } catch (e) {
      showToast("error", failure(e));
    }
  }

  $effect(() => {
    void platform;
    untrack(() => void load());
  });
</script>

<div class="af" role="group" aria-label={$t("social_audience.saved.label")}>
  <span class="af-label">{$t("social_audience.saved.label")}</span>
  {#if loading}
    <span class="spinner"></span>
  {:else if failed}
    <button class="btn btn-ghost btn-sm" onclick={() => void load()}>{$t("social.common.retry")}</button>
  {:else}
    {#each items as f (f.kind + f.name)}
      <span class="af-chip" class:af-on={active?.toLowerCase() === f.name.toLowerCase()}>
        <button class="af-pick" aria-pressed={active?.toLowerCase() === f.name.toLowerCase()} title={f.about ?? ""} onclick={() => onpick(active?.toLowerCase() === f.name.toLowerCase() ? null : f.name)}>{label(f)}</button>
        {#if f.kind === "saved"}
          <button class="af-x" aria-label={$t("social_audience.saved.remove", { name: f.name })} title={$t("social_audience.saved.remove", { name: f.name })} onclick={() => void remove(f)}>×</button>
        {/if}
      </span>
    {/each}
    {#if naming}
      <form class="af-form" onsubmit={(e) => { e.preventDefault(); void save(); }}>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input" bind:value={name} maxlength="40" placeholder={$t("social_audience.saved.save_name")} aria-label={$t("social_audience.saved.save_name")} autofocus onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); naming = false; } }} />
        <button class="btn btn-primary btn-sm" disabled={!name.trim() || saving}>{$t("social_audience.saved.save_confirm")}</button>
      </form>
    {:else}
      <button class="btn btn-ghost btn-sm" disabled={!fields} title={fields ? "" : $t("social_audience.saved.need_fields")} onclick={() => (naming = true)}>
        <SocialIcon name="plus" /> {$t("social_audience.saved.save")}
      </button>
    {/if}
  {/if}
</div>

<style>
  .af {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
  }

  .af-label {
    margin-right: var(--space-1);
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .af-chip {
    display: inline-flex;
    align-items: center;
    height: var(--control-h);
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-dim);
    font-size: var(--text-xs);
  }

  .af-on {
    background: var(--accent);
    color: var(--on-accent);
  }

  .af-pick,
  .af-x {
    height: 100%;
    border: 0;
    border-radius: var(--radius-full);
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .af-pick {
    padding: 0 var(--space-3);
  }

  .af-x {
    padding: 0 var(--space-2) 0 0;
  }

  .af-pick:focus-visible,
  .af-x:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: 2px;
  }

  .af-form {
    display: inline-flex;
    gap: var(--space-1);
  }

  .af-form .input {
    width: 160px;
  }
</style>
