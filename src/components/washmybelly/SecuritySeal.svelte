<script lang="ts">
  import { t } from "$lib/i18n";
  import { classKey, toneOf, type SecurityClass } from "$lib/washmybelly/security";

  /// The signature seal of one item. Ported from the "dot indicator" badge
  /// of "Styled Badges" (ai2, 21st.dev): a pill with a leading dot, here
  /// coloured by tone only (green = Apple's checks pass, amber = look at
  /// it yourself, grey = unknown) and without the pulse animation.
  let { cls, title }: { cls: SecurityClass | null | undefined; title?: string } = $props();

  let tone = $derived(toneOf(cls));
</script>

<span class="seal seal-{tone}" {title}>
  <span class="seal-dot" aria-hidden="true"></span>
  {$t(classKey(cls))}
</span>

<style>
  .seal {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
    padding: 1px 8px 1px 7px;
    border-radius: var(--radius-full);
    font-size: var(--text-caption);
    font-weight: 600;
    white-space: nowrap;
    background: var(--fill-2);
    color: var(--text-muted);
  }
  .seal-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
  }
  .seal-good {
    background: color-mix(in srgb, var(--success, #30d158) 15%, transparent);
    color: var(--success, #30d158);
  }
  .seal-neutral {
    background: color-mix(in srgb, var(--accent) 13%, transparent);
    color: var(--accent);
  }
  .seal-warn {
    background: color-mix(in srgb, var(--warning, #ff9f0a) 17%, transparent);
    color: var(--warning, #ff9f0a);
  }
</style>
