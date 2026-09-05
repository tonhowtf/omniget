<script lang="ts">
  import type { Snippet } from "svelte";
  import HeroGradient from "./HeroGradient.svelte";

  type Action = {
    id: string;
    label: string;
    icon?: Snippet;
    onClick: () => void;
    primary?: boolean;
    busy?: boolean;
    disabled?: boolean;
    ariaLabel?: string;
  };

  type Stat = {
    label: string;
    value?: string | null;
  };

  type Props = {
    coverUrl?: string | null;
    eyebrow?: string | null;
    title: string;
    subtitle?: string | null;
    stats?: Stat[];
    actions?: Action[];
    description?: Snippet;
    coverShape?: "square" | "circle";
  };

  let {
    coverUrl = null,
    eyebrow = null,
    title,
    subtitle = null,
    stats = [],
    actions = [],
    description,
    coverShape = "square",
  }: Props = $props();

  const visibleStats = $derived(stats.filter((s) => s.value && s.value.trim() !== ""));
</script>

<HeroGradient coverUrl={coverUrl} minHeight={280}>
  <div class="hero-row">
    <div class="cover" class:circle={coverShape === "circle"}>
      {#if coverUrl}
        <img src={coverUrl} alt="" loading="eager" />
      {:else}
        <div class="cover-fallback" aria-hidden="true"></div>
      {/if}
    </div>
    <div class="meta">
      {#if eyebrow}
        <p class="eyebrow">{eyebrow}</p>
      {/if}
      <h1 class="title">{title}</h1>
      {#if subtitle}
        <p class="subtitle">{subtitle}</p>
      {/if}
      {#if visibleStats.length > 0}
        <p class="stats">
          {#each visibleStats as stat, idx (stat.label + idx)}
            {#if idx > 0}<span class="sep" aria-hidden="true">·</span>{/if}
            <span class="stat"><span class="stat-value">{stat.value}</span>{#if stat.label}
              <span class="stat-label"> {stat.label}</span>
            {/if}</span>
          {/each}
        </p>
      {/if}
      {#if description}
        <div class="description">{@render description()}</div>
      {/if}
      {#if actions.length > 0}
        <div class="actions">
          {#each actions as action (action.id)}
            <button
              type="button"
              class="action"
              class:primary={action.primary}
              disabled={action.disabled || action.busy}
              onclick={action.onClick}
              aria-label={action.ariaLabel ?? action.label}
            >
              {#if action.icon}
                <span class="icon">{@render action.icon()}</span>
              {/if}
              <span>{action.label}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </div>
</HeroGradient>

<style>
  .hero-row {
    display: flex;
    flex-direction: row;
    gap: 24px;
    padding: 24px 24px 28px;
    align-items: flex-end;
    min-height: 280px;
  }
  .cover {
    flex: 0 0 auto;
    width: 200px;
    height: 200px;
    border-radius: 12px;
    overflow: hidden;
    background: color-mix(in oklab, var(--button) 60%, transparent);
    box-shadow: 0 12px 24px rgba(0, 0, 0, 0.35);
  }
  .cover.circle {
    border-radius: 50%;
  }
  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .cover-fallback {
    width: 100%;
    height: 100%;
    background: linear-gradient(135deg, rgba(255, 255, 255, 0.04), rgba(255, 255, 255, 0.02));
  }
  .meta {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
    color: rgba(255, 255, 255, 0.95);
  }
  .eyebrow {
    margin: 0;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: rgba(255, 255, 255, 0.65);
  }
  .title {
    margin: 0;
    font-size: clamp(28px, 4vw, 44px);
    font-weight: 800;
    line-height: 1.05;
    color: #fff;
    word-break: break-word;
  }
  .subtitle {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: rgba(255, 255, 255, 0.85);
  }
  .stats {
    margin: 0;
    font-size: 13px;
    color: rgba(255, 255, 255, 0.7);
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: baseline;
  }
  .sep {
    color: rgba(255, 255, 255, 0.35);
  }
  .stat-value {
    font-weight: 600;
    color: rgba(255, 255, 255, 0.85);
  }
  .description {
    margin-top: 4px;
    color: rgba(255, 255, 255, 0.78);
    font-size: 13px;
    line-height: 1.5;
    max-width: 720px;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 8px;
  }
  .action {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 10px 18px;
    background: rgba(255, 255, 255, 0.1);
    border: none;
    color: rgba(255, 255, 255, 0.92);
    border-radius: 999px;
    font: inherit;
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
    transition: background 160ms ease, transform 160ms ease;
  }
  .action:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.16);
  }
  .action:active:not(:disabled) {
    transform: scale(0.98);
  }
  .action:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .action.primary {
    background: var(--accent);
    color: var(--on-accent);
    border-color: transparent;
  }
  .action.primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .icon {
    display: inline-grid;
    place-items: center;
    width: 16px;
    height: 16px;
  }
  @media (max-width: 720px) {
    .hero-row {
      flex-direction: column;
      align-items: flex-start;
      padding: 16px;
    }
    .cover {
      width: 140px;
      height: 140px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .action {
      transition: none;
    }
  }
</style>
