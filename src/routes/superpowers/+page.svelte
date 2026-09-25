<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import NavIcon from "$components/shell/NavIcon.svelte";

  // Hub for the optional abilities that used to sit loose in the sidebar.
  // Each card keeps its own on/off switch in Settings; the card only reflects it.
  type Power = {
    id: string;
    icon: string;
    title: string;
    descKey: string;
    href: string;
    enabled: boolean;
  };

  let powers = $derived<Power[]>([
    {
      id: "league",
      icon: "league",
      title: "League of Legends",
      descKey: "superpowers.league_desc",
      href: "/league",
      enabled: getSettings()?.league?.enabled ?? true,
    },
  ]);
</script>

<section class="powers">
  <header class="powers-head">
    <NavIcon icon="superpowers" size={56} />
    <div>
      <h1>{$t("nav.superpowers")}</h1>
      <p class="powers-subtitle">{$t("superpowers.subtitle")}</p>
    </div>
  </header>

  <div class="powers-grid">
    {#each powers as power (power.id)}
      <article class="power-card" class:off={!power.enabled}>
        <NavIcon icon={power.icon} size={64} />
        <div class="power-copy">
          <h2>{power.title}</h2>
          <p>{$t(power.descKey)}</p>
        </div>
        {#if power.enabled}
          <a class="btn btn-primary power-action" href={power.href}>{$t("superpowers.open")}</a>
        {:else}
          <p class="power-off">
            <span class="power-off-badge">{$t("superpowers.disabled")}</span>
            {$t("superpowers.disabled_hint")}
          </p>
          <a class="btn btn-secondary power-action" href="/settings?tab=advanced">{$t("superpowers.open_settings")}</a>
        {/if}
      </article>
    {/each}
  </div>
</section>

<style>
  .powers {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .powers-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .powers-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .powers-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .powers-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--space-4);
  }

  .power-card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-5);
    border-radius: 14px;
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .power-card.off :global(.nav-art) {
    filter: grayscale(1);
    opacity: 0.55;
  }

  .power-copy h2 {
    margin: 0 0 var(--space-1);
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .power-copy p,
  .power-off {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
    line-height: 1.45;
  }

  .power-off-badge {
    display: inline-block;
    margin-inline-end: 6px;
    padding: 1px 7px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .power-action {
    margin-top: auto;
  }
</style>
