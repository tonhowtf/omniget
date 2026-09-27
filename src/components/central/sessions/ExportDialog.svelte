<script lang="ts">
  /**
   * Exportar (Markdown/JSON neutro) ou converter para outra ferramenta
   * (Markdown de contexto para colar como primeira mensagem). O conteúdo vem
   * pronto do núcleo; aqui só se escolhe o destino e se grava ou copia.
   */
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import Modal from "./Modal.svelte";
  import { copyText, errText, saveTextAs, sessionsExport, TOOL_NAMES, toolName, type ExportFormat, type ExportOut } from "$lib/central/sessions";

  interface Props {
    tool: string;
    id: string;
    mode: "export" | "convert";
    onclose: () => void;
  }

  let { tool, id, mode, onclose }: Props = $props();

  let format = $state<ExportFormat>("markdown");
  let target = $state("");
  let busy = $state(false);
  let out = $state<ExportOut | null>(null);
  let error = $state("");

  let targets = $derived(Object.keys(TOOL_NAMES).filter((k) => k !== tool));

  async function build(): Promise<ExportOut | null> {
    busy = true;
    error = "";
    try {
      const f: ExportFormat = mode === "convert" ? "context" : format;
      out = await sessionsExport(tool, id, f, mode === "convert" ? target || null : null);
      return out;
    } catch (e) {
      error = errText(e);
      return null;
    } finally {
      busy = false;
    }
  }

  async function save() {
    const o = out ?? (await build());
    if (!o) return;
    const ext = o.filename.split(".").pop() || (o.mime.includes("json") ? "json" : "md");
    try {
      const dest = await saveTextAs(o.filename, o.content, ext);
      if (dest) showToast("success", `${$t("llm.central.sessions.saved")} ${dest}`);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function copy() {
    const o = out ?? (await build());
    if (!o) return;
    await copyText(o.content);
    showToast("success", $t("llm.central.sessions.copied"));
  }

  $effect(() => {
    void format;
    void target;
    out = null;
  });
</script>

<Modal title={mode === "convert" ? $t("llm.central.sessions.convert.title") : $t("llm.central.sessions.export.title")} {onclose}>
  <div class="form">
    {#if mode === "export"}
      <div class="mac-segmented" role="radiogroup" aria-label={$t("llm.central.sessions.export.format")}>
        <button type="button" class="mac-segmented-btn" class:active={format === "markdown"} role="radio" aria-checked={format === "markdown"} onclick={() => (format = "markdown")}>Markdown</button>
        <button type="button" class="mac-segmented-btn" class:active={format === "json"} role="radio" aria-checked={format === "json"} onclick={() => (format = "json")}>JSON</button>
      </div>
      <p class="hint">{format === "json" ? $t("llm.central.sessions.export.json_hint") : $t("llm.central.sessions.export.md_hint")}</p>
    {:else}
      <p class="hint">{$t("llm.central.sessions.convert.hint")}</p>
      <label class="field">
        <span class="field-label">{$t("llm.central.sessions.convert.target")}</span>
        <select class="input" bind:value={target}>
          <option value="">{$t("llm.central.sessions.convert.any")}</option>
          {#each targets as k (k)}<option value={k}>{toolName(k)}</option>{/each}
        </select>
      </label>
    {/if}
    {#if error}<p class="err">{error}</p>{/if}
    {#if out}
      <p class="meta">{out.filename} · {out.turns_included}/{out.turns_total} {$t("llm.central.sessions.turns_unit")} · {Math.round(out.content.length / 1024)} KB</p>
      <pre class="preview">{out.content.slice(0, 3000)}{out.content.length > 3000 ? "\n…" : ""}</pre>
    {/if}
  </div>
  {#snippet actions()}
    <button type="button" class="btn btn-secondary" onclick={build} disabled={busy}>{$t("llm.central.sessions.preview")}</button>
    <button type="button" class="btn btn-secondary" onclick={copy} disabled={busy}>{$t("llm.central.sessions.copy")}</button>
    <button type="button" class="btn btn-primary" onclick={save} disabled={busy}>{$t("llm.central.sessions.save_as")}</button>
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .hint,
  .meta {
    margin: 0;
    font-size: var(--text-sm);
  }
  .meta {
    color: var(--text-dim);
    font-size: var(--text-xs);
  }
  .preview {
    margin: 0;
    max-height: 280px;
    overflow: auto;
    padding: 8px 10px;
    background: var(--fill-1);
    border-radius: var(--radius-md);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--text);
  }
  .err {
    margin: 0;
    color: var(--danger);
  }
</style>
