<script lang="ts">
  import { t } from "$lib/i18n";

  type Props = {
    open: boolean;
    onSubmit: (content: string, sourceUrl: string, alias: string) => void;
    onClose: () => void;
  };

  let { open, onSubmit, onClose }: Props = $props();

  let content = $state("");
  let sourceUrl = $state("");
  let alias = $state("");
  let busy = $state(false);

  function submit() {
    if (!content.trim()) return;
    busy = true;
    onSubmit(content, sourceUrl.trim(), alias.trim());
  }

  function reset() {
    content = "";
    sourceUrl = "";
    alias = "";
    busy = false;
  }

  $effect(() => {
    if (!open) reset();
  });
</script>

{#if open}
  <div
    class="overlay"
    onclick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    onkeydown={(e) => { if (e.key === "Escape") onClose(); }}
    role="presentation"
  >
    <div class="modal" role="dialog" aria-label={$t("settings.cookies.paste_modal_title") as string}>
      <header class="modal-head">
        <h2>{$t("settings.cookies.paste_modal_title")}</h2>
        <p class="subtitle">{$t("settings.cookies.paste_modal_subtitle")}</p>
        <button type="button" class="close" onclick={onClose} aria-label={$t("settings.cookies.modal_close") as string}>×</button>
      </header>

      <div class="body">
        <label class="field">
          <span class="lbl">{$t("settings.cookies.paste_field_content")}</span>
          <textarea bind:value={content} rows="10" placeholder={$t("settings.cookies.paste_content_placeholder") as string} spellcheck="false"></textarea>
        </label>

        <label class="field">
          <span class="lbl">{$t("settings.cookies.paste_field_source")}</span>
          <input type="url" bind:value={sourceUrl} placeholder={$t("settings.cookies.paste_source_placeholder") as string} />
        </label>

        <label class="field">
          <span class="lbl">{$t("settings.cookies.paste_field_alias")}</span>
          <input type="text" bind:value={alias} placeholder={$t("settings.cookies.paste_alias_placeholder") as string} />
        </label>
      </div>

      <footer class="modal-foot">
        <button type="button" class="ghost-btn" onclick={onClose}>{$t("settings.cookies.paste_cancel")}</button>
        <button type="button" class="primary-btn" onclick={submit} disabled={!content.trim() || busy}>
          {busy ? $t("settings.cookies.paste_importing") : $t("settings.cookies.paste_submit")}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    padding: 24px;
  }
  .modal {
    width: min(560px, 100%);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    background: var(--surface, var(--bg));
    border: none;
    border-radius: 16px;
    overflow: hidden;
  }
  .modal-head {
    position: relative;
    padding: 20px 56px 16px 20px;
    border-bottom: none;
  }
  .modal-head h2 { margin: 0; font-size: 18px; font-weight: 600; color: var(--secondary); }
  .subtitle { margin: 4px 0 0; font-size: 12px; color: var(--tertiary); line-height: 1.5; }
  .close {
    position: absolute;
    top: 14px; right: 14px;
    width: 28px; height: 28px;
    padding: 0;
    background: transparent;
    border: 0;
    border-radius: 50%;
    color: var(--tertiary);
    font-size: 18px;
    cursor: pointer;
  }
  .close:hover { color: var(--secondary); background: color-mix(in oklab, var(--button) 40%, transparent); }
  .body { padding: 16px 20px; display: flex; flex-direction: column; gap: 14px; overflow-y: auto; flex: 1; }
  .field { display: flex; flex-direction: column; gap: 6px; }
  .lbl { font-size: 11px; font-weight: 600; color: var(--tertiary); text-transform: uppercase; letter-spacing: 0.04em; }
  textarea, input[type="url"], input[type="text"] {
    padding: 8px 12px;
    background: color-mix(in oklab, var(--button) 40%, transparent);
    border: none;
    border-radius: 8px;
    color: var(--secondary);
    font: inherit;
    font-size: 13px;
    outline: none;
    resize: vertical;
  }
  textarea { font-family: ui-monospace, "Cascadia Code", monospace; min-height: 120px; }
  textarea:focus, input:focus { border-color: var(--accent); }
  .modal-foot {
    padding: 14px 20px;
    border-top: none;
    display: flex;
    justify-content: flex-end;
    gap: 10px;
  }
  .ghost-btn {
    padding: 7px 16px;
    background: transparent;
    border: none;
    border-radius: 999px;
    color: var(--secondary);
    font-size: 13px;
    cursor: pointer;
  }
  .ghost-btn:hover { border-color: var(--accent); color: var(--accent); }
  .primary-btn {
    padding: 7px 18px;
    background: var(--cta);
    color: var(--on-cta);
    border: 0;
    border-radius: 999px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
  }
  .primary-btn:hover:not(:disabled) { filter: brightness(1.1); }
  .primary-btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
