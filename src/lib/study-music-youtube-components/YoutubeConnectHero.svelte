<script lang="ts">
  import { t } from "$lib/i18n";
  import { goto } from "$app/navigation";

  type Props = {
    status: "missing" | "no_youtube";
    filePath: string;
  };

  let { status, filePath }: Props = $props();

  function openConfigPanel() {
    void goto("/settings?cat=cookies");
  }

  function exploreAnonymously() {
    void goto("/study/music/youtube/explore");
  }
</script>

<section class="connect-hero">
  <div class="hero-card">
    <div class="hero-icon" aria-hidden="true">
      <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
        <path d="M23 12s-3-7-11-7-11 7-11 7 3 7 11 7 11-7 11-7z"/>
        <circle cx="12" cy="12" r="3"/>
      </svg>
    </div>

    <h1 class="hero-title">{$t("study.music.youtube.connect_title")}</h1>
    <p class="hero-body">{$t("study.music.youtube.connect_body")}</p>

    {#if status === "missing"}
      <div class="hint-block">
        <p class="hint-label">{$t("study.music.youtube.connect_step_missing_label")}</p>
        <p class="hint-text">{$t("study.music.youtube.connect_step_missing")}</p>
        {#if filePath}
          <code class="path">{filePath}</code>
        {/if}
      </div>
    {:else}
      <div class="hint-block">
        <p class="hint-label">{$t("study.music.youtube.connect_step_no_youtube_label")}</p>
        <p class="hint-text">{$t("study.music.youtube.connect_step_no_youtube")}</p>
      </div>
    {/if}

    <div class="cta-row">
      <button type="button" class="cta-primary" onclick={openConfigPanel}>
        {$t("study.music.youtube.connect_cta_setup")}
      </button>
      <button type="button" class="cta-secondary" onclick={exploreAnonymously}>
        {$t("study.music.youtube.connect_cta_anon")}
      </button>
    </div>

    <p class="footnote">{$t("study.music.youtube.connect_footnote")}</p>
  </div>
</section>

<style>
  .connect-hero {
    display: flex;
    justify-content: center;
    padding: 40px 20px;
  }
  .hero-card {
    max-width: 560px;
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 14px;
    padding: 32px 28px;
    background: color-mix(in oklab, var(--button) 30%, transparent);
    border: none;
    border-radius: 16px;
  }
  .hero-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 56px;
    height: 56px;
    border-radius: 50%;
    background: rgba(255, 0, 0, 0.12);
    color: #ff4444;
    margin-bottom: 4px;
  }
  .hero-title {
    margin: 0;
    font-size: 22px;
    font-weight: 700;
    color: var(--secondary);
    line-height: 1.25;
  }
  .hero-body {
    margin: 0;
    font-size: 14px;
    color: var(--secondary);
    line-height: 1.5;
  }
  .hint-block {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 14px 16px;
    background: color-mix(in oklab, var(--button) 40%, transparent);
    border-left: 3px solid var(--accent);
    border-radius: 0 8px 8px 0;
  }
  .hint-label {
    margin: 0;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--tertiary);
  }
  .hint-text {
    margin: 0;
    font-size: 13px;
    color: var(--secondary);
    line-height: 1.5;
  }
  .path {
    display: inline-block;
    margin-top: 4px;
    padding: 3px 8px;
    font-family: ui-monospace, "Cascadia Code", "Source Code Pro", monospace;
    font-size: 11px;
    color: var(--tertiary);
    background: color-mix(in oklab, var(--button) 60%, transparent);
    border-radius: 4px;
    word-break: break-all;
  }
  .cta-row {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
    margin-top: 6px;
  }
  .cta-primary {
    padding: 9px 18px;
    background: var(--accent);
    color: var(--on-accent);
    border: 0;
    border-radius: 999px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: filter 120ms;
  }
  .cta-primary:hover { filter: brightness(1.1); }
  .cta-secondary {
    padding: 9px 18px;
    background: transparent;
    color: var(--secondary);
    border: none;
    border-radius: 999px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: border-color 120ms, color 120ms;
  }
  .cta-secondary:hover {
    color: var(--accent);
    border-color: var(--accent);
  }
  .footnote {
    margin: 8px 0 0;
    font-size: 11px;
    color: var(--tertiary);
    line-height: 1.5;
  }
</style>
