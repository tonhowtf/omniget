<script lang="ts">
  // Proposed plan card: title from the first heading, 10-line preview for
  // long plans, copy / download, and (when actionable) Implement, Refine or
  // Implement in a new thread.
  import { t } from "$lib/i18n";
  import type { PlanRow } from "$lib/central/threads/types";
  import Markdown from "./Markdown.svelte";
  import Icon from "./Icon.svelte";
  import PlanFanout from "$components/central/arena/PlanFanout.svelte";

  let {
    plan,
    actionable = false,
    onimplement,
    onrefine,
    onimplementnew,
    onopenfile,
  }: {
    plan: PlanRow;
    actionable?: boolean;
    onimplement?: (plan: PlanRow) => void;
    onrefine?: (plan: PlanRow) => void;
    onimplementnew?: (plan: PlanRow) => void;
    onopenfile?: (path: string) => void;
  } = $props();

  let expanded = $state(false);
  let copied = $state(false);
  let menu = $state(false);

  let md = $derived(plan.markdown ?? "");
  let title = $derived(/^\s*#{1,6}\s+(.+)$/m.exec(md)?.[1]?.trim() ?? ($t("llm.central.threads.plan.title") as string));
  let body = $derived(md.replace(/^\s*#{1,6}\s+.+\n?/, "").replace(/^\s*#{1,6}\s+summary\s*\n/i, ""));
  let long = $derived(body.length > 900 || body.split("\n").length > 20);
  let shown = $derived(long && !expanded ? body.split("\n").slice(0, 10).join("\n") + "\n\n…" : body);

  async function copy() {
    try {
      await navigator.clipboard.writeText(md);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      /* clipboard denied */
    }
    menu = false;
  }

  function download() {
    const slug = title.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, "-").replace(/^-|-$/g, "").slice(0, 60) || "plan";
    const url = URL.createObjectURL(new Blob([md], { type: "text/markdown" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = `${slug}.md`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    menu = false;
  }
</script>

<article class="plan" aria-label={title}>
  <header>
    <span class="badge"><Icon name="list-checks" size={12} />{$t("llm.central.threads.plan.badge")}</span>
    <h3>{title}</h3>
    <div class="menu-wrap">
      <button type="button" class="icon" aria-label={$t("llm.central.threads.more")} aria-expanded={menu} onclick={() => (menu = !menu)}><Icon name="dots-three" size={16} /></button>
      {#if menu}
        <div class="menu" role="menu">
          <button type="button" role="menuitem" onclick={copy}><Icon name="copy" size={13} />{copied ? $t("llm.central.threads.copied") : $t("llm.central.threads.plan.copy")}</button>
          <button type="button" role="menuitem" onclick={download}><Icon name="arrow-down" size={13} />{$t("llm.central.threads.plan.download")}</button>
        </div>
      {/if}
    </div>
  </header>
  <div class="body" class:faded={long && !expanded}><Markdown text={shown} {onopenfile} /></div>
  {#if long}
    <button type="button" class="more" onclick={() => (expanded = !expanded)}>{expanded ? $t("llm.central.threads.plan.collapse") : $t("llm.central.threads.plan.expand")}</button>
  {/if}
  {#if actionable}
    <footer>
      <button type="button" class="button primary" onclick={() => onimplement?.(plan)}><Icon name="play" size={13} />{$t("llm.central.threads.plan.implement")}</button>
      <button type="button" class="button" onclick={() => onimplementnew?.(plan)}><Icon name="arrows-split" size={13} />{$t("llm.central.threads.plan.implement_new")}</button>
      <PlanFanout threadId={plan.threadId} planId={plan.planId} />
      <button type="button" class="button" onclick={() => onrefine?.(plan)}><Icon name="pencil-simple" size={13} />{$t("llm.central.threads.plan.refine")}</button>
    </footer>
  {/if}
</article>

<style>
  .plan { border: 1px solid var(--separator); border-radius: 18px; background: color-mix(in srgb, var(--surface, #fff) 70%, transparent); padding: 14px 16px; display: grid; gap: 10px; }
  header { display: flex; align-items: center; gap: 10px; }
  h3 { margin: 0; font-size: 15px; font-weight: 650; flex: 1; min-width: 0; }
  .badge { display: inline-flex; align-items: center; gap: 4px; font-size: 11px; font-weight: 600; color: var(--purple, #8b5cf6); background: color-mix(in srgb, var(--purple, #8b5cf6) 14%, transparent); padding: 2px 8px; border-radius: 999px; }
  .menu-wrap { position: relative; }
  .icon { border: 0; background: transparent; color: var(--text-muted); border-radius: 6px; padding: 3px; cursor: pointer; display: inline-flex; }
  .icon:hover { background: var(--fill-2); }
  .menu { position: absolute; right: 0; top: 100%; z-index: 10; background: var(--popup-bg, var(--surface)); border: 1px solid var(--separator); border-radius: 10px; padding: 4px; box-shadow: 0 10px 30px color-mix(in srgb, #000 20%, transparent); display: grid; min-width: 190px; }
  .menu button { display: flex; align-items: center; gap: 8px; border: 0; background: transparent; color: var(--text); padding: 6px 8px; font-size: 12.5px; border-radius: 6px; cursor: pointer; text-align: left; }
  .menu button:hover { background: var(--accent-soft); }
  .body { position: relative; }
  .body.faded::after { content: ""; position: absolute; inset: auto 0 0 0; height: 48px; background: linear-gradient(transparent, var(--surface, #fff)); pointer-events: none; }
  .more { justify-self: start; border: 0; background: transparent; color: var(--accent-hi); font-size: 12.5px; cursor: pointer; padding: 0; }
  footer { display: flex; gap: 8px; flex-wrap: wrap; }
  footer .button { display: inline-flex; align-items: center; gap: 6px; }
</style>
