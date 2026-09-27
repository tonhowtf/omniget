<script lang="ts">
  /// Segmented control (radio group) with a sliding thumb and arrow-key
  /// navigation. Ported from ddoemonn's "Segmented Control" on 21st.dev:
  /// motion springs became a CSS transform transition, colours the app's
  /// tokens, and the masked label copy became a plain label on the thumb.
  type Option = { value: string; label: string; disabled?: boolean };
  let {
    options,
    value,
    label,
    busy = false,
    onChange,
  }: { options: Option[]; value: string | null | undefined; label: string; busy?: boolean; onChange: (v: string) => void } = $props();

  let buttons: HTMLButtonElement[] = $state([]);
  let index = $derived(Math.max(0, options.findIndex((o) => o.value === value)));
  let known = $derived(options.some((o) => o.value === value));

  function seek(from: number, dir: number): number {
    let i = from;
    for (let k = 0; k < options.length; k++) {
      i = (i + dir + options.length) % options.length;
      if (!options[i]?.disabled) return i;
    }
    return from;
  }

  function go(i: number) {
    const o = options[i];
    if (!o || o.disabled || busy) return;
    buttons[i]?.focus();
    if (o.value !== value) onChange(o.value);
  }

  function key(e: KeyboardEvent, i: number) {
    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      go(seek(i, 1));
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      go(seek(i, -1));
    }
  }
</script>

<div class="seg" role="radiogroup" aria-label={label} aria-busy={busy} style:--n={options.length} style:--i={index}>
  {#if known}<span class="thumb" aria-hidden="true"></span>{/if}
  {#each options as o, i (o.value)}
    <button
      bind:this={buttons[i]}
      type="button"
      role="radio"
      aria-checked={known && i === index}
      tabindex={i === index ? 0 : -1}
      disabled={o.disabled || busy}
      class:on={known && i === index}
      onclick={() => go(i)}
      onkeydown={(e) => key(e, i)}
    >{o.label}</button>
  {/each}
</div>

<style>
  .seg {
    position: relative;
    display: grid;
    grid-template-columns: repeat(var(--n), minmax(0, 1fr));
    padding: 3px;
    border-radius: 9px;
    background: var(--surface-mut, var(--separator));
    box-shadow: inset 0 1px 2px rgb(0 0 0 / 0.07);
    user-select: none;
  }
  .thumb {
    position: absolute;
    top: 3px;
    bottom: 3px;
    left: 3px;
    width: calc((100% - 6px) / var(--n));
    transform: translateX(calc(var(--i) * 100%));
    border-radius: 6px;
    background: var(--surface-hi, var(--surface));
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.16), 0 0 0 0.5px rgb(0 0 0 / 0.06);
    transition: transform 0.28s cubic-bezier(0.3, 1.3, 0.5, 1);
  }
  button {
    all: unset;
    position: relative;
    z-index: 1;
    padding: 6px 12px;
    text-align: center;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-dim);
    border-radius: 6px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: pointer;
  }
  button.on { color: var(--text); }
  button:hover:not(:disabled):not(.on) { color: var(--text); }
  button:disabled { cursor: default; opacity: 0.55; }
  button.on:disabled { opacity: 1; }
  button:focus-visible { box-shadow: inset 0 0 0 1.5px var(--accent); }
  @media (prefers-reduced-motion: reduce) { .thumb { transition: none; } }
</style>
