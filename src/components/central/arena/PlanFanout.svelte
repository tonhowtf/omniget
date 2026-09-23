<script lang="ts">
  // Botão do cartão de plano: "Implementar em N ferramentas". Abre a Arena
  // com o plano aprovado da thread como prompt (a Arena lê o plano pelo
  // `threads_turns_page`). Feito para ser montado no PlanCard das threads.
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import Icon from "$components/central/threads/Icon.svelte";

  let {
    threadId,
    planId = null,
    compact = false,
  }: { threadId: string; planId?: string | null; compact?: boolean } = $props();

  function open() {
    const q = new URLSearchParams({ thread: threadId });
    if (planId) q.set("plan", planId);
    void goto(`/llm/arena?${q.toString()}`);
  }
</script>

<button type="button" class="fanout" class:compact onclick={open} title={$t("llm.central.arena.plan.hint")}>
  <Icon name="arrows-split" size={13} />
  {#if !compact}{$t("llm.central.arena.plan.implement_n")}{/if}
</button>

<style>
  .fanout { display: inline-flex; align-items: center; gap: 5px; border: 1px solid color-mix(in srgb, var(--purple) 40%, var(--separator)); background: transparent; color: var(--purple); font-size: var(--text-sm); padding: 4px 10px; border-radius: 999px; cursor: pointer; }
  .fanout:hover { background: color-mix(in srgb, var(--purple) 10%, transparent); }
  .fanout.compact { padding: 4px 6px; }
</style>
