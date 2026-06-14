<script lang="ts">
    import { t } from "$lib/i18n";
    import { getVersion } from "@tauri-apps/api/app";
    import { open } from "@tauri-apps/plugin-shell";
    import { BUILD_INFO } from "$lib/build-info";

    let version = $state("");

    $effect(() => {
        getVersion().then(v => { version = v; }).catch(() => {});
    });

    const buildDetails = $derived(
        [BUILD_INFO.commitShort, BUILD_INFO.branch, BUILD_INFO.date]
            .filter((part) => part && part !== "unknown")
            .join(" · ")
    );

    const cards = [
        { href: "/about/changelog", titleKey: "about.card_changelog_title", descKey: "about.card_changelog_desc" },
        { href: "/about/project", titleKey: "about.card_project_title", descKey: "about.card_project_desc" },
        { href: "/about/terms", titleKey: "about.card_terms_title", descKey: "about.card_terms_desc" },
    ] as const;

    async function openAuthorGithub(e: Event) {
        e.preventDefault();
        await open("https://github.com/tonhowtf");
    }
</script>

<div class="about-overview">
    <header class="about-hero">
        <img src="/favicon.png" alt="" class="about-app-icon" width="64" height="64" draggable="false" />
        <div class="about-identity">
            <h1>OmniGet</h1>
            {#if version}
                <span class="about-version">{$t("about.version")} {version}</span>
            {/if}
            <p class="about-tagline">{$t("about.tagline")}</p>
            <p class="about-desc">{$t("about.description")}</p>
            {#if buildDetails}
                <span class="about-build">{buildDetails}</span>
            {/if}
        </div>
    </header>

    <div class="about-cards">
        {#each cards as card}
            <a href={card.href} class="about-card">
                <span class="about-card-title">{$t(card.titleKey)}</span>
                <span class="about-card-desc">{$t(card.descKey)}</span>
                <span class="about-card-chevron" aria-hidden="true">›</span>
            </a>
        {/each}
    </div>

    <div class="about-external">
        <a href="https://github.com/tonhowtf/omniget" target="_blank" rel="noopener" class="about-ext-link">
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M9 19c-5 1.5-5-2.5-7-3m14 6v-3.87a3.37 3.37 0 0 0-.94-2.61c3.14-.35 6.44-1.54 6.44-7A5.44 5.44 0 0 0 20 4.77 5.07 5.07 0 0 0 19.91 1S18.73.65 16 2.48a13.38 13.38 0 0 0-7 0C6.27.65 5.09 1 5.09 1A5.07 5.07 0 0 0 5 4.77a5.44 5.44 0 0 0-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 0 0 9 18.13V22"/>
            </svg>
            {$t("about.star_button")}
        </a>
        <a href="https://discord.gg/jgdxyPy7Vn" target="_blank" rel="noopener" class="about-ext-link">
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M18.9 5.3a16.6 16.6 0 0 0-4.1-1.3 12.2 12.2 0 0 0-.5 1.1 15.4 15.4 0 0 0-4.6 0A12.2 12.2 0 0 0 9.2 4a16.6 16.6 0 0 0-4.1 1.3A17.3 17.3 0 0 0 2 17.2a16.7 16.7 0 0 0 5.1 2.6 12.5 12.5 0 0 0 1.1-1.8 10.8 10.8 0 0 1-1.7-.8l.4-.3a11.9 11.9 0 0 0 10.2 0l.4.3a10.8 10.8 0 0 1-1.7.8 12.5 12.5 0 0 0 1.1 1.8 16.7 16.7 0 0 0 5.1-2.6A17.3 17.3 0 0 0 18.9 5.3zM8.7 14.8c-1 0-1.8-.9-1.8-2s.8-2 1.8-2 1.8.9 1.8 2-.8 2-1.8 2zm6.6 0c-1 0-1.8-.9-1.8-2s.8-2 1.8-2 1.8.9 1.8 2-.8 2-1.8 2z"/>
            </svg>
            Discord
        </a>
    </div>

    <footer class="about-footer">
        <p class="about-credit">{$t("about.credit")}</p>
        <a href="https://github.com/tonhowtf" class="about-watermark" onclick={openAuthorGithub} title="@tonhowtf">
            @tonhowtf
        </a>
    </footer>
</div>

<style>
    .about-overview {
        display: flex;
        flex-direction: column;
        gap: calc(var(--padding) * 2);
    }

    .about-hero {
        display: flex;
        align-items: flex-start;
        gap: var(--space-4);
    }

    .about-app-icon {
        width: 64px;
        height: 64px;
        border-radius: var(--radius-lg);
        object-fit: cover;
        box-shadow: var(--elev-1);
        flex-shrink: 0;
    }

    .about-identity {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
        min-width: 0;
    }

    .about-identity h1 {
        font-family: var(--font-display);
        font-size: var(--text-display);
        line-height: var(--leading-display);
        font-weight: 600;
        letter-spacing: -0.03em;
        margin: 0;
    }

    .about-tagline {
        font-size: var(--text-md);
        color: var(--text-muted);
        margin: 0;
    }

    .about-desc {
        font-size: var(--text-sm);
        color: var(--text);
        margin: 0;
        max-width: 420px;
        line-height: 1.5;
    }

    .about-version {
        font-size: var(--text-xs);
        color: var(--text-dim);
        width: fit-content;
    }

    .about-build {
        font-family: var(--font-mono);
        font-size: var(--text-xs);
        color: var(--text-dim);
        opacity: 0.75;
        letter-spacing: 0.3px;
        user-select: all;
    }

    .about-cards {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: var(--space-3);
    }

    .about-card {
        position: relative;
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
        padding: var(--space-4);
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius-md);
        text-decoration: none;
        color: inherit;
        transition: background var(--duration-fast) var(--ease-out), border-color var(--duration-fast) var(--ease-out);
    }

    @media (hover: hover) {
        .about-card:hover {
            background: var(--surface-hi);
            border-color: color-mix(in srgb, var(--accent) 25%, var(--border));
        }
    }

    .about-card-title {
        font-size: var(--text-sm);
        font-weight: 600;
        color: var(--text);
    }

    .about-card-desc {
        font-size: var(--text-xs);
        color: var(--text-muted);
        line-height: 1.45;
        padding-right: var(--space-4);
    }

    .about-card-chevron {
        position: absolute;
        top: var(--space-4);
        right: var(--space-3);
        font-size: 18px;
        color: var(--text-dim);
        line-height: 1;
    }

    .about-external {
        display: flex;
        gap: var(--space-3);
        flex-wrap: wrap;
    }

    .about-ext-link {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-2) var(--space-4);
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        color: var(--text);
        font-size: var(--text-sm);
        text-decoration: none;
        transition: background var(--duration-fast) var(--ease-out);
    }

    @media (hover: hover) {
        .about-ext-link:hover {
            background: var(--surface-hi);
        }
    }

    .about-footer {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
        padding-top: var(--space-2);
        border-top: 1px solid var(--border);
    }

    .about-credit {
        font-size: var(--text-xs);
        color: var(--text-muted);
        margin: 0;
    }

    .about-watermark {
        font-size: var(--text-xs);
        color: var(--text-dim);
        opacity: 0.6;
        text-decoration: none;
        width: fit-content;
        transition: opacity var(--duration-fast) var(--ease-out);
    }

    @media (hover: hover) {
        .about-watermark:hover {
            opacity: 1;
        }
    }

    @media (max-width: 520px) {
        .about-cards {
            grid-template-columns: 1fr;
        }

        .about-hero {
            flex-direction: column;
            align-items: center;
            text-align: center;
        }

        .about-desc {
            max-width: none;
        }
    }
</style>
