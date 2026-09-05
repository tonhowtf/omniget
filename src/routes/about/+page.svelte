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
            <div class="about-name-row">
                <h1>OmniGet</h1>
                {#if version}
                    <span class="tag about-version">{$t("about.version")} {version}</span>
                {/if}
            </div>
            <p class="about-tagline">{$t("about.tagline")}</p>
            <p class="about-desc">{$t("about.description")}</p>
            {#if buildDetails}
                <span class="about-build">{buildDetails}</span>
            {/if}
        </div>
    </header>

    <div class="about-cards">
        {#each cards as card}
            <a href={card.href} class="surface-card interactive about-card">
                <span class="list-row">
                    <span class="list-row-content">
                        <span class="list-row-title">{$t(card.titleKey)}</span>
                        <span class="list-row-sub about-card-desc">{$t(card.descKey)}</span>
                    </span>
                    <span class="list-row-trailing about-card-chevron" aria-hidden="true">›</span>
                </span>
            </a>
        {/each}
    </div>

    <div class="about-external">
        <a href="https://github.com/tonhowtf/omniget" target="_blank" rel="noopener" class="btn about-ext-link">
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M9 19c-5 1.5-5-2.5-7-3m14 6v-3.87a3.37 3.37 0 0 0-.94-2.61c3.14-.35 6.44-1.54 6.44-7A5.44 5.44 0 0 0 20 4.77 5.07 5.07 0 0 0 19.91 1S18.73.65 16 2.48a13.38 13.38 0 0 0-7 0C6.27.65 5.09 1 5.09 1A5.07 5.07 0 0 0 5 4.77a5.44 5.44 0 0 0-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 0 0 9 18.13V22"/>
            </svg>
            {$t("about.star_button")}
        </a>
        <a href="https://discord.gg/jgdxyPy7Vn" target="_blank" rel="noopener" class="btn about-ext-link">
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M18.9 5.3a16.6 16.6 0 0 0-4.1-1.3 12.2 12.2 0 0 0-.5 1.1 15.4 15.4 0 0 0-4.6 0A12.2 12.2 0 0 0 9.2 4a16.6 16.6 0 0 0-4.1 1.3A17.3 17.3 0 0 0 2 17.2a16.7 16.7 0 0 0 5.1 2.6 12.5 12.5 0 0 0 1.1-1.8 10.8 10.8 0 0 1-1.7-.8l.4-.3a11.9 11.9 0 0 0 10.2 0l.4.3a10.8 10.8 0 0 1-1.7.8 12.5 12.5 0 0 0 1.1 1.8 16.7 16.7 0 0 0 5.1-2.6A17.3 17.3 0 0 0 18.9 5.3zM8.7 14.8c-1 0-1.8-.9-1.8-2s.8-2 1.8-2 1.8.9 1.8 2-.8 2-1.8 2zm6.6 0c-1 0-1.8-.9-1.8-2s.8-2 1.8-2 1.8.9 1.8 2-.8 2-1.8 2z"/>
            </svg>
            Discord
        </a>
    </div>

    <footer class="about-footer">
        <hr class="separator" />
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
        gap: var(--space-6);
    }

    .about-hero {
        display: flex;
        align-items: flex-start;
        gap: var(--space-5);
        padding: var(--space-2) 0;
    }

    .about-app-icon {
        width: 72px;
        height: 72px;
        border-radius: 17px;
        object-fit: cover;
        box-shadow: 0 8px 20px rgba(var(--shadow-ink), var(--elev-alpha-2)), 0 0 0 var(--hairline) var(--content-border);
        flex-shrink: 0;
    }

    .about-identity {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        min-width: 0;
    }

    .about-name-row {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        flex-wrap: wrap;
    }

    .about-identity h1 {
        font-family: var(--font-display);
        font-size: var(--text-2xl);
        line-height: var(--leading-2xl);
        font-weight: 700;
        letter-spacing: var(--track-tight);
        margin: 0;
    }

    .about-tagline {
        font-size: var(--text-md);
        line-height: var(--leading-md);
        color: var(--text-muted);
        margin: 0;
    }

    .about-desc {
        font-size: var(--text-base);
        line-height: var(--leading-base);
        color: var(--text-muted);
        margin: 0;
        max-width: 60ch;
    }

    .about-build {
        font-family: var(--font-mono);
        font-size: var(--text-xs);
        color: var(--text-dim);
        user-select: all;
    }

    .about-cards {
        display: flex;
        flex-direction: column;
        gap: 0;
        background: var(--surface);
        border-radius: var(--radius-lg);
        box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
        overflow: hidden;
    }

    .about-card {
        display: block;
        text-decoration: none;
        color: inherit;
        border-radius: 0;
        box-shadow: none;
        position: relative;
    }

    .about-card + .about-card::before {
        content: "";
        position: absolute;
        top: 0;
        left: var(--space-4);
        right: 0;
        height: var(--hairline);
        background: var(--separator);
    }

    .about-card .list-row {
        padding: var(--space-3) var(--space-4);
        border-radius: 0;
    }

    .about-card-desc {
        white-space: normal;
    }

    .about-card-chevron {
        font-size: var(--text-lg);
        line-height: 1;
    }

    .about-external {
        display: flex;
        gap: var(--space-2);
        flex-wrap: wrap;
    }

    .about-ext-link {
        text-decoration: none;
    }

    .about-footer {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
    }

    .about-footer .separator {
        margin-bottom: var(--space-3);
    }

    .about-credit {
        font-size: var(--text-xs);
        line-height: var(--leading-xs);
        color: var(--text-muted);
        margin: 0;
    }

    .about-watermark {
        font-size: var(--text-xs);
        color: var(--text-dim);
        text-decoration: none;
        width: fit-content;
        transition: color var(--duration-fast) var(--ease-out);
    }

    @media (hover: hover) {
        .about-watermark:hover {
            color: var(--text);
        }
    }

    @media (prefers-reduced-motion: reduce) {
        .about-watermark {
            transition: none;
        }
    }

    @media (max-width: 520px) {
        .about-hero {
            flex-direction: column;
            align-items: center;
            text-align: center;
        }

        .about-name-row {
            justify-content: center;
        }
    }
</style>
