<script lang="ts">
  import { page } from "$app/stores";
  import { t } from "$lib/i18n";

  type Item = { href: string; label: string; icon: string };
  type Section = { title: string; items: Item[] };

  const sections = $derived([
    {
      title: $t("study.anki.sidebar.sec_study"),
      items: [
        { href: "/study/anki", label: $t("study.anki.sidebar.dashboard"), icon: "M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" },
        { href: "/study/anki/decks", label: $t("study.anki.sidebar.decks"), icon: "M4 4h16v4H4z M4 10h16v4H4z M4 16h16v4H4z" },
        { href: "/study/anki/browse", label: $t("study.anki.sidebar.browse"), icon: "M11 4a7 7 0 1 0 4.9 12L21 21 M11 4a7 7 0 0 1 7 7" },
      ],
    },
    {
      title: $t("study.anki.sidebar.sec_content"),
      items: [
        { href: "/study/anki/notetypes", label: $t("study.anki.sidebar.notetypes"), icon: "M4 4h6v6H4z M14 4h6v6h-6z M4 14h6v6H4z M14 14h6v6h-6z" },
        { href: "/study/anki/tags", label: $t("study.anki.sidebar.tags"), icon: "M20 12L12 20l-9-9V3h8z M7 7h.01" },
        { href: "/study/anki/media", label: $t("study.anki.sidebar.media"), icon: "M21 15V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10 M21 15l-5 6 M14 8a2 2 0 1 0 0-4 2 2 0 0 0 0 4z M3 16l-5-5 8 8" },
        { href: "/study/anki/import", label: $t("study.anki.sidebar.import"), icon: "M12 3v12 M7 10l5 5 5-5 M5 21h14" },
      ],
    },
    {
      title: $t("study.anki.sidebar.sec_analysis"),
      items: [
        { href: "/study/anki/stats", label: $t("study.anki.sidebar.stats"), icon: "M3 21h18 M6 17V9 M11 17V5 M16 17v-7 M21 17V13" },
      ],
    },
    {
      title: $t("study.anki.sidebar.sec_system"),
      items: [
        { href: "/study/anki/sync", label: $t("study.anki.sidebar.sync"), icon: "M21 12a9 9 0 0 1-15 6.7L3 16 M3 12a9 9 0 0 1 15-6.7L21 8 M21 4v4h-4 M3 20v-4h4" },
        { href: "/study/anki/settings", label: $t("study.anki.sidebar.settings"), icon: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M19 12a7 7 0 1 1-14 0 7 7 0 0 1 14 0z" },
      ],
    },
  ]);

  const currentPath = $derived($page.url.pathname);

  function isActive(href: string): boolean {
    if (href === "/study/anki") return currentPath === "/study/anki";
    return currentPath === href || currentPath.startsWith(href + "/");
  }
</script>

<aside class="anki-sidebar" aria-label={$t("study.anki.sidebar.nav_aria")}>
  <header class="brand">
    <span class="brand-mark" aria-hidden="true">
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M4 5h16v14H4z" />
        <path d="M4 9h16" />
      </svg>
    </span>
    <span class="brand-name">Anki</span>
  </header>

  {#each sections as section (section.title)}
    <nav class="section" aria-label={section.title}>
      <span class="eyebrow">{section.title}</span>
      <ul>
        {#each section.items as item (item.href)}
          {@const active = isActive(item.href)}
          <li>
            <a href={item.href} class="item" class:active aria-current={active ? "page" : undefined}>
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d={item.icon} />
              </svg>
              <span class="item-label">{item.label}</span>
            </a>
          </li>
        {/each}
      </ul>
    </nav>
  {/each}
</aside>

<style>
  .anki-sidebar {
    width: 220px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding: var(--space-4) var(--space-3);
    background: var(--surface);
    border: none;
    border-radius: var(--radius-md);
    position: sticky;
    top: var(--space-3);
    align-self: flex-start;
    max-height: calc(100vh - var(--space-7));
    overflow-y: auto;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px var(--space-2) var(--space-2);
    border-bottom: none;
    margin-bottom: var(--space-2);
  }

  .brand-mark {
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--cta);
    color: var(--on-cta);
    border-radius: var(--radius-xs);
  }

  :global([data-theme^="eink-"]) .brand-mark {
    background: var(--text);
    color: var(--bg);
  }

  .brand-name {
    font-family: var(--font-display);
    font-size: var(--text-md);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text);
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .eyebrow {
    padding: 0 var(--space-2);
    margin-bottom: 0;
    font-family: var(--font-body);
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-dim);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 7px var(--space-2);
    border-radius: var(--radius-xs);
    color: var(--text-muted);
    font-size: var(--text-sm);
    text-decoration: none;
    transition: background var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }

  .item:hover {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    color: var(--text);
  }

  .item.active {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    font-weight: 600;
  }

  .item.active svg {
    color: var(--accent);
  }

  .item svg {
    flex-shrink: 0;
    opacity: 0.85;
  }

  .item-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (max-width: 1080px) {
    .anki-sidebar {
      width: 180px;
    }
  }

  @media (max-width: 860px) {
    .anki-sidebar {
      width: 56px;
      padding: var(--space-3) var(--space-2);
    }
    .brand-name,
    .item-label,
    .eyebrow {
      display: none;
    }
    .brand {
      justify-content: center;
      border-bottom: none;
      padding: 0 0 var(--space-2);
    }
    .item {
      justify-content: center;
      padding: var(--space-2);
    }
  }
</style>
