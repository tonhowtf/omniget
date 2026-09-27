<script lang="ts">
  import { t } from "$lib/i18n";
  import type { Conflict, PowerAction } from "$lib/washmybelly/power";
  import Icon from "./ui/Icon.svelte";

  /// Two power managers fighting over the same knobs, and the one action
  /// we offer: keep one, turn the others off (undoable from the receipt).
  let { conflicts, action, onResolve }: { conflicts: Conflict[]; action: PowerAction | undefined; onResolve: (id: string) => void } = $props();

  let keep = $derived(conflicts[0]?.keep ?? "");
  let off = $derived([...new Set(conflicts.flatMap((c) => c.disable))]);
  let names = $derived([...new Set(conflicts.flatMap((c) => [c.a, c.b]))]);
</script>

<section class="card" role="alert">
  <span class="icon"><Icon name="warning" size={20} /></span>
  <div class="body">
    <h3>{$t("washmybelly.power.conflict.title")}</h3>
    <p>{$t("washmybelly.power.conflict.body", { names: names.join(", ") })}</p>
    {#if off.length}<p class="dim">{$t("washmybelly.power.conflict.will_stop", { units: off.join(", ") })}</p>{/if}
  </div>
  {#if action}
    <button class="btn btn-primary btn-sm" type="button" onclick={() => onResolve(action.id)}>{$t("washmybelly.power.conflict.keep", { name: keep })}</button>
  {/if}
</section>

<style>
  .card { display: flex; align-items: flex-start; gap: 12px; border-radius: var(--radius-lg, 12px); padding: 12px 16px; background: color-mix(in srgb, var(--warning) 10%, var(--surface)); box-shadow: inset 0 0 0 var(--hairline, 1px) color-mix(in srgb, var(--warning) 35%, transparent); }
  .icon { color: var(--warning); flex: none; margin-top: 1px; }
  .body { flex: 1; min-width: 0; }
  h3 { margin: 0; font-size: 13px; font-weight: 600; }
  p { margin: 3px 0 0; font-size: 12.5px; }
  .dim { font-size: 11.5px; color: var(--text-dim); }
</style>
