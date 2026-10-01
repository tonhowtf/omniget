<script lang="ts">
  import { t } from "$lib/i18n";
  import TgIcon from "./TgIcon.svelte";
  import { net, errText, NEEDS_CLASH, type AddLinksResult } from "./api";
  import { refreshNet } from "./net.svelte";

  let { onAdded, rows = 5 }: { onAdded?: (r: AddLinksResult) => void; rows?: number } = $props();

  let text = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let result = $state<AddLinksResult | null>(null);

  let clashOnly = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const s of result?.unsupported ?? []) counts.set(s, (counts.get(s) ?? 0) + 1);
    return [...counts.entries()];
  });
  let needsClash = $derived((result?.unsupported ?? []).some((s) => NEEDS_CLASH.has(s)));
  let okCount = $derived((result?.added ?? []).length);

  async function add() {
    if (!text.trim()) return;
    busy = true;
    error = null;
    result = null;
    try {
      result = await net.addLinks(text);
      if (result.added.length) text = "";
      await refreshNet();
      onAdded?.(result);
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  async function pasteClipboard() {
    try {
      const clip = await navigator.clipboard.readText();
      if (clip) text = text ? `${text}\n${clip}` : clip;
    } catch {
    }
  }
</script>

<div class="paste">
  <textarea
    class="input mono"
    {rows}
    bind:value={text}
    spellcheck="false"
    autocomplete="off"
    placeholder={$t("telegram.net.paste_placeholder")}
    aria-label={$t("telegram.net.paste_title")}
  ></textarea>
  <div class="row">
    <span class="hint">{$t("telegram.net.paste_formats")}</span>
    <button class="btn btn-ghost btn-sm" onclick={pasteClipboard}>{$t("telegram.net.paste_clipboard")}</button>
    <button class="btn btn-primary btn-sm" onclick={add} disabled={busy || !text.trim()}>
      {#if busy}<span class="spinner" aria-hidden="true"></span>{:else}<TgIcon name="link" />{/if}
      {busy ? $t("telegram.net.paste_testing") : $t("telegram.net.paste_add")}
    </button>
  </div>
  {#if error}<p class="field-error">{error}</p>{/if}
  {#if result}
    <div class="result">
      <span class:ok={okCount > 0}>
        {okCount > 0 ? $t("telegram.net.paste_added", { count: okCount }) : $t("telegram.net.paste_none")}
      </span>
      {#if clashOnly.length}
        <span class="unsup">
          {$t("telegram.net.paste_unsupported")}
          {#each clashOnly as [scheme, n] (scheme)}<span class="tag tag-warning">{scheme}{n > 1 ? ` ×${n}` : ""}</span>{/each}
        </span>
      {/if}
    </div>
    {#if needsClash}
      <div class="notice notice-info">
        <TgIcon name="info" size={16} />
        <div class="notice-body">
          <div class="notice-text">{$t("telegram.net.needs_clash")}</div>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .paste {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  textarea {
    width: 100%;
    resize: vertical;
    min-height: 88px;
    padding: var(--space-2) var(--space-3);
    line-height: 1.5;
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .hint {
    flex: 1;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .result {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .result .ok {
    color: var(--success);
    font-weight: 600;
  }

  .unsup {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
  }
</style>
