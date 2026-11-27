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
        { href: "/community", titleKey: "about.card_chat_title", descKey: "about.card_chat_desc", icon: "chat" },
        { href: "/community/roadmap", titleKey: "about.card_roadmap_title", descKey: "about.card_roadmap_desc", icon: "roadmap" },
        { href: "/community/changelog", titleKey: "about.card_changelog_title", descKey: "about.card_changelog_desc", icon: "changelog" },
        { href: "/community/project", titleKey: "about.card_project_title", descKey: "about.card_project_desc", icon: "project" },
        { href: "/community/terms", titleKey: "about.card_terms_title", descKey: "about.card_terms_desc", icon: "terms" },
    ] as const;

    async function openAuthorGithub(e: Event) {
        e.preventDefault();
        await open("https://github.com/tonhowtf");
    }
</script>

<div class="about-overview">
    <header class="surface-card about-hero">
        <img src="/favicon.png" alt="OmniGet" class="about-app-icon" width="68" height="68" draggable="false" />
        <div class="about-identity">
            <div class="about-name-row">
                <h1>OmniGet</h1>
                {#if version}
                    <span class="tag about-version">{$t("about.version")} {version}</span>
                {/if}
            </div>
            <p class="about-tagline">{$t("about.tagline")}</p>
            <p class="about-desc">{$t("about.description")}</p>
            <div class="about-external">
                <a href="https://getomniget.com" target="_blank" rel="noopener" class="btn btn-secondary btn-sm about-ext-link">
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                        <circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>
                    </svg>
                    {$t("about.website_button")}
                </a>
                <a href="https://github.com/tonhowtf/omniget" target="_blank" rel="noopener" class="btn btn-secondary btn-sm about-ext-link">
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                        <path d="M9 19c-5 1.5-5-2.5-7-3m14 6v-3.87a3.37 3.37 0 0 0-.94-2.61c3.14-.35 6.44-1.54 6.44-7A5.44 5.44 0 0 0 20 4.77 5.07 5.07 0 0 0 19.91 1S18.73.65 16 2.48a13.38 13.38 0 0 0-7 0C6.27.65 5.09 1 5.09 1A5.07 5.07 0 0 0 5 4.77a5.44 5.44 0 0 0-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 0 0 9 18.13V22"/>
                    </svg>
                    {$t("about.star_button")}
                </a>
                <a href="https://discord.gg/jgdxyPy7Vn" target="_blank" rel="noopener" class="btn btn-secondary btn-sm about-ext-link">
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                        <path d="M18.9 5.3a16.6 16.6 0 0 0-4.1-1.3 12.2 12.2 0 0 0-.5 1.1 15.4 15.4 0 0 0-4.6 0A12.2 12.2 0 0 0 9.2 4a16.6 16.6 0 0 0-4.1 1.3A17.3 17.3 0 0 0 2 17.2a16.7 16.7 0 0 0 5.1 2.6 12.5 12.5 0 0 0 1.1-1.8 10.8 10.8 0 0 1-1.7-.8l.4-.3a11.9 11.9 0 0 0 10.2 0l.4.3a10.8 10.8 0 0 1-1.7.8 12.5 12.5 0 0 0 1.1 1.8 16.7 16.7 0 0 0 5.1-2.6A17.3 17.3 0 0 0 18.9 5.3zM8.7 14.8c-1 0-1.8-.9-1.8-2s.8-2 1.8-2 1.8.9 1.8 2-.8 2-1.8 2zm6.6 0c-1 0-1.8-.9-1.8-2s.8-2 1.8-2 1.8.9 1.8 2-.8 2-1.8 2z"/>
                    </svg>
                    Discord
                </a>
            </div>
            {#if buildDetails}
                <details class="about-technical"><summary>{$t("about.technical_details")}</summary><span class="about-build">{buildDetails}</span></details>
            {/if}
        </div>
    </header>

    <div class="about-cards">
        {#each cards as card}
            <a href={card.href} class="about-card">
                <span class="about-card-icon" aria-hidden="true">
                    {#if card.icon === "chat"}
                        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>
                    {:else if card.icon === "roadmap"}
                        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="5" height="18" rx="1"/><rect x="10" y="3" width="5" height="12" rx="1"/><rect x="17" y="3" width="4" height="8" rx="1"/></svg>
                    {:else if card.icon === "changelog"}
                        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/></svg>
                    {:else if card.icon === "project"}
                        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/></svg>
                    {:else}
                        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>
                    {/if}
                </span>
                <span class="about-card-copy">
                    <span class="about-card-title">{$t(card.titleKey)}</span>
                    <span class="about-card-desc">{$t(card.descKey)}</span>
                </span>
                <span class="about-card-chevron" aria-hidden="true">›</span>
            </a>
        {/each}
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
        gap: var(--space-5);
    }

    .about-hero {
        display: flex;
        align-items: flex-start;
        gap: var(--space-5);
        padding: var(--space-5);
    }

    .about-app-icon {
        width: 68px;
        height: 68px;
        border-radius: var(--radius-xl);
        object-fit: cover;
        box-shadow: 0 8px 20px rgba(var(--shadow-ink), var(--elev-alpha-2)), 0 0 0 var(--hairline) var(--content-border);
        flex-shrink: 0;
    }

    .about-identity {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        min-width: 0;
        flex: 1;
    }

    .about-name-row {
        display: flex;
        align-items: center;
        gap: var(--space-2);
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
        color: var(--text);
        font-weight: 500;
        margin: 0;
    }

    .about-desc {
        font-size: var(--text-base);
        line-height: 1.6;
        color: var(--text-muted);
        margin: 0;
        max-width: 60ch;
    }

    .about-external {
        display: flex;
        gap: var(--space-2);
        flex-wrap: wrap;
        padding-top: var(--space-1);
    }

    .about-ext-link {
        text-decoration: none;
    }

    .about-technical {
        color: var(--text-dim);
        font-size: var(--text-xs);
        padding-top: var(--space-1);
    }
    .about-technical summary { cursor: pointer; padding: 2px 0; }
    .about-technical summary:focus-visible { outline: var(--focus-ring); }

    .about-build {
        display: block;
        margin-top: 4px;
        font-family: var(--font-mono);
        font-size: var(--text-xs);
        color: var(--text-muted);
        user-select: all;
    }

    .about-cards {
        display: flex;
        flex-direction: column;
        background: var(--surface);
        border-radius: var(--radius-lg);
        box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
        overflow: hidden;
    }

    .about-card {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        padding: var(--space-3) var(--space-4);
        text-decoration: none;
        color: inherit;
        position: relative;
        transition: background var(--duration-fast) var(--ease-out);
    }

    @media (hover: hover) {
        .about-card:hover {
            background: var(--fill-1);
        }
    }

    .about-card + .about-card::before {
        content: "";
        position: absolute;
        top: 0;
        left: var(--space-4);
        right: var(--space-4);
        height: var(--hairline);
        background: var(--separator);
    }

    .about-card-icon {
        width: 32px;
        height: 32px;
        border-radius: var(--radius-md);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        background: var(--surface-hi);
        color: var(--text-muted);
        flex-shrink: 0;
    }

    .about-card-copy {
        display: flex;
        flex-direction: column;
        gap: 2px;
        min-width: 0;
        flex: 1;
    }

    .about-card-title {
        font-size: var(--text-base);
        font-weight: 600;
        color: var(--text);
    }

    .about-card-desc {
        font-size: var(--text-sm);
        line-height: 1.45;
        color: var(--text-muted);
    }

    .about-card-chevron {
        font-size: var(--text-lg);
        color: var(--text-dim);
        line-height: 1;
    }

    .about-footer {
        display: flex;
        align-items: center;
        justify-content: space-between;
        flex-wrap: wrap;
        gap: var(--space-2);
        padding-top: var(--space-2);
        border-top: var(--hairline) solid var(--separator);
    }

    .about-credit {
        font-size: var(--text-xs);
        color: var(--text-muted);
        margin: 0;
    }

    .about-watermark {
        font-family: var(--font-mono);
        font-size: var(--text-xs);
        color: var(--text-dim);
        text-decoration: none;
    }

    .about-watermark:hover {
        color: var(--text);
    }

    @media (max-width: 520px) {
        .about-hero {
            flex-direction: column;
            align-items: center;
            text-align: center;
        }
        .about-name-row,
        .about-external {
            justify-content: center;
        }
    }
</style>
