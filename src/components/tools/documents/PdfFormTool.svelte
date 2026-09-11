<script lang="ts">
  /** Preencher AcroForm e achatar: o valor vira tinta na página. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openPath, pct, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Field = {
    name: string;
    kind: string;
    value: string;
    options: string[];
    page: number;
    rect: [number, number, number, number];
    readonly: boolean;
    required: boolean;
    multiline: boolean;
  };
  type Result = { input: string; output: string; fields: number; filled: number; flattened: boolean; missing: string[] };

  const PDF = [{ name: "PDF", extensions: ["pdf"] }];

  let input = $state("");
  let fields = $state<Field[]>([]);
  let values = $state<Record<string, string>>({});
  let flatten = $state(true);
  let busy = $state(false);
  let loading = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | undefined;

  const editable = $derived(fields.filter((f) => f.kind !== "push" && f.kind !== "signature"));

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "pdf-form-fill") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function load(path: string) {
    loading = true;
    result = null;
    try {
      fields = await invoke<Field[]>("tool_pdf_form_fields", { input: path });
      values = Object.fromEntries(fields.map((f) => [f.name, f.value]));
      if (!fields.length) showToast("info", $t("tools.form.empty") as string);
    } catch (e) {
      fields = [];
      showToast("error", errText(e));
    } finally {
      loading = false;
    }
  }

  function toggleCheck(f: Field) {
    const on = values[f.name] && values[f.name] !== "Off";
    values[f.name] = on ? "Off" : f.options[0] || "Yes";
  }

  async function run() {
    if (!input || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_pdf_form_fill", {
        opts: {
          input,
          values: editable.map((f) => ({ name: f.name, value: values[f.name] ?? "" })),
          flatten,
          font_size: 0,
        },
      });
      showToast("success", `${result.filled} ${$t("tools.form.filled")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{input ? baseName(input) : $t("tools.form.file")}</div>
          <div class="group-row-sub">{loading ? $t("tools.common.working") : $t("tools.form.file_sub")}</div>
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(PDF); if (f) { input = f; await load(f); } }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if editable.length}
    <section>
      <div class="group">
        {#each editable as f (f.name)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{f.name}{#if f.required}<span class="tag tag-danger">{$t("tools.form.required")}</span>{/if}</div>
              <div class="group-row-sub">{$t("tools.form.page")} {f.page} · {f.kind}{#if f.readonly} · {$t("tools.form.readonly")}{/if}</div>
            </div>
            <div class="group-row-trailing">
              {#if f.kind === "check"}
                <button class="toggle" class:on={values[f.name] && values[f.name] !== "Off"} type="button" role="switch" aria-checked={!!values[f.name] && values[f.name] !== "Off"} aria-label={f.name} onclick={() => toggleCheck(f)}><span class="toggle-knob"></span></button>
              {:else if f.options.length}
                <select class="input" bind:value={values[f.name]}>
                  <option value="Off">—</option>
                  {#each f.options as o (o)}<option value={o}>{o}</option>{/each}
                </select>
              {:else if f.multiline}
                <textarea class="input area" rows="3" bind:value={values[f.name]}></textarea>
              {:else}
                <input class="input" type="text" bind:value={values[f.name]} />
              {/if}
            </div>
          </div>
        {/each}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.form.flatten")}</div><div class="group-row-sub">{$t("tools.form.flatten_sub")}</div></div>
          <div class="group-row-trailing"><button class="toggle" class:on={flatten} type="button" role="switch" aria-checked={flatten} aria-label={$t("tools.form.flatten") as string} onclick={() => (flatten = !flatten)}><span class="toggle-knob"></span></button></div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            {#if busy}
              <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
              <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
            {/if}
          </div>
          <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !input} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.form.run")}</button></div>
        </div>
      </div>
    </section>
  {/if}

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title"><span class="tag tag-success">{result.filled} {$t("tools.form.filled")}</span></div>
            <div class="group-row-sub">
              {result.fields} {$t("tools.form.fields")}{#if result.flattened} · {$t("tools.form.flattened")}{/if}
              {#if result.missing.length}<span class="dim"> · {$t("tools.form.missing")}: {result.missing.join(", ")}</span>{/if}
            </div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => openPath(result!.output)}>{$t("tools.common.open")}</button>
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.output)}>{$t("tools.common.reveal")}</button>
          </div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .area { min-width: 16rem; resize: vertical; }
</style>
