<script lang="ts">
  import { t } from "$lib/i18n";
  import { cssColor, parseColor, sameColor, toHex, type Oklch } from "$lib/palette/color";

  let { color, label, onchange }: { color: Oklch; label: string; onchange: (c: Oklch) => void } = $props();

  let text = $state("");
  let invalid = $state(false);
  let last: Oklch | null = null;

  $effect(() => {
    if (!last || !sameColor(last, color)) {
      text = toHex(color);
      invalid = false;
      last = color;
    }
  });

  function commit() {
    const parsed = parseColor(text);
    if (!parsed) {
      invalid = true;
      return;
    }
    invalid = false;
    last = parsed;
    onchange(parsed);
  }
</script>

<label class="pl-cf">
  <span class="pl-cf-swatch" style:--c={cssColor(color)} aria-hidden="true"></span>
  <input
    class="input"
    class:error={invalid}
    type="text"
    spellcheck="false"
    autocomplete="off"
    aria-label={label}
    aria-invalid={invalid}
    bind:value={text}
    onchange={commit}
    onkeydown={(e) => e.key === "Enter" && commit()}
  />
</label>
{#if invalid}
  <p class="pl-cf-err" role="alert">{$t("palette.invalid_color")}</p>
{/if}

<style>
  .pl-cf {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .pl-cf-swatch {
    width: var(--control-h);
    height: var(--control-h);
    flex-shrink: 0;
    border-radius: var(--radius-sm);
    background: var(--c);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-cf .input {
    min-width: 0;
    flex: 1;
    font-family: var(--font-mono);
  }

  .pl-cf-err {
    margin: var(--space-1) 0 0;
    font-size: var(--text-xs);
    color: var(--error);
  }
</style>
