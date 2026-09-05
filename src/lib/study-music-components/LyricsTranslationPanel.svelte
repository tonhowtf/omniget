<script lang="ts">
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";
  import { lyricsStore, type TranslationSettings } from "$lib/study-music/lyrics-store.svelte";

  type Props = {
    open: boolean;
    onClose: () => void;
  };

  let { open, onClose }: Props = $props();

  let settings = $state<TranslationSettings | null>(null);
  let translatorInput = $state<"libretranslate" | "deepl" | "none">("libretranslate");
  let urlInput = $state("");
  let targetLangInput = $state("pt");
  let showTranslationInput = $state(false);
  let deeplKeyDraft = $state("");
  let deeplKeyEditing = $state(false);
  let busy = $state(false);

  async function load() {
    try {
      const res = await pluginInvoke<TranslationSettings>(
        "study",
        "study:music:lyrics:translation_settings:get",
      );
      settings = res;
      translatorInput = res.translator;
      urlInput = res.libretranslate_url;
      targetLangInput = res.target_lang;
      showTranslationInput = res.show_translation;
      deeplKeyDraft = "";
      deeplKeyEditing = !res.deepl_api_key_set;
      lyricsStore.showTranslation = res.show_translation;
    } catch {
      settings = null;
    }
  }

  $effect(() => {
    if (open) void load();
  });

  async function save() {
    if (busy) return;
    busy = true;
    try {
      const payload: Record<string, unknown> = {
        translator: translatorInput,
        libretranslate_url: urlInput.trim(),
        target_lang: targetLangInput.trim() || "pt",
        show_translation: showTranslationInput,
      };
      if (deeplKeyEditing) {
        payload.deepl_api_key = deeplKeyDraft;
      }
      await pluginInvoke("study", "study:music:lyrics:translation_settings:set", payload);
      lyricsStore.showTranslation = showTranslationInput;
      showToast("success", $t("study.music.translation_settings_saved") as string);
      deeplKeyDraft = "";
      await load();
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    }
  }

  function startEditDeeplKey() {
    deeplKeyEditing = true;
    deeplKeyDraft = "";
  }
</script>

{#if open}
  <div
    class="overlay"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
    onkeydown={onKey}
  >
    <div class="dialog" role="dialog" aria-modal="true" tabindex="-1">
      <header class="head">
        <h3>{$t("study.music.translation_settings_title")}</h3>
        <button type="button" class="close" onclick={onClose} aria-label={$t("study.common.close") as string}>×</button>
      </header>
      <div class="body">
        {#if settings === null}
          <p class="muted">{$t("study.common.loading")}</p>
        {:else}
          <label class="field">
            <span>{$t("study.music.translation_settings_translator_label")}</span>
            <select bind:value={translatorInput} disabled={busy}>
              <option value="libretranslate">{$t("study.music.translation_settings_translator_libretranslate")}</option>
              <option value="deepl">{$t("study.music.translation_settings_translator_deepl")}</option>
              <option value="none">{$t("study.music.translation_settings_translator_none")}</option>
            </select>
          </label>

          {#if translatorInput === "libretranslate"}
            <label class="field">
              <span>{$t("study.music.translation_settings_libretranslate_url_label")}</span>
              <input
                type="text"
                bind:value={urlInput}
                placeholder="http://localhost:5000"
                autocomplete="off"
                disabled={busy}
              />
              <small>{$t("study.music.translation_settings_libretranslate_url_hint")}</small>
            </label>
          {/if}

          {#if translatorInput === "deepl"}
            <label class="field">
              <span>{$t("study.music.translation_settings_deepl_key_label")}</span>
              {#if deeplKeyEditing}
                <input
                  type="password"
                  bind:value={deeplKeyDraft}
                  placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx:fx"
                  autocomplete="off"
                  disabled={busy}
                />
              {:else}
                <div class="key-saved">
                  <span class="key-saved-text">••••••••••••••••</span>
                  <button type="button" class="cta-ghost" onclick={startEditDeeplKey}>
                    {$t("study.music.translation_settings_deepl_key_set")}
                  </button>
                </div>
              {/if}
              <small>{$t("study.music.translation_settings_deepl_key_hint")}</small>
            </label>
          {/if}

          <label class="field">
            <span>{$t("study.music.translation_settings_target_lang_label")}</span>
            <select bind:value={targetLangInput} disabled={busy}>
              <option value="pt">Português (PT-BR)</option>
              <option value="es">Español</option>
              <option value="fr">Français</option>
              <option value="de">Deutsch</option>
              <option value="it">Italiano</option>
              <option value="ja">日本語</option>
            </select>
          </label>

          <label class="toggle-row">
            <input
              type="checkbox"
              bind:checked={showTranslationInput}
              disabled={busy}
            />
            <span>{$t("study.music.translation_settings_show_label")}</span>
          </label>

          <div class="actions">
            <button type="button" class="cta" onclick={save} disabled={busy}>
              {$t("study.music.translation_settings_saved")}
            </button>
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 320;
    display: grid;
    place-items: center;
    backdrop-filter: blur(4px);
  }
  .dialog {
    background: rgb(20, 20, 20);
    color: rgba(255, 255, 255, 0.95);
    border: none;
    border-radius: 14px;
    width: min(540px, 92vw);
    max-height: 86vh;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.5);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 16px 18px;
    border-bottom: none;
  }
  .head h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: rgba(255, 255, 255, 0.9);
  }
  .close {
    width: 32px;
    height: 32px;
    background: transparent;
    border: 0;
    border-radius: 50%;
    color: rgba(255, 255, 255, 0.6);
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
  }
  .close:hover { color: white; background: rgba(255, 255, 255, 0.08); }
  .body {
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 18px;
    overflow-y: auto;
  }
  .muted { color: rgba(255, 255, 255, 0.45); font-size: 13px; margin: 0; }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field > span {
    font-size: 12px;
    color: rgba(255, 255, 255, 0.65);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .field input, .field select {
    padding: 10px 12px;
    background: rgba(255, 255, 255, 0.04);
    border: none;
    border-radius: 10px;
    color: white;
    font-size: 14px;
  }
  .field input:focus, .field select:focus {
    outline: none;
    border-color: var(--accent);
    background: rgba(255, 255, 255, 0.08);
  }
  .field small {
    font-size: 11px;
    color: rgba(255, 255, 255, 0.45);
    line-height: 1.5;
  }
  .toggle-row {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13px;
    color: rgba(255, 255, 255, 0.85);
    cursor: pointer;
  }
  .toggle-row input { width: 16px; height: 16px; cursor: pointer; }
  .key-saved {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .key-saved-text {
    flex: 1;
    padding: 10px 12px;
    background: rgba(255, 255, 255, 0.04);
    border: none;
    border-radius: 10px;
    color: rgba(255, 255, 255, 0.6);
    font-family: monospace;
    letter-spacing: 0.1em;
    font-size: 14px;
  }
  .cta-ghost {
    padding: 10px 14px;
    background: transparent;
    border: none;
    border-radius: 999px;
    color: rgba(255, 255, 255, 0.85);
    font-size: 12px;
    cursor: pointer;
  }
  .cta-ghost:hover {
    background: rgba(255, 255, 255, 0.06);
    color: white;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    margin-top: 4px;
  }
  .cta {
    padding: 10px 18px;
    background: var(--accent);
    border: 0;
    border-radius: 999px;
    color: var(--on-accent);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: filter 160ms ease, transform 160ms ease;
  }
  .cta:hover { filter: brightness(1.1); }
  .cta:active { transform: scale(0.98); }
  .cta:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
