<script lang="ts">
  /**
   * Sandbox choice for a coding-CLI run: none, OmniGet's Docker recipe (image
   * generated per tool, project copied, normal permissions) or E2B with the
   * user's key from the vault. Docker missing → the option says why.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { runSandboxDockerfile, runSandboxStatus, type SandboxOpts, type SandboxStatus } from "./run-api";

  let {
    tool,
    value = $bindable<SandboxOpts | null>(null),
    hasProject = true,
  }: { tool: string; value?: SandboxOpts | null; hasProject?: boolean } = $props();

  let status = $state<SandboxStatus | null>(null);
  let provider = $state<"" | "docker" | "e2b">("");
  let keyId = $state("");
  let e2bKeyId = $state("");
  let bind = $state(false);
  let network = $state(true);
  let dockerfile = $state<{ dockerfile: string; tag: string } | null>(null);

  onMount(async () => {
    try {
      status = await runSandboxStatus();
      const e2b = status.keys.find((k) => k.e2b && k.has_key);
      if (e2b) e2bKeyId = e2b.id;
    } catch {
      status = null;
    }
  });

  let supported = $derived(!!status?.tools.some((x) => x.tool === tool));
  let toolKeys = $derived(
    (status?.keys ?? []).filter((k) => k.has_key && !k.e2b),
  );

  $effect(() => {
    value = provider
      ? {
          provider,
          bind_original: provider === "docker" ? bind : false,
          key_id: keyId || null,
          e2b_key_id: provider === "e2b" ? e2bKeyId || null : null,
          network,
        }
      : null;
  });

  $effect(() => {
    if (provider !== "docker" || !supported) {
      dockerfile = null;
      return;
    }
    runSandboxDockerfile(tool).then((d) => (dockerfile = d)).catch(() => (dockerfile = null));
  });
</script>

<fieldset class="box">
  <legend class="field-label">{$t("llm.central.run.sandbox.title")}</legend>
  <div class="choices">
    <label><input type="radio" name="sb-{tool}" value="" bind:group={provider} /> {$t("llm.central.run.sandbox.none")}</label>
    <label class:off={!status?.docker.available || !supported}>
      <input type="radio" name="sb-{tool}" value="docker" bind:group={provider} disabled={!status?.docker.available || !supported} />
      Docker {status?.docker.version ? `(${status.docker.version})` : ""}
    </label>
    <label class:off={!supported}>
      <input type="radio" name="sb-{tool}" value="e2b" bind:group={provider} disabled={!supported} /> E2B
    </label>
  </div>
  {#if status && !status.docker.available}
    <p class="hint">{$t("llm.central.run.sandbox.docker_missing", { reason: status.docker.error ?? "" })}</p>
  {/if}
  {#if status && !supported}
    <p class="hint">{$t("llm.central.run.sandbox.unsupported", { tool, list: status.tools.map((x) => x.tool).join(", ") })}</p>
  {/if}
  {#if provider}
    {#if !hasProject}<p class="warn">{$t("llm.central.run.sandbox.needs_project")}</p>{/if}
    <label class="field">
      <span class="field-label">{$t("llm.central.run.sandbox.key")}</span>
      <select class="input" bind:value={keyId}>
        <option value="">{$t("llm.central.run.sandbox.no_key")}</option>
        {#each toolKeys as k (k.id)}<option value={k.id}>{k.name} · {k.kind}</option>{/each}
      </select>
      <span class="hint">{$t("llm.central.run.sandbox.key_hint")}</span>
    </label>
    {#if provider === "e2b"}
      <label class="field">
        <span class="field-label">{$t("llm.central.run.sandbox.e2b_key")}</span>
        <select class="input" bind:value={e2bKeyId}>
          <option value="">—</option>
          {#each status?.keys.filter((k) => k.has_key) ?? [] as k (k.id)}<option value={k.id}>{k.name}</option>{/each}
        </select>
        <span class="hint">{$t("llm.central.run.sandbox.e2b_hint")}</span>
      </label>
    {:else}
      <label class="check"><input type="checkbox" bind:checked={bind} /> {$t("llm.central.run.sandbox.bind")}</label>
      <label class="check"><input type="checkbox" bind:checked={network} /> {$t("llm.central.run.sandbox.network")}</label>
      {#if dockerfile}
        <details>
          <summary class="hint">{$t("llm.central.run.sandbox.dockerfile", { tag: dockerfile.tag })}</summary>
          <pre class="pre">{dockerfile.dockerfile}</pre>
        </details>
      {/if}
    {/if}
    <p class="hint">{bind ? $t("llm.central.run.sandbox.bind_hint") : $t("llm.central.run.sandbox.copy_hint")}</p>
  {/if}
</fieldset>

<style>
  .box {
    border: var(--hairline) solid var(--separator);
    border-radius: var(--radius-md);
    padding: var(--space-2) var(--space-3) var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
  }
  .choices {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    font-size: var(--text-sm);
  }
  .off {
    opacity: 0.55;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .check {
    font-size: var(--text-sm);
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .hint,
  .warn {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .warn {
    color: var(--warning);
  }
  .pre {
    white-space: pre-wrap;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    background: var(--fill-1);
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    margin: 4px 0 0;
  }
</style>
