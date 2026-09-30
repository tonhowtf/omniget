<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import TgIcon from "./TgIcon.svelte";
  import OutputDir from "./OutputDir.svelte";
  import { tg, errText, isUnknownCommand, type TelegramChat } from "./api";
  import { chatsState, loadChats } from "./session.svelte";

  let { outputDir = $bindable("") }: { outputDir: string } = $props();

  const FORMATS = ["html", "json"] as const;

  let chatKey = $state("");
  let format = $state<(typeof FORMATS)[number]>("html");
  let withMedia = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let resultPath = $state<string | null>(null);

  let chat = $derived<TelegramChat | undefined>(chatsState.chats.find((c) => `${c.chat_type}:${c.id}` === chatKey));

  onMount(() => {
    if (!chatsState.loaded) void loadChats(true);
  });

  async function run() {
    if (!chat) return;
    busy = true;
    error = null;
    resultPath = null;
    try {
      // The export runs in the background; its end arrives as an event.
      let exportId: number | null = null;
      const done = new Promise<{ success: boolean; path?: string; error?: string | null }>((resolve) => {
        const off = listen<{ export_id: number; success: boolean; path?: string; error?: string | null }>(
          "telegram-export-complete",
          (e) => {
            if (exportId === null || e.payload.export_id === exportId) {
              void off.then((f) => f());
              resolve(e.payload);
            }
          },
        );
      });
      const r = await tg.exportChat(chat, format, withMedia, outputDir);
      exportId = r?.export_id ?? null;
      const end = await done;
      if (!end.success) throw new Error(end.error ?? "export failed");
      resultPath = end.path ?? outputDir;
    } catch (e) {
      error = isUnknownCommand(e) ? ($t("telegram.common.not_ready") as string) : errText(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="export">
  <header>
    <h3>{$t("telegram.export.title")}</h3>
    <p>{$t("telegram.export.hint")}</p>
  </header>

  <div class="group">
    <div class="group-row">
      <div class="group-row-content"><span class="group-row-title">{$t("telegram.export.chat")}</span></div>
      <select class="input sel" bind:value={chatKey} aria-label={$t("telegram.export.chat")}>
        <option value="" disabled>{chatsState.loading ? $t("telegram.common.loading") : $t("telegram.export.pick_chat")}</option>
        {#each chatsState.chats as c (`${c.chat_type}:${c.id}`)}
          <option value={`${c.chat_type}:${c.id}`}>{c.title}</option>
        {/each}
      </select>
    </div>
    <div class="group-row">
      <div class="group-row-content"><span class="group-row-title">{$t("telegram.export.format")}</span></div>
      <div class="segmented" role="radiogroup" aria-label={$t("telegram.export.format")}>
        {#each FORMATS as f (f)}
          <button class="segmented-btn" role="radio" aria-checked={format === f} onclick={() => (format = f)}>{$t(`telegram.export.formats.${f}`)}</button>
        {/each}
      </div>
    </div>
    <div class="group-row">
      <div class="group-row-content">
        <span class="group-row-title">{$t("telegram.export.with_media")}</span>
        <span class="group-row-sub">{$t("telegram.export.with_media_hint")}</span>
      </div>
      <button class="toggle" role="switch" aria-checked={withMedia} aria-label={$t("telegram.export.with_media")} onclick={() => (withMedia = !withMedia)}><span class="toggle-knob"></span></button>
    </div>
    <div class="group-row">
      <div class="group-row-content"><span class="group-row-title">{$t("telegram.common.output_folder")}</span></div>
      <OutputDir bind:value={outputDir} />
    </div>
  </div>

  <div class="btn-row end">
    <button class="btn btn-primary" onclick={run} disabled={!chat || !outputDir || busy}>
      {#if busy}<span class="spinner"></span>{:else}<TgIcon name="save" />{/if}
      {busy ? $t("telegram.export.running") : $t("telegram.export.start")}
    </button>
  </div>
  {#if error}<p class="field-error">{error}</p>{/if}
  {#if resultPath}
    <div class="notice notice-success">
      <TgIcon name="check" size={16} />
      <div class="notice-body">
        <div class="notice-title">{$t("telegram.export.done")}</div>
        <div class="notice-text path">{resultPath}</div>
      </div>
      <button class="btn btn-ghost btn-sm" onclick={() => void invoke("reveal_file", { path: resultPath }).catch(() => {})}>
        <TgIcon name="folder" /> {$t("telegram.common.open_folder")}
      </button>
    </div>
  {/if}
</section>

<style>
  .export {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 640px;
  }

  header h3 {
    margin: 0 0 2px;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  header p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: 1.5;
  }

  .sel {
    max-width: 280px;
  }

  .path {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    word-break: break-all;
  }
</style>
