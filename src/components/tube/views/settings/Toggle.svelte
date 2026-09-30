<script lang="ts">
  // Switch in the OmniTube look (accent track, white knob).
  let {
    checked,
    label,
    disabled = false,
    onchange,
  }: { checked: boolean; label: string; disabled?: boolean; onchange: (v: boolean) => void } = $props();
</script>

<button class="tg" class:on={checked} role="switch" aria-checked={checked} aria-label={label} {disabled} onclick={() => onchange(!checked)}>
  <span class="tg-knob"></span>
</button>

<style>
  .tg {
    position: relative; flex: none;
    width: 40px; height: 22px; padding: 0; border: 0; border-radius: 999px;
    background: var(--yt-tonal-hover); cursor: pointer;
    transition: background var(--yt-dur-base) var(--yt-ease-standard);
  }
  .tg.on { background: var(--yt-accent); }
  .tg:disabled { opacity: .4; cursor: default; }
  .tg:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 2px; }
  .tg-knob {
    position: absolute; top: 3px; left: 3px; width: 16px; height: 16px; border-radius: 50%;
    background: #fff; box-shadow: 0 1px 3px rgba(0, 0, 0, .3);
    transition: transform var(--yt-dur-base) var(--yt-ease-standard);
  }
  .tg.on .tg-knob { transform: translateX(18px); }
  @media (prefers-reduced-motion: reduce) { .tg, .tg-knob { transition: none; } }
</style>
