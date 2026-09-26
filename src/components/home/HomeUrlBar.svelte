<script lang="ts">
  import { readText } from "@tauri-apps/plugin-clipboard-manager";
  import HomeMoreMenu from "$components/home/HomeMoreMenu.svelte";
  import type { MoreAction } from "$lib/home/omnibox-controller";
  import { isUrl } from "$lib/home/omnibox-controller";
  import { shortcut } from "$lib/platform";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";

  let {
    url = $bindable(""),
    busy = false,
    busyLabel = "",
    showSubmit = true,
    inputEl = $bindable(null),
    onInput,
    onSubmit,
    onMore,
  }: {
    url?: string;
    busy?: boolean;
    busyLabel?: string;
    showSubmit?: boolean;
    inputEl?: HTMLInputElement | null;
    onInput: () => void;
    onSubmit: () => void;
    onMore: (action: MoreAction) => void;
  } = $props();

  let hasText = $derived(url.trim().length > 0);

  // One click from the clipboard to a detected link: the value comes before
  // any typing (RCD: value first). Falls back to the web clipboard in dev.
  async function pasteFromClipboard() {
    let text = "";
    try {
      text = (await readText()) ?? "";
    } catch {
      try { text = await navigator.clipboard.readText(); } catch {}
    }
    const trimmed = text.trim();
    if (!trimmed || !trimmed.split(/\s+/).some(isUrl)) {
      showToast("info", $t("home.paste_empty") as string);
      inputEl?.focus();
      return;
    }
    url = trimmed;
    onInput();
    inputEl?.focus();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter" && hasText) {
      e.preventDefault();
      onSubmit();
    } else if (e.key === "Escape" && hasText) {
      url = "";
      onInput();
    }
  }
</script>

<div class="url-bar" class:busy class:filled={hasText}>
  <HomeMoreMenu onPick={onMore} />
  <input
    bind:this={inputEl}
    class="url-input"
    type="text"
    placeholder={$t("omnibox.placeholder")}
    aria-label={$t("omnibox.placeholder")}
    aria-busy={busy}
    bind:value={url}
    oninput={onInput}
    onkeydown={onKey}
    autocomplete="off"
    autocorrect="off"
    autocapitalize="off"
    spellcheck="false"
    enterkeyhint="go"
  />
  {#if hasText}
    <button type="button" class="clear" onclick={() => { url = ""; onInput(); inputEl?.focus(); }} aria-label={$t("common.clear")}>
      <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor" aria-hidden="true"><path d="M8 1.4a6.6 6.6 0 1 0 0 13.2A6.6 6.6 0 0 0 8 1.4zm2.35 3.5a.75.75 0 0 1 1.06 1.06L9.06 8l2.35 2.35a.75.75 0 1 1-1.06 1.06L8 9.06 5.65 11.4a.75.75 0 1 1-1.06-1.06L6.94 8 4.6 5.65A.75.75 0 0 1 5.65 4.6L8 6.94l2.35-2.04z" /></svg>
    </button>
    {#if showSubmit}
    <button type="button" class="go" onclick={onSubmit} aria-label={$t("home.submit")} title={$t("home.submit")}>
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path class="go-arrow" d="M12 5v14M6 13l6 6 6-6" /></svg>
    </button>
    {/if}
  {:else}
    <button type="button" class="paste" onclick={pasteFromClipboard}>
      <span>{$t("home.paste")}</span>
      <kbd>{shortcut("V")}</kbd>
    </button>
  {/if}
  {#if busy}
    <span class="beam" aria-hidden="true"></span>
  {/if}
</div>
{#if busy && busyLabel}
  <p class="busy-label" role="status">{busyLabel}</p>
{/if}

<style>
  .url-bar {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 58px;
    padding: 0 10px 0 11px;
    border-radius: 20px;
    background: var(--surface);
    box-shadow:
      inset 0 0 0 var(--hairline) var(--border-hi),
      0 12px 32px rgba(var(--shadow-ink), var(--elev-alpha-1)),
      0 2px 6px rgba(var(--shadow-ink), calc(var(--elev-alpha-1) * 0.6));
    transition: box-shadow var(--duration-base) var(--ease-out);
  }

  .url-bar:focus-within {
    box-shadow:
      inset 0 0 0 1px color-mix(in srgb, var(--accent) 70%, transparent),
      0 0 0 5px var(--accent-soft),
      0 12px 32px rgba(var(--shadow-ink), var(--elev-alpha-1));
  }

  .url-input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 17px;
    letter-spacing: -0.01em;
    outline: none;
  }

  /* the field's ring is on the whole bar (focus-within), not on the bare input */
  .url-input:focus-visible {
    outline: none;
  }

  .url-input::placeholder {
    color: var(--text-dim);
  }

  .clear {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: var(--text-faint);
    cursor: pointer;
  }

  .clear:hover { color: var(--text-muted); }

  /* the page's one call to action once there is a link */
  .go {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    flex-shrink: 0;
    border: none;
    border-radius: 50%;
    background: var(--cta);
    color: var(--on-cta);
    cursor: pointer;
    box-shadow: 0 4px 14px var(--accent-glow);
    animation: pop-in 180ms var(--ease-out);
    transition: background var(--duration-fast) var(--ease-out), transform var(--duration-fast) var(--ease-out);
  }

  .go:hover { background: var(--cta-hover); }
  .go:active { background: var(--cta-press); transform: scale(0.95); }

  /* the arrow draws itself in (21st Placeholders And Vanish Input, #1420) */
  .go-arrow {
    stroke-dasharray: 40;
    stroke-dashoffset: 40;
    animation: draw 260ms 60ms var(--ease-out) forwards;
  }

  .paste {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    padding: 0 8px 0 14px;
    flex-shrink: 0;
    border: none;
    border-radius: 999px;
    background: var(--fill-1);
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }

  .paste:hover { background: var(--fill-2); }

  .paste kbd {
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 6px;
    background: var(--surface);
    color: var(--text-dim);
    box-shadow: inset 0 0 0 var(--hairline) var(--border);
  }

  .go:focus-visible,
  .paste:focus-visible,
  .clear:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  /* checking: a light that runs around the field (21st Border Beam, #1268) */
  .beam {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    padding: 1.5px;
    pointer-events: none;
    background: conic-gradient(from var(--beam-angle, 0deg), transparent 0 70%, var(--accent) 85%, transparent 100%);
    -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
    mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
    -webkit-mask-composite: xor;
    mask-composite: exclude;
    animation: beam 1.4s linear infinite;
  }

  @property --beam-angle {
    syntax: "<angle>";
    initial-value: 0deg;
    inherits: false;
  }

  /* shimmering status text (21st Text Shimmer, #1641) */
  .busy-label {
    margin: 10px 0 0;
    text-align: center;
    font-size: var(--text-sm);
    font-weight: 500;
    color: transparent;
    background: linear-gradient(90deg, var(--text-dim) 0%, var(--text-dim) 40%, var(--text) 50%, var(--text-dim) 60%, var(--text-dim) 100%);
    background-size: 250% 100%;
    -webkit-background-clip: text;
    background-clip: text;
    animation: shimmer 1.6s linear infinite;
  }

  @keyframes beam { to { --beam-angle: 360deg; } }
  @keyframes shimmer { from { background-position: 100% 0; } to { background-position: -150% 0; } }
  @keyframes draw { to { stroke-dashoffset: 0; } }
  @keyframes pop-in { from { transform: scale(0.6); opacity: 0; } to { transform: scale(1); opacity: 1; } }

  @media (prefers-reduced-motion: reduce) {
    .beam { animation: none; background: none; box-shadow: inset 0 0 0 1.5px var(--accent); }
    .busy-label { animation: none; color: var(--text-dim); background: none; }
    .go, .go-arrow { animation: none; stroke-dashoffset: 0; }
  }
</style>
