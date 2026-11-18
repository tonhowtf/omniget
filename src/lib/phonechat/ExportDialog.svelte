<script lang="ts">
  import { save } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "./api";
  import type { ExportFormat, WaChat } from "./api";
  import WaModal from "./WaModal.svelte";

  let { chat, onclose }: { chat: WaChat; onclose: () => void } = $props();

  const FORMATS: ExportFormat[] = ["txt", "html", "json"];

  let format = $state<ExportFormat>("txt");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let result = $state<{ path: string; count: number } | null>(null);

  async function run() {
    busy = true;
    error = null;
    result = null;
    try {
      const name = await api.exportName(chat.jid, format);
      const dir = await api.defaultDir().catch(() => "");
      const path = await save({
        defaultPath: dir ? `${dir}/${name}` : name,
        filters: [{ name: format.toUpperCase(), extensions: [format] }],
      });
      if (!path) return;
      const count = await api.exportChat(chat.jid, format, path, {
        you: $t("phonechat.you"),
        media_omitted: $t("phonechat.export.media_omitted"),
        attached: $t("phonechat.export.attached"),
        exported_from: $t("phonechat.export.exported_from"),
        messages: $t("phonechat.export.messages"),
        deleted: $t("phonechat.export.deleted"),
        edited: $t("phonechat.export.edited"),
      });
      result = { path, count };
    } catch (e) {
      error = api.errText(e);
    } finally {
      busy = false;
    }
  }
</script>

<WaModal title={$t("phonechat.export.title")} subtitle={chat.name} width={480} onclose={onclose}>
  <div class="formats" role="radiogroup" aria-label={$t("phonechat.export.format")}>
    {#each FORMATS as f (f)}
      <label class="format" class:active={format === f}>
        <input type="radio" name="wa-export-format" value={f} bind:group={format} />
        <span class="format-copy">
          <span class="format-name">{$t(`phonechat.export.formats.${f}`)}</span>
          <span class="format-hint">{$t(`phonechat.export.formats.${f}_hint`)}</span>
        </span>
      </label>
    {/each}
  </div>
  <p class="hint">{$t("phonechat.export.scope_hint", { count: chat.message_count })}</p>
  {#if result}
    <div class="notice notice-success">
      <div class="notice-body"><div class="notice-text">{$t("phonechat.export.done", { count: result.count })}</div></div>
    </div>
  {/if}
  {#if error}
    <div class="notice notice-warning"><div class="notice-body"><div class="notice-text">{error}</div></div></div>
  {/if}

  {#snippet footer()}
    {#if result}
      <button class="btn btn-ghost" onclick={() => void api.reveal(result?.path ?? "").catch(() => {})}>{$t("phonechat.media.reveal")}</button>
      <button class="btn btn-primary" onclick={onclose}>{$t("phonechat.close")}</button>
    {:else}
      <button class="btn btn-ghost" onclick={onclose}>{$t("phonechat.cancel")}</button>
      <button class="btn btn-primary" onclick={run} disabled={busy}>
        {#if busy}<span class="spinner"></span>{/if}
        {$t("phonechat.export.save")}
      </button>
    {/if}
  {/snippet}
</WaModal>

<style>
  .formats {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .format {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: var(--hairline) solid var(--content-border);
    border-radius: var(--radius-md);
    cursor: pointer;
  }

  .format.active {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }

  .format input {
    margin-top: 3px;
  }

  .format-copy {
    display: flex;
    flex-direction: column;
  }

  .format-name {
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
  }

  .format-hint,
  .hint {
    font-size: var(--text-xs);
    color: var(--text-dim);
    line-height: 1.45;
  }

  .hint {
    margin: 0;
  }
</style>
