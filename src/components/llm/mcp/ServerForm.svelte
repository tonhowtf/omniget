<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import {
    formatArgs,
    formatPairs,
    isSecretRef,
    parseArgs,
    parsePairs,
    slugifyId,
    validateServer,
    type McpServerConfig,
  } from "$lib/llm/mcp";

  let {
    config,
    existingIds = [],
    onsave,
    oncancel,
  }: {
    config: McpServerConfig;
    existingIds?: string[];
    onsave: (config: McpServerConfig) => void;
    oncancel: () => void;
  } = $props();

  const seed = untrack(() => structuredClone($state.snapshot(config)) as McpServerConfig);
  const isNew = untrack(() => !existingIds.includes(seed.id));

  let id = $state(seed.id);
  let name = $state(seed.name);
  let enabled = $state(seed.enabled);
  let kind = $state<"stdio" | "http">(seed.transport.kind);
  let command = $state(seed.transport.kind === "stdio" ? seed.transport.command : "");
  let argsText = $state(seed.transport.kind === "stdio" ? formatArgs(seed.transport.args) : "");
  let envText = $state(seed.transport.kind === "stdio" ? formatPairs(seed.transport.env) : "");
  let cwd = $state(seed.transport.kind === "stdio" ? (seed.transport.cwd ?? "") : "");
  let url = $state(seed.transport.kind === "http" ? seed.transport.url : "");
  let headersText = $state(
    seed.transport.kind === "http" ? formatPairs(seed.transport.headers, ": ") : "",
  );
  let touchedId = $state(!isNew);

  let draft = $derived<McpServerConfig>({
    id: id.trim(),
    name: name.trim(),
    enabled,
    transport:
      kind === "stdio"
        ? {
            kind: "stdio",
            command: command.trim(),
            args: parseArgs(argsText),
            env: parsePairs(envText),
            cwd: cwd.trim() || null,
          }
        : {
            kind: "http",
            url: url.trim(),
            headers: parsePairs(headersText),
          },
  });

  let problems = $derived(validateServer(draft));
  let duplicate = $derived(isNew && existingIds.includes(draft.id));
  let secretHeaders = $derived(
    kind === "http"
      ? Object.entries(parsePairs(headersText)).filter(([, value]) => isSecretRef(value)).length
      : 0,
  );

  function onName(value: string) {
    name = value;
    if (!touchedId) id = slugifyId(value);
  }

  function submit(event: SubmitEvent) {
    event.preventDefault();
    if (problems.length > 0 || duplicate) return;
    onsave($state.snapshot(draft) as McpServerConfig);
  }
</script>

<form class="form" onsubmit={submit}>
  <label class="field">
    <span class="field-label">{$t("llm.mcp.field.name")}</span>
    <input
      class="input"
      value={name}
      oninput={(e) => onName(e.currentTarget.value)}
      required
      maxlength="48"
    />
  </label>

  <label class="field">
    <span class="field-label">{$t("llm.mcp.field.id")}</span>
    <input
      class="input"
      bind:value={id}
      oninput={() => (touchedId = true)}
      disabled={!isNew}
      maxlength="64"
    />
    <span class="field-hint">{$t("llm.mcp.field.id_hint")}</span>
  </label>

  <fieldset class="block">
    <legend class="field-label">{$t("llm.mcp.field.transport")}</legend>
    <div class="mac-segmented" role="tablist">
      <button
        type="button"
        class="mac-segmented-btn"
        class:active={kind === "stdio"}
        role="tab"
        aria-selected={kind === "stdio"}
        onclick={() => (kind = "stdio")}
      >
        {$t("llm.mcp.transport.stdio")}
      </button>
      <button
        type="button"
        class="mac-segmented-btn"
        class:active={kind === "http"}
        role="tab"
        aria-selected={kind === "http"}
        onclick={() => (kind = "http")}
      >
        {$t("llm.mcp.transport.http")}
      </button>
    </div>

    {#if kind === "stdio"}
      <label class="field">
        <span class="field-label">{$t("llm.mcp.field.command")}</span>
        <input class="input mono" bind:value={command} placeholder="npx" />
      </label>
      <label class="field">
        <span class="field-label">{$t("llm.mcp.field.args")}</span>
        <input class="input mono" bind:value={argsText} placeholder="-y @scope/server /path" />
        <span class="field-hint">{$t("llm.mcp.field.args_hint")}</span>
      </label>
      <label class="field">
        <span class="field-label">{$t("llm.mcp.field.env")}</span>
        <textarea class="input mono" rows="3" bind:value={envText} placeholder="TOKEN=secret:my-key"
        ></textarea>
        <span class="field-hint">{$t("llm.mcp.field.env_hint")}</span>
      </label>
      <label class="field">
        <span class="field-label">{$t("llm.mcp.field.cwd")}</span>
        <input class="input mono" bind:value={cwd} />
      </label>
    {:else}
      <label class="field">
        <span class="field-label">{$t("llm.mcp.field.url")}</span>
        <input class="input mono" bind:value={url} placeholder="https://example.com/mcp" />
      </label>
      <label class="field">
        <span class="field-label">{$t("llm.mcp.field.headers")}</span>
        <textarea
          class="input mono"
          rows="3"
          bind:value={headersText}
          placeholder="Authorization: secret:my-key"
        ></textarea>
        <span class="field-hint">{$t("llm.mcp.field.headers_hint")}</span>
      </label>
      {#if secretHeaders > 0}
        <p class="field-hint">{$t("llm.mcp.field.secret_count", { count: secretHeaders })}</p>
      {/if}
    {/if}
  </fieldset>

  <label class="check">
    <input type="checkbox" class="checkbox" bind:checked={enabled} />
    <span>{$t("llm.mcp.field.enabled")}</span>
  </label>

  {#if duplicate}
    <p class="field-error">{$t("llm.mcp.invalid.duplicate")}</p>
  {/if}
  {#each problems as problem (problem)}
    <p class="field-error">{$t(problem)}</p>
  {/each}

  <div class="actions">
    <button type="submit" class="button primary" disabled={problems.length > 0 || duplicate}>
      {$t("llm.mcp.save")}
    </button>
    <button type="button" class="button" onclick={oncancel}>{$t("llm.mcp.cancel")}</button>
  </div>
</form>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 560px;
  }

  .block {
    border: var(--hairline) solid var(--separator);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  textarea.input {
    resize: vertical;
  }

  .check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }

  .actions {
    display: flex;
    gap: var(--space-2);
  }
</style>
