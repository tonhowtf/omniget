<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/devtools/api";
  import { buildUtilArgs, guessOp, utilFields, utilOutput, type UtilForm } from "$lib/devtools/helpers";
  import type { DevtoolsError, UtilOp } from "$lib/devtools/types";
  import ErrorNotice from "./ErrorNotice.svelte";
  import CopyButton from "$components/palette/CopyButton.svelte";

  const OPS: UtilOp[] = ["json_format", "json_minify", "json_query", "jwt_decode", "base64_encode", "base64_decode", "url_encode", "url_decode", "hash", "uuid", "timestamp", "regex_test", "text_diff"];
  const ALGORITHMS: UtilForm["algorithm"][] = ["sha256", "sha512", "sha1", "md5"];

  let op = $state<UtilOp>("json_format");
  let form = $state<UtilForm>({ text: "", other: "", query: "", pattern: "", algorithm: "sha256", value: "" });
  let busy = $state(false);
  let result = $state<{ op: UtilOp; data: Record<string, unknown> } | null>(null);
  let error = $state<DevtoolsError | null>(null);

  let fields = $derived(utilFields(op));
  let guess = $derived(guessOp(form.text));
  let suggestion = $derived(guess && guess !== op && fields.includes("text") ? guess : null);
  let output = $derived(result ? utilOutput(result.op, result.data) : "");

  async function run() {
    if (busy) return;
    busy = true;
    error = null;
    const ran = op;
    try {
      result = { op: ran, data: await api.util(ran, buildUtilArgs(ran, form)) };
    } catch (e) {
      result = null;
      error = api.parseError(e);
    } finally {
      busy = false;
    }
  }

  function choose(next: UtilOp) {
    if (next === "timestamp" && /^\d+$/.test(form.text.trim())) form.value = form.text.trim();
    op = next;
    void run();
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void run();
    }
  }
</script>

<form
  class="dv-panel"
  aria-busy={busy}
  onsubmit={(e) => {
    e.preventDefault();
    void run();
  }}
>
  <div class="dv-row">
    <label class="dv-field up-op">
      {$t("devtools.op_label")}
      <select
        class="input input-lg"
        bind:value={op}
        onchange={() => {
          error = null;
          result = null;
        }}
      >
        {#each OPS as id (id)}<option value={id}>{$t(`devtools.op_${id}`)}</option>{/each}
      </select>
    </label>
    {#if fields.includes("algorithm")}
      <label class="dv-field fixed">
        {$t("devtools.field_algorithm")}
        <select class="input input-lg dv-mono" bind:value={form.algorithm}>
          {#each ALGORITHMS as id (id)}<option value={id}>{id}</option>{/each}
        </select>
      </label>
    {/if}
    {#if fields.includes("query")}
      <label class="dv-field wide">
        {$t("devtools.field_query")}
        <input class="input input-lg dv-mono" bind:value={form.query} placeholder={'items[0].name'} spellcheck="false" autocomplete="off" />
      </label>
    {/if}
    {#if fields.includes("pattern")}
      <label class="dv-field wide">
        {$t("devtools.field_pattern")}
        <input class="input input-lg dv-mono" bind:value={form.pattern} placeholder={'(\\w+)@(\\w+)'} spellcheck="false" autocomplete="off" />
      </label>
    {/if}
    {#if fields.includes("value")}
      <label class="dv-field wide">
        {$t("devtools.field_value")}
        <input class="input input-lg dv-mono" bind:value={form.value} placeholder="1900000000" spellcheck="false" autocomplete="off" />
      </label>
    {/if}
    <button type="submit" class="btn btn-primary btn-lg" disabled={busy}>
      {#if busy}<span class="spinner" aria-hidden="true"></span>{/if}
      {$t("devtools.run")}
    </button>
  </div>

  {#if fields.includes("text")}
    <div class="up-inputs" class:two={fields.includes("other")}>
      <label class="dv-field">
        {$t("devtools.input")}
        <!-- svelte-ignore a11y_autofocus -->
        <textarea class="input" rows="7" bind:value={form.text} onkeydown={keydown} placeholder={$t("devtools.input_hint") as string} spellcheck="false" autofocus></textarea>
      </label>
      {#if fields.includes("other")}
        <label class="dv-field">
          {$t("devtools.field_other")}
          <textarea class="input" rows="7" bind:value={form.other} onkeydown={keydown} spellcheck="false"></textarea>
        </label>
      {/if}
    </div>
  {/if}

  {#if suggestion}
    <div class="up-suggest" role="status">
      <span>{$t("devtools.suggest", { op: $t(`devtools.op_${suggestion}`) })}</span>
      <button type="button" class="btn btn-tinted btn-sm" onclick={() => suggestion && choose(suggestion)}>{$t("devtools.suggest_use")}</button>
    </div>
  {/if}
</form>

{#if error}
  <ErrorNotice {error}>
    {#if error.candidates.length > 0}
      <div class="dv-chips">
        {#each error.candidates as candidate (candidate)}
          <span class="tag dv-mono">{candidate}</span>
        {/each}
      </div>
    {/if}
  </ErrorNotice>
{/if}

<div class="dv-panel" aria-live="polite">
  <div class="dv-bar">
    <h2 class="dv-title">{$t("devtools.output")}</h2>
    {#if output}<CopyButton text={output} label={$t("devtools.copy_output") as string} named />{/if}
  </div>
  {#if !result}
    <p class="dv-meta">{$t("devtools.output_empty")}</p>
  {:else if output}
    <pre class="dv-code">{output}</pre>
    {#if result.data.truncated === true}
      <p class="dv-warn">{$t("devtools.output_truncated", { total: Number(result.data.total).toLocaleString() })}</p>
    {/if}
    {#if result.op === "jwt_decode"}
      <p class="dv-warn">{$t("devtools.jwt_unverified")}</p>
    {/if}
  {:else}
    <p class="dv-meta">{$t("devtools.no_result")}</p>
  {/if}
</div>

<style>
  .dv-row > .up-op {
    flex: 0 1 220px;
  }

  .up-inputs {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--space-3);
  }

  .up-inputs.two {
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
  }

  .up-suggest {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-muted);
    font-size: var(--text-sm);
  }
</style>
