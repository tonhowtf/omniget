<script lang="ts">
  let { id, name = "", size = 28 }: { id: string; name?: string; size?: number } = $props();

  const COLOR = new Set(["claude", "gemini", "playwright", "microsoft"]);
  const MATCHERS: [RegExp, string][] = [
    [/claude/i, "claude"],
    [/anthropic/i, "anthropic"],
    [/codex|openai|gpt/i, "openai"],
    [/gemini|google/i, "gemini"],
    [/github|copilot/i, "github"],
    [/ollama/i, "ollama"],
    [/openrouter/i, "openrouter"],
    [/playwright/i, "playwright"],
    [/microsoft/i, "microsoft"],
    [/grok|xai/i, "grok"],
  ];

  let brand = $derived(MATCHERS.find(([pattern]) => pattern.test(id))?.[1] ?? null);
  let url = $derived(brand ? `/brands/${brand}.svg` : "");
  let letter = $derived((name || id).trim().charAt(0).toUpperCase());
</script>

<span class="logo" style:--logo-size={`${size}px`} aria-hidden="true">
  {#if brand && COLOR.has(brand)}
    <img src={url} alt="" />
  {:else if brand}
    <span class="mark" style:mask-image={`url(${url})`} style:-webkit-mask-image={`url(${url})`}></span>
  {:else}
    <span class="letter">{letter}</span>
  {/if}
</span>

<style>
  .logo {
    flex: none;
    display: grid;
    place-items: center;
    inline-size: calc(var(--logo-size) + 12px);
    block-size: calc(var(--logo-size) + 12px);
    border: 1px solid var(--content-border);
    border-radius: var(--radius-md);
    background: var(--surface-hi-src, var(--surface-src));
  }
  img, .mark { display: block; inline-size: var(--logo-size); block-size: var(--logo-size); }
  img { object-fit: contain; }
  .mark {
    background: var(--secondary);
    mask-repeat: no-repeat; mask-position: center; mask-size: contain;
    -webkit-mask-repeat: no-repeat; -webkit-mask-position: center; -webkit-mask-size: contain;
  }
  .letter { font-size: calc(var(--logo-size) * 0.6); line-height: 1; font-weight: 600; color: var(--tertiary); }
</style>
