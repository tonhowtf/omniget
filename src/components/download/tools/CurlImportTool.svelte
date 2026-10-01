<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Preview = {
    url: string;
    host: string;
    header_names: string[];
    has_cookie: boolean;
    has_referer: boolean;
    has_user_agent: boolean;
  };
  type Queued = { url: string; already_queued: boolean };

  const KNOWN_ERRORS = ["CURL_NOT_A_COMMAND", "CURL_NOT_GET", "CURL_NO_URL", "CURL_NOT_HTTP", "CURL_UNCLOSED_QUOTE", "CURL_MISSING_VALUE"];

  let command = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let preview = $state<Preview | null>(null);

  function explain(e: unknown): string {
    const msg = typeof e === "string" ? e : ((e as { message?: string })?.message ?? "");
    if (KNOWN_ERRORS.includes(msg)) return $t(`downloads.tools.curl_import.error_${msg.slice(5).toLowerCase()}`);
    return msg ? translateBackendError(msg, $t) : $t("common.error");
  }

  async function check() {
    const text = command.trim();
    preview = null;
    error = null;
    if (!text) return;
    try {
      preview = await invoke<Preview>("curl_command_inspect", { command: text });
    } catch (e) {
      error = explain(e);
    }
  }

  async function download() {
    const text = command.trim();
    if (!text || busy) return;
    busy = true;
    error = null;
    try {
      const r = await invoke<Queued>("curl_command_download", { command: text });
      showToast("success", $t(r.already_queued ? "downloads.tools.curl_import.already_queued" : "downloads.tools.curl_import.queued"));
      command = "";
      preview = null;
    } catch (e) {
      error = explain(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="ci" data-agent-readiness={busy ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.curl_import.lead")}</p>
  <textarea
    bind:value={command}
    oninput={check}
    rows="6"
    placeholder={$t("downloads.tools.curl_import.placeholder")}
    aria-label={$t("downloads.tools.curl_import.placeholder")}
    spellcheck="false"
    autocomplete="off"
  ></textarea>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if preview}
    <div class="preview" data-host={preview.host}>
      <span class="dim small">{$t("downloads.tools.curl_import.file")}</span>
      <strong class="url">{preview.url}</strong>
      <span class="dim small">
        {#if preview.header_names.length}
          {$t("downloads.tools.curl_import.headers", { count: preview.header_names.length })}: {preview.header_names.join(", ")}
        {:else}
          {$t("downloads.tools.curl_import.no_headers")}
        {/if}
      </span>
      {#if preview.has_cookie}
        <span class="dim small">{$t("downloads.tools.curl_import.cookie_note")}</span>
      {/if}
    </div>
  {/if}

  <div class="actions">
    <button class="button" onclick={download} disabled={busy || !preview}>
      {busy ? $t("downloads.tools.curl_import.adding") : $t("downloads.tools.curl_import.download")}
    </button>
  </div>
  <p class="dim small">{$t("downloads.tools.curl_import.how")}</p>
</div>

<style>
  .ci {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .small {
    font-size: var(--text-xs, 11px);
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-xs, 11px);
    resize: vertical;
  }
  textarea:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .preview {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    min-width: 0;
  }
  .url {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
  }
</style>
