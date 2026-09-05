<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { getSettings, loadSettings } from "$lib/stores/settings-store.svelte";
  import DialogContainer from "./DialogContainer.svelte";

  const settings = $derived(getSettings());
  let isOpen = $state(false);
  let busy = $state(false);

  $effect(() => {
    if (!settings) return;
    if (settings.onboarding_completed && !settings.legal_acknowledged) {
      isOpen = true;
    }
  });

  async function acknowledge() {
    if (busy) return;
    busy = true;
    try {
      await invoke("mark_legal_acknowledged");
      await loadSettings();
      isOpen = false;
    } catch (_) {
      isOpen = false;
    } finally {
      busy = false;
    }
  }
</script>

<DialogContainer bind:isOpen onClose={() => { if (!settings?.legal_acknowledged) acknowledge(); }} titleId="legal-dialog-title">
  <div class="legal-header">
    <h2 id="legal-dialog-title">{$t("legal.title")}</h2>
  </div>

  <div class="legal-body">
    <p>{$t("legal.body_paragraph_1")}</p>
    <p>{$t("legal.body_paragraph_2")}</p>
    <ul>
      <li>{$t("legal.rule_copyright")}</li>
      <li>{$t("legal.rule_tos")}</li>
      <li>{$t("legal.rule_personal_use")}</li>
    </ul>
  </div>

  <div class="legal-footer">
    <button class="btn btn-primary btn-lg" onclick={acknowledge} disabled={busy}>
      {$t("legal.acknowledge")}
    </button>
  </div>
</DialogContainer>

<style>
  .legal-header {
    padding: 20px 24px 4px;
  }

  .legal-header h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
    color: var(--secondary);
  }

  .legal-body {
    padding: 8px 24px 16px;
    color: var(--secondary);
    font-size: 13px;
    line-height: 1.55;
    max-height: 50vh;
    overflow-y: auto;
  }

  .legal-body p {
    margin: 0 0 10px 0;
  }

  .legal-body ul {
    margin: 4px 0 0 0;
    padding-left: 18px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .legal-body li {
    color: var(--tertiary);
    font-size: var(--text-sm);
  }

  .legal-footer {
    display: flex;
    justify-content: flex-end;
    padding: 8px 20px 20px;
  }



</style>
