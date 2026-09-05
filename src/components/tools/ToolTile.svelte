<script lang="ts">
  /**
   * Quadrado grande da grade de ferramentas: ícone, nome embaixo e, quando
   * precisa, o aviso de plataforma ou de "em breve". Uma ferramenta que só
   * roda no Windows ganha um selo no canto do ícone e a legenda "Só Windows",
   * para ninguém clicar achando que roda no Mac.
   */
  import { t } from "$lib/i18n";
  import ToolIcon from "$components/tools/ToolIcon.svelte";
  import { ALL_OS, type OsName, type ToolStatus } from "$lib/tools/catalog";

  let {
    href,
    label,
    sublabel,
    icon,
    from,
    to,
    via,
    platforms = ALL_OS,
    status = "ready",
    size = 96,
  }: {
    href: string;
    label: string;
    sublabel?: string;
    icon: string;
    from: string;
    to: string;
    via?: string;
    platforms?: OsName[];
    status?: ToolStatus;
    size?: number;
  } = $props();

  let onlyOs = $derived<OsName | null>(platforms.length === 1 ? platforms[0] : null);
  let soon = $derived(status === "soon");

  const OS_LABEL: Record<OsName, string> = {
    windows: "tools.hub.only_windows",
    macos: "tools.hub.only_macos",
    linux: "tools.hub.only_linux",
  };
</script>

<a class="tool-tile" {href} class:soon aria-label={label}>
  <span class="tool-tile-art">
    <ToolIcon {icon} {from} {to} {via} {size} muted={soon} />
    {#if onlyOs}
      <span class="os-badge" title={$t(OS_LABEL[onlyOs])}>
        {#if onlyOs === "windows"}
          <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true">
            <path d="M3 5.6 10.6 4.5v7.1H3zM11.6 4.4 21 3v8.6h-9.4zM3 12.4h7.6v7.1L3 18.4zM11.6 12.4H21V21l-9.4-1.3z" />
          </svg>
        {:else}
          <span class="os-badge-glyph" style:--glyph="url(/brands/{onlyOs === 'macos' ? 'apple' : 'linux'}.svg)"></span>
        {/if}
      </span>
    {/if}
  </span>
  <span class="tool-tile-label">{label}</span>
  {#if soon || onlyOs || status === "beta"}
    <span class="tool-tile-tags">
      {#if soon}
        <span class="tool-tile-sub tool-tile-soon">{$t("tools.hub.soon")}</span>
      {:else if status === "beta"}
        <span class="tool-tile-sub tool-tile-beta">{$t("tools.hub.beta")}</span>
      {/if}
      {#if onlyOs}
        <span class="tool-tile-sub tool-tile-os">{$t(OS_LABEL[onlyOs])}</span>
      {/if}
    </span>
  {:else if sublabel}
    <span class="tool-tile-sub">{sublabel}</span>
  {/if}
</a>

<style>
  .tool-tile {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-2) var(--space-3);
    border-radius: var(--radius-xl);
    text-decoration: none;
    color: inherit;
    text-align: center;
    -webkit-user-drag: none;
    transition: background var(--duration-fast) var(--ease-out);
  }

  .tool-tile-art {
    position: relative;
    display: inline-flex;
  }

  @media (hover: hover) {
    .tool-tile:hover {
      background: var(--fill-1);
    }
    .tool-tile:hover :global(.tool-icon) {
      transform: translateY(-2px) scale(1.03);
      box-shadow:
        inset 0 0 0 0.5px rgba(255, 255, 255, 0.3),
        inset 0 1px 0 rgba(255, 255, 255, 0.24),
        inset 0 -2px 3px rgba(0, 0, 0, 0.14),
        0 2px 4px rgba(0, 0, 0, 0.18),
        0 12px 24px -8px rgba(0, 0, 0, 0.45);
    }
  }

  .tool-tile:active :global(.tool-icon) {
    transform: scale(0.97);
  }

  .tool-tile:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  .tool-tile-label {
    max-width: 100%;
    font-family: var(--font-display);
    font-size: var(--text-base);
    font-weight: 600;
    letter-spacing: var(--track-snug);
    color: var(--text);
    line-height: 1.25;
    overflow-wrap: anywhere;
  }

  .tool-tile.soon .tool-tile-label {
    color: var(--text-muted);
  }

  .tool-tile-tags {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 4px;
  }

  .tool-tile-sub {
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text-dim);
    line-height: 1.2;
  }

  .tool-tile-soon {
    padding: 1px 7px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-muted);
  }

  .tool-tile-os {
    padding: 1px 7px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--warning) 16%, transparent);
    color: var(--warning);
  }

  .tool-tile-beta {
    padding: 1px 7px;
    border-radius: var(--radius-full);
    background: var(--accent-soft);
    color: var(--accent-hi);
  }

  .os-badge {
    position: absolute;
    right: -6px;
    bottom: -6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border-radius: 8px;
    background: var(--surface-hi);
    color: var(--text);
    box-shadow:
      inset 0 0 0 var(--hairline) var(--content-border),
      0 1px 3px rgba(0, 0, 0, 0.25);
  }

  .os-badge-glyph {
    display: block;
    width: 14px;
    height: 14px;
    background: currentColor;
    -webkit-mask: var(--glyph) center / contain no-repeat;
    mask: var(--glyph) center / contain no-repeat;
  }
</style>
