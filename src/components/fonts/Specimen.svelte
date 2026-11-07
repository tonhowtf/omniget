<script lang="ts">
  import { t } from "$lib/i18n";
  import { ensureFont } from "$lib/fonts/loader";
  import { fontStack, snapWeight, type PreviewFont } from "$lib/fonts/preview";

  let {
    font,
    text,
    size,
    weight,
    italic = false,
    lineHeight = 1.3,
    tracking = 0,
    lang,
    multiline = false,
    label,
  }: {
    font: PreviewFont | null;
    text: string;
    size: number | string;
    weight: number;
    italic?: boolean;
    lineHeight?: number;
    tracking?: number;
    lang: string;
    multiline?: boolean;
    label?: string;
  } = $props();

  let status = $state<"loading" | "ready" | "failed">("ready");
  let effective = $derived(font ? snapWeight(font, weight) : weight);

  $effect(() => {
    const f = font;
    const w = effective;
    const i = italic;
    const l = lang;
    if (!f || f.source !== "catalog") {
      status = "ready";
      return;
    }
    let alive = true;
    status = "loading";
    void ensureFont(f, w, i, l).then((ok) => {
      if (alive) status = ok ? "ready" : "failed";
    });
    return () => {
      alive = false;
    };
  });
</script>

<figure class="ty-spec" class:loading={status === "loading"} aria-label={label}>
  <p
    class="ty-spec-text"
    class:multiline
    style:font-family={fontStack(font)}
    style:font-size={typeof size === "number" ? `${size}px` : size}
    style:font-weight={effective}
    style:font-style={italic ? "italic" : "normal"}
    style:line-height={lineHeight}
    style:letter-spacing="{tracking}em"
  >{text}</p>
  {#if status === "failed"}
    <p class="ty-spec-warn" role="status">{$t("fonts.preview_failed")}</p>
  {/if}
</figure>

<style>
  .ty-spec {
    min-width: 0;
    margin: 0;
  }

  .ty-spec-text {
    margin: 0;
    color: var(--text);
    overflow-wrap: anywhere;
    transition: opacity var(--duration-base) var(--ease-out);
  }

  .ty-spec-text.multiline {
    white-space: pre-wrap;
  }

  .ty-spec.loading .ty-spec-text {
    opacity: 0.45;
  }

  .ty-spec-warn {
    margin: var(--space-2) 0 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  @media (prefers-reduced-motion: reduce) {
    .ty-spec-text {
      transition: none;
    }
  }
</style>
