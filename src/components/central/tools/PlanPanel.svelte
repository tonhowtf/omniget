<script lang="ts">
  // Os planos de uma ação com o comando exato à vista. Só roda o que o
  // backend marcou `runnable`; o resto fica para copiar.
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import type { Plan } from "$lib/central/clitools";

  let {
    toolName,
    plans,
    busy = false,
    onrun,
    onclose,
  }: {
    toolName: string;
    plans: Plan[];
    busy?: boolean;
    onrun: (plan: Plan) => void;
    onclose: () => void;
  } = $props();

  let selected = $state<string | null>(null);
  $effect(() => {
    if (!plans.some((p) => p.id === selected)) {
      selected = (plans.find((p) => p.recommended) ?? plans.find((p) => p.runnable) ?? plans[0])?.id ?? null;
    }
  });
  let chosen = $derived(plans.find((p) => p.id === selected) ?? null);

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast("success", $t("llm.central.tools.copied") as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }
</script>

<section class="surface-card panel" aria-label={$t("llm.central.tools.plan_title")}>
  <header class="panel-head">
    <h2>{$t(plans[0]?.action === "update" ? "llm.central.tools.plan_update" : "llm.central.tools.plan_install")} · {toolName}</h2>
    <button class="button" type="button" onclick={onclose}>{$t("llm.central.tools.close")}</button>
  </header>

  {#if plans.length === 0}
    <p class="dim">{$t("llm.central.tools.no_plans")}</p>
  {:else}
    <ul class="plans" role="radiogroup">
      {#each plans as p (p.id)}
        <li class:disabled={!p.runnable}>
          <label>
            <input class="radio" type="radio" name="plan" value={p.id} bind:group={selected} />
            <span class="method">
              {$t(`llm.central.tools.method.${p.method}`)}
              {#if p.recommended}<span class="tag tag-accent">{$t("llm.central.tools.recommended")}</span>{/if}
              {#if !p.runnable}<span class="tag">{$t("llm.central.tools.copy_only")}</span>{/if}
            </span>
          </label>
          <div class="cmd">
            <code>{p.command.display}</code>
            <button class="button" type="button" onclick={() => copy(p.command.display)}>{$t("llm.central.tools.copy")}</button>
          </div>
          {#if p.reason}<p class="reason">{p.reason}</p>{/if}
          {#each p.warnings as w}<p class="warn">{w}</p>{/each}
        </li>
      {/each}
    </ul>
    <footer class="panel-foot">
      <p class="dim">{$t("llm.central.tools.plan_hint")}</p>
      <button
        class="button active"
        type="button"
        disabled={busy || !chosen?.runnable}
        onclick={() => chosen && onrun(chosen)}
      >
        {$t("llm.central.tools.run")}
      </button>
    </footer>
  {/if}
</section>

<style>
  .panel {
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .panel-head,
  .panel-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  h2 {
    margin: 0;
    font-size: var(--text-md);
  }
  .plans {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .plans li {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .plans li.disabled .method {
    color: var(--text-dim);
  }
  label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .method {
    display: inline-flex;
    gap: 6px;
    align-items: center;
    font-weight: 600;
  }
  .cmd {
    display: flex;
    gap: var(--space-2);
    align-items: flex-start;
  }
  .cmd code {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    background: var(--fill-1);
    border-radius: var(--radius-sm);
    padding: 6px 8px;
    white-space: pre-wrap;
    word-break: break-all;
    user-select: text;
  }
  .reason,
  .warn,
  .dim {
    margin: 0;
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .warn {
    color: var(--warning);
  }
</style>
