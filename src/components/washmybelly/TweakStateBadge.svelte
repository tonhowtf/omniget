<script lang="ts">
  import { t } from "$lib/i18n";
  import { stateKey } from "$lib/washmybelly/tweaks";

  /// What Windows has now, read from the system. Ported from "Status Badge"
  let { state }: { state: string } = $props();
  let tone = $derived(state === "applied" ? "on" : state === "partial" ? "part" : state === "unknown" ? "unk" : "off");
</script>

<span class="tw-state tw-state-{tone}">
  <span class="dot" aria-hidden="true"></span>{$t(stateKey(state))}
</span>

<style>
  .tw-state {
    --tone: var(--text-dim);
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 18px;
    padding: 0 7px 0 6px;
    border-radius: var(--radius-full);
    font-size: var(--text-caption);
    font-weight: 600;
    white-space: nowrap;
    background: color-mix(in srgb, var(--tone) 13%, transparent);
    color: color-mix(in srgb, var(--tone) 85%, var(--text));
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--tone);
  }
  .tw-state-on {
    --tone: var(--success, #30d158);
  }
  .tw-state-part {
    --tone: var(--warning, #ff9f0a);
  }
  .tw-state-unk {
    --tone: var(--text-dim);
  }
  .tw-state-off {
    --tone: var(--text-muted);
  }
</style>
