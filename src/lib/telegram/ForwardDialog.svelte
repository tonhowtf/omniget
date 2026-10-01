<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgModal from "./TgModal.svelte";
  import TgIcon from "./TgIcon.svelte";
  import ChatSelect from "./ChatSelect.svelte";
  import { tg2, type TelegramChat } from "./api";
  import { failText } from "./session.svelte";

  let {
    from,
    messageIds,
    onclose,
    ondone,
  }: { from: TelegramChat; messageIds: number[]; onclose: () => void; ondone?: () => void } = $props();

  let to = $state<TelegramChat | null>(null);
  let dropAuthor = $state(false);
  let dropCaptions = $state(false);
  let confirming = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function send() {
    if (!to) return;
    busy = true;
    error = null;
    try {
      const ids = [...messageIds].sort((a, b) => a - b);
      for (let i = 0; i < ids.length; i += 100) {
        await tg2.forward(from, to, ids.slice(i, i + 100), dropAuthor, dropCaptions);
      }
      showToast("success", $t("telegram.forward.done", { count: ids.length, chat: to.title }) as string);
      ondone?.();
      onclose();
    } catch (e) {
      error = failText(e);
      confirming = false;
    } finally {
      busy = false;
    }
  }
</script>

<TgModal title={$t("telegram.forward.title", { count: messageIds.length })} subtitle={$t("telegram.forward.hint", { chat: from.title })} width={520} {onclose}>
  <div class="field">
    <span class="field-label">{$t("telegram.forward.to")}</span>
    <ChatSelect bind:value={to} exclude={from} label={$t("telegram.forward.to")} />
  </div>

  <div class="group">
    <div class="group-row">
      <div class="group-row-content">
        <span class="group-row-title">{$t("telegram.forward.drop_author")}</span>
        <span class="group-row-sub">{$t("telegram.forward.drop_author_hint")}</span>
      </div>
      <button class="toggle" role="switch" aria-checked={dropAuthor} aria-label={$t("telegram.forward.drop_author")} onclick={() => (dropAuthor = !dropAuthor)}><span class="toggle-knob"></span></button>
    </div>
    <div class="group-row">
      <div class="group-row-content"><span class="group-row-title">{$t("telegram.forward.drop_captions")}</span></div>
      <button class="toggle" role="switch" aria-checked={dropCaptions} aria-label={$t("telegram.forward.drop_captions")} onclick={() => (dropCaptions = !dropCaptions)}><span class="toggle-knob"></span></button>
    </div>
  </div>

  {#if confirming && to}
    <div class="notice notice-warning">
      <TgIcon name="warn" size={16} />
      <div class="notice-body">
        <div class="notice-title">{$t("telegram.forward.confirm", { count: messageIds.length, chat: to.title })}</div>
      </div>
    </div>
  {/if}
  {#if error}<p class="field-error">{error}</p>{/if}

  {#snippet footer()}
    <button class="btn btn-secondary" onclick={() => (confirming ? (confirming = false) : onclose())} disabled={busy}>
      {confirming ? $t("telegram.common.back") : $t("telegram.common.cancel")}
    </button>
    {#if confirming}
      <button class="btn btn-primary" onclick={send} disabled={busy}>
        {#if busy}<span class="spinner"></span>{:else}<TgIcon name="forward" />{/if}
        {$t("telegram.forward.send_now")}
      </button>
    {:else}
      <button class="btn btn-primary" onclick={() => (confirming = true)} disabled={!to}>
        <TgIcon name="forward" /> {$t("telegram.forward.review")}
      </button>
    {/if}
  {/snippet}
</TgModal>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
</style>
