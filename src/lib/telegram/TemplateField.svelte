<script lang="ts">
  import { t } from "$lib/i18n";
  import { TEMPLATE_VARS, renderTemplate, type ChatRef, type TelegramMediaItem } from "./api";

  let {
    value = $bindable(""),
    chat,
    sample = null,
  }: { value: string; chat: ChatRef | null; sample?: TelegramMediaItem | null } = $props();

  const DEFAULT = "{chat}/{file_name}";
  let input: HTMLInputElement | undefined = $state();

  let example = $derived<TelegramMediaItem>(
    sample ?? {
      message_id: 4821,
      file_name: "Aula 03 - Introdução.mp4",
      file_size: 187_000_000,
      media_type: "video",
      date: Math.floor(Date.now() / 1000),
      caption: "Aula 03: introdução ao módulo",
    },
  );
  let preview = $derived(
    chat && value.trim() ? renderTemplate(value.trim(), chat, example) : "",
  );

  function insert(v: string) {
    const tok = `{${v}}`;
    if (!input) {
      value += tok;
      return;
    }
    const s = input.selectionStart ?? value.length;
    const e = input.selectionEnd ?? value.length;
    value = value.slice(0, s) + tok + value.slice(e);
    const at = s + tok.length;
    requestAnimationFrame(() => {
      input?.focus();
      input?.setSelectionRange(at, at);
    });
  }
</script>

<div class="tf field">
  <span class="field-label">{$t("telegram.template.label")}</span>
  <input class="input mono" bind:this={input} bind:value placeholder={$t("telegram.template.placeholder", { example: DEFAULT })} spellcheck="false" />
  <div class="vars">
    {#each TEMPLATE_VARS as v (v)}
      <button type="button" class="var" onclick={() => insert(v)} title={$t(`telegram.template.vars.${v}`)}>{`{${v}}`}</button>
    {/each}
  </div>
  <span class="field-hint">
    {#if preview}
      {$t("telegram.template.preview")} <code>{preview}</code>
    {:else}
      {$t("telegram.template.default_hint")}
    {/if}
  </span>
</div>

<style>
  .tf {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .vars {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .var {
    padding: 2px 7px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    color: var(--text-muted);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    cursor: pointer;
  }

  .var:hover {
    background: var(--fill-2);
    color: var(--text);
  }

  .var:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 1px;
  }

  code {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text);
    word-break: break-all;
  }
</style>
