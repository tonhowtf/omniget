<script lang="ts" module>
  export type PreviewProp = "x" | "scale" | "opacity" | "rotate";
</script>

<script lang="ts">
  let { progress, prop, label, tone = "accent" }: { progress: number; prop: PreviewProp; label: string; tone?: "accent" | "teal" } = $props();

  let style = $derived.by(() => {
    const p = progress;
    switch (prop) {
      case "x":
        return `transform: translateX(calc(${p} * (100cqw - 36px)))`;
      case "scale":
        return `transform: translateX(calc(50cqw - 18px)) scale(${0.3 + 0.9 * p})`;
      case "opacity":
        return `transform: translateX(calc(50cqw - 18px)); opacity: ${Math.min(1, Math.max(0, p))}`;
      case "rotate":
        return `transform: translateX(calc(50cqw - 18px)) rotate(${p * 180}deg)`;
    }
  });
</script>

<div class="pl">
  <span class="pl-label">{label}</span>
  <div class="pl-track">
    <div class="pl-box tone-{tone}" {style}></div>
  </div>
</div>

<style>
  .pl {
    display: grid;
    grid-template-columns: 20px 1fr;
    align-items: center;
    gap: var(--space-2);
  }

  .pl-label {
    font-size: var(--text-xs);
    font-weight: 700;
    color: var(--text-muted);
    text-align: center;
  }

  .pl-track {
    container-type: inline-size;
    position: relative;
    height: 52px;
    display: flex;
    align-items: center;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    overflow: hidden;
  }

  .pl-box {
    width: 36px;
    height: 36px;
    border-radius: var(--radius-md);
    will-change: transform, opacity;
  }

  .tone-accent {
    background: var(--accent);
  }

  .tone-teal {
    background: var(--teal);
  }
</style>
