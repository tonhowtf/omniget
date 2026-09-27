<script lang="ts">
  /**
   * Who runs it: an agent of the roster (native or not), a Claude/Codex
   * account, an ACP agent, or a coding CLI run headless. Tool runners also
   * take a model and a permission level; "bypass" names the exact flag.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import {
    parseRunnerKey,
    runnerKey,
    runRunners,
    type RunnerCatalog,
    type RunnerSpec,
  } from "./run-api";

  let {
    value = $bindable<RunnerSpec>({ kind: "agent", id: "omni" }),
    toolsOnly = false,
    compact = false,
  }: { value?: RunnerSpec; toolsOnly?: boolean; compact?: boolean } = $props();

  let cat = $state<RunnerCatalog | null>(null);
  let error = $state("");

  onMount(async () => {
    try {
      cat = await runRunners();
      const firstTool = cat.tools.find((x) => x.installed);
      if (toolsOnly && value.kind !== "tool" && firstTool) value = { kind: "tool", id: firstTool.id, permission: "accept_edits" };
      else if (!toolsOnly && value.kind === "agent" && !cat.roster.some((a) => a.id === value.id) && cat.roster[0])
        value = { kind: "agent", id: cat.roster[0].id };
    } catch (e) {
      error = String(e);
    }
  });

  let key = $derived(runnerKey(value));
  let tool = $derived(value.kind === "tool" ? cat?.tools.find((x) => x.id === value.id) : undefined);
  let bypassFlag = $derived(tool?.permission_flags?.bypass?.join(" ") ?? "");
  let levels = $derived(
    ["default", "plan", "accept_edits", "bypass"].filter(
      (l) => l === "default" || (tool?.permission_flags && tool.permission_flags[l]),
    ),
  );

  function pick(k: string) {
    const r = parseRunnerKey(k);
    if (r.kind === "tool") r.permission = value.permission ?? "accept_edits";
    r.model = value.model ?? null;
    value = r;
  }
</script>

<div class="runner" class:compact>
  <label class="field grow">
    <span class="field-label">{$t("llm.central.run.runner.label")}</span>
    <select class="input" value={key} onchange={(e) => pick((e.currentTarget as HTMLSelectElement).value)} disabled={!cat}>
      {#if cat}
        {#if !toolsOnly && cat.roster.length}
          <optgroup label={$t("llm.central.run.runner.roster")}>
            {#each cat.roster as a (a.id)}
              <option value={runnerKey({ kind: "agent", id: a.id })}>{a.name}</option>
            {/each}
          </optgroup>
        {/if}
        {#if !toolsOnly}
          <optgroup label={$t("llm.central.run.runner.accounts")}>
            {#each ["claude", "codex"] as cli (cli)}
              <option value={runnerKey({ kind: "account", id: cli, account: "" })}>
                {cli === "claude" ? "Claude Code" : "Codex"} · {$t("llm.central.run.runner.default_login")}
              </option>
            {/each}
            {#each cat.accounts.filter((a) => !a.disabled) as a (a.id)}
              <option value={runnerKey({ kind: "account", id: a.cli, account: a.id })}>{a.cli} · {a.label || a.id}</option>
            {/each}
          </optgroup>
          {#if cat.acp.some((a) => a.available)}
            <optgroup label={$t("llm.central.run.runner.acp")}>
              {#each cat.acp.filter((a) => a.available) as a (a.id)}
                <option value={runnerKey({ kind: "acp", id: a.id })}>{a.name}</option>
              {/each}
            </optgroup>
          {/if}
        {/if}
        <optgroup label={$t("llm.central.run.runner.tools")}>
          {#each cat.tools as x (x.id)}
            <option value={runnerKey({ kind: "tool", id: x.id })} disabled={!x.installed}>
              {x.name}{x.installed ? "" : ` (${$t("llm.central.run.runner.not_installed")})`}
            </option>
          {/each}
        </optgroup>
      {/if}
    </select>
  </label>
  {#if value.kind === "tool" || value.kind === "account"}
    <label class="field small">
      <span class="field-label">{$t("llm.central.run.runner.model")}</span>
      <input class="input" type="text" placeholder={value.id === "claude" ? "haiku" : ""} bind:value={value.model} />
    </label>
  {/if}
  {#if value.kind === "tool"}
    <label class="field small">
      <span class="field-label">{$t("llm.central.run.runner.permission")}</span>
      <select class="input" bind:value={value.permission}>
        {#each levels as l (l)}
          <option value={l}>{$t(`llm.central.run.permission.${l}`)}</option>
        {/each}
      </select>
    </label>
  {/if}
</div>
{#if value.kind === "tool" && value.permission === "bypass"}
  <p class="warn">{$t("llm.central.run.permission.bypass_warning", { flag: bypassFlag || "—" })}</p>
{/if}
{#if tool && !tool.system_prompt_flag}
  <p class="hint">{$t("llm.central.run.runner.no_system_flag", { tool: tool.name })}</p>
{/if}
{#if error}<p class="warn">{error}</p>{/if}

<style>
  .runner {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: flex-end;
  }
  .grow {
    flex: 1;
    min-width: 220px;
  }
  .small {
    width: 150px;
  }
  .compact .grow {
    min-width: 180px;
  }
  .hint,
  .warn {
    margin: 4px 0 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .warn {
    color: var(--warning);
  }
</style>
