<script lang="ts">
  // Create or edit an auto-save rule ("save new videos from this chat").
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgModal from "./TgModal.svelte";
  import TgIcon from "./TgIcon.svelte";
  import OutputDir from "./OutputDir.svelte";
  import ChatSelect from "./ChatSelect.svelte";
  import MediaFilterFields from "./MediaFilterFields.svelte";
  import TemplateField from "./TemplateField.svelte";
  import { tg2, newScanId, type TelegramChat, type WatchRule, type WatchRuleDraft } from "./api";
  import { emptyFilter, filterFromForm, toMb, type FilterForm } from "./filters";
  import { chatRef, expireScan, failText, finishScan, loadWatches, trackScan } from "./session.svelte";

  let {
    rule = null,
    presetChat = null,
    onclose,
    onsaved,
  }: {
    rule?: WatchRule | null;
    presetChat?: TelegramChat | null;
    onclose: () => void;
    onsaved?: (r: WatchRule | null) => void;
  } = $props();

  const init = () => {
    if (rule) {
      return {
        chat: chatRef(rule.chatId, rule.chatType, rule.chatTitle),
        form: {
          types: [...rule.mediaTypes],
          minMb: toMb(rule.minSize),
          maxMb: toMb(rule.maxSize),
          keyword: rule.keyword ?? "",
          extIn: (rule.extInclude ?? []).join(", "),
          extOut: (rule.extExclude ?? []).join(", "),
        } satisfies FilterForm,
        outputDir: rule.outputDir ?? "",
        template: rule.template ?? "",
        dedupe: rule.dedupe ?? true,
        dedupeRaw: rule.dedupe ?? null,
        addToWatch: rule.addToWatch ?? false,
      };
    }
    return {
      chat: presetChat,
      form: emptyFilter(["video"]),
      outputDir: getSettings()?.download?.default_output_dir ?? "",
      template: "",
      dedupe: true,
      dedupeRaw: null as boolean | null,
      addToWatch: false,
    };
  };
  const start = init();

  let chat = $state<TelegramChat | null>(start.chat);
  let form = $state<FilterForm>(start.form);
  let outputDir = $state(start.outputDir);
  let template = $state(start.template);
  let dedupe = $state(start.dedupe);
  // Untouched, a rule keeps following the global "skip duplicates" setting (null).
  let dedupeTouched = $state(false);
  let addToWatch = $state(start.addToWatch);
  let backfillOn = $state(false);
  let backfill = $state(20);
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function submit() {
    if (!chat) return;
    busy = true;
    error = null;
    const f = filterFromForm(form);
    const draft: WatchRuleDraft = {
      chatId: chat.id,
      chatType: chat.chat_type,
      chatTitle: chat.title,
      enabled: rule?.enabled ?? true,
      mediaTypes: f.mediaTypes ?? [],
      minSize: f.minSize ?? null,
      maxSize: f.maxSize ?? null,
      keyword: f.keyword ?? null,
      extInclude: f.extInclude ?? [],
      extExclude: f.extExclude ?? [],
      outputDir: outputDir || null,
      template: template.trim() || null,
      dedupe: dedupeTouched ? dedupe : start.dedupeRaw,
      addToWatch,
    };
    const n = backfillOn ? Math.max(1, Math.floor(backfill)) : 0;
    const scanId = !rule && n ? newScanId() : undefined;
    if (scanId) trackScan(scanId, chat.id, chat.title, "backfill");
    try {
      let saved: WatchRule | null = null;
      if (rule) await tg2.watchUpdate(rule.id, draft);
      else saved = await tg2.watchAdd(draft, n, scanId);
      if (scanId) expireScan(scanId);
      showToast("success", $t(rule ? "telegram.watch.updated" : "telegram.watch.created", { chat: chat.title }) as string);
      void loadWatches();
      onsaved?.(saved);
      onclose();
    } catch (e) {
      if (scanId) finishScan(scanId);
      error = failText(e);
    } finally {
      busy = false;
    }
  }
</script>

<TgModal
  title={rule ? $t("telegram.watch.edit_title") : $t("telegram.watch.new_title")}
  subtitle={$t("telegram.watch.editor_hint")}
  width={600}
  {onclose}
>
  <div class="field">
    <span class="field-label">{$t("telegram.watch.chat")}</span>
    {#if rule}
      <div class="fixed-chat">{chat?.title}</div>
    {:else}
      <ChatSelect bind:value={chat} label={$t("telegram.watch.chat")} />
    {/if}
  </div>

  <MediaFilterFields bind:form />

  <div class="field">
    <span class="field-label">{$t("telegram.common.output_folder")}</span>
    <OutputDir bind:value={outputDir} />
  </div>

  <TemplateField bind:value={template} {chat} />

  <div class="group">
    <div class="group-row">
      <div class="group-row-content">
        <span class="group-row-title">{$t("telegram.dedupe.option")}</span>
        <span class="group-row-sub">{$t("telegram.dedupe.option_hint")}</span>
      </div>
      <button class="toggle" role="switch" aria-checked={dedupe} aria-label={$t("telegram.dedupe.option")} onclick={() => { dedupe = !dedupe; dedupeTouched = true; }}><span class="toggle-knob"></span></button>
    </div>
    <div class="group-row">
      <div class="group-row-content">
        <span class="group-row-title">{$t("telegram.watch_lib.rule_option")}</span>
        <span class="group-row-sub">{$t("telegram.watch_lib.rule_option_hint")}</span>
      </div>
      <button class="toggle" role="switch" aria-checked={addToWatch} aria-label={$t("telegram.watch_lib.rule_option")} onclick={() => (addToWatch = !addToWatch)}><span class="toggle-knob"></span></button>
    </div>
  </div>

  {#if !rule}
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <span class="group-row-title">{$t("telegram.watch.backfill")}</span>
          <span class="group-row-sub">{$t("telegram.watch.backfill_hint")}</span>
        </div>
        {#if backfillOn}
          <input class="input num" type="number" min="1" max="10000" bind:value={backfill} aria-label={$t("telegram.watch.backfill")} />
        {/if}
        <button class="toggle" role="switch" aria-checked={backfillOn} aria-label={$t("telegram.watch.backfill")} onclick={() => (backfillOn = !backfillOn)}><span class="toggle-knob"></span></button>
      </div>
    </div>
  {/if}

  {#if error}<p class="field-error">{error}</p>{/if}

  {#snippet footer()}
    <button class="btn btn-secondary" onclick={onclose}>{$t("telegram.common.cancel")}</button>
    <button class="btn btn-primary" onclick={submit} disabled={!chat || busy}>
      {#if busy}<span class="spinner"></span>{:else}<TgIcon name="bolt" />{/if}
      {rule ? $t("telegram.common.save") : $t("telegram.watch.create")}
    </button>
  {/snippet}
</TgModal>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .fixed-chat {
    font-weight: 600;
    color: var(--text);
  }

  .num {
    width: 90px;
  }
</style>
