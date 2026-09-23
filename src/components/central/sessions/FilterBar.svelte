<script lang="ts">
  /**
   * Uma linha de filtros acima de tudo o que eles escopam: período primeiro
   * (7D/30D/90D/ano/tudo), depois ferramenta, conta, projeto e modelo.
   * Seletores nativos: teclado e leitor de tela de graça.
   */
  import { t } from "$lib/i18n";
  import { RANGE_KEYS, type RangeKey } from "$lib/central/sessions";

  type Opt = { value: string; label: string };

  interface Props {
    rangeKey?: RangeKey;
    tool?: string;
    account?: string;
    project?: string;
    model?: string;
    tools?: Opt[];
    accounts?: Opt[];
    projects?: Opt[];
    models?: Opt[];
    showRange?: boolean;
  }

  let {
    rangeKey = $bindable("30d"),
    tool = $bindable(""),
    account = $bindable(""),
    project = $bindable(""),
    model = $bindable(""),
    tools = [],
    accounts = [],
    projects = [],
    models = [],
    showRange = true,
  }: Props = $props();

  function withSelected(opts: Opt[], value: string): Opt[] {
    if (!value || opts.some((o) => o.value === value)) return opts;
    return [{ value, label: value }, ...opts];
  }

  let anyDim = $derived(!!(tool || account || project || model));

  function clear() {
    tool = "";
    account = "";
    project = "";
    model = "";
  }
</script>

<div class="filters" role="group" aria-label={$t("llm.central.sessions.filters")}>
  {#if showRange}
    <div class="mac-segmented" role="radiogroup" aria-label={$t("llm.central.sessions.period")}>
      {#each RANGE_KEYS as k (k)}
        <button type="button" class="mac-segmented-btn" class:active={rangeKey === k} role="radio" aria-checked={rangeKey === k} onclick={() => (rangeKey = k)}>
          {$t(`llm.central.sessions.range.${k}`)}
        </button>
      {/each}
    </div>
  {/if}
  <label class="sel">
    <span class="sr">{$t("llm.central.sessions.tool")}</span>
    <select class="input" bind:value={tool}>
      <option value="">{$t("llm.central.sessions.all_tools")}</option>
      {#each withSelected(tools, tool) as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
    </select>
  </label>
  {#if accounts.length > 1 || account}
    <label class="sel">
      <span class="sr">{$t("llm.central.sessions.account")}</span>
      <select class="input" bind:value={account}>
        <option value="">{$t("llm.central.sessions.all_accounts")}</option>
        {#each withSelected(accounts, account) as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
      </select>
    </label>
  {/if}
  <label class="sel">
    <span class="sr">{$t("llm.central.sessions.project")}</span>
    <select class="input" bind:value={project}>
      <option value="">{$t("llm.central.sessions.all_projects")}</option>
      {#each withSelected(projects, project) as o (o.value)}<option value={o.value} title={o.value}>{o.label}</option>{/each}
    </select>
  </label>
  <label class="sel">
    <span class="sr">{$t("llm.central.sessions.model")}</span>
    <select class="input" bind:value={model}>
      <option value="">{$t("llm.central.sessions.all_models")}</option>
      {#each withSelected(models, model) as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
    </select>
  </label>
  {#if anyDim}
    <button type="button" class="btn btn-ghost btn-sm" onclick={clear}>{$t("llm.central.sessions.clear_filters")}</button>
  {/if}
</div>

<style>
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }
  .mac-segmented {
    max-width: 100%;
    overflow-x: auto;
  }
  .sel select {
    height: 28px;
    max-width: 200px;
    padding-block: 0;
    font-size: var(--text-sm);
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  @media (max-width: 640px) {
    .mac-segmented {
      width: 100%;
      overflow-x: auto;
    }
    .sel {
      flex: 1 1 140px;
    }
    .sel select {
      max-width: none;
      width: 100%;
    }
  }
</style>
