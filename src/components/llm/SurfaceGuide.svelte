<script lang="ts">
  import { t } from "$lib/i18n";
  let { text, steps = [], href = "/help", label, collapsed = false }: { text: string; steps?: string[]; href?: string; label?: string; collapsed?: boolean } = $props();
</script>
{#snippet body()}
  <div><p>{text}</p>{#if steps.length}<ol>{#each steps as step}<li>{step}</li>{/each}</ol>{/if}</div>
  <a href={href}>{label ?? $t("llm.surface.help")}<svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="m9 5 7 7-7 7"/></svg></a>
{/snippet}
{#if collapsed}
  <details class="surface-guide-disclosure">
    <summary>{label ?? $t("llm.surface.help")}</summary>
    <aside class="surface-guide">{@render body()}</aside>
  </details>
{:else}
  <aside class="surface-guide">{@render body()}</aside>
{/if}
<style>
.surface-guide { display:flex; align-items:flex-start; justify-content:space-between; gap:24px; padding:16px 0 24px; margin-bottom:16px; border-bottom:1px solid var(--separator); }
p { margin:0; max-width:72ch; color:var(--text-muted); font-size:14px; line-height:1.6; }
a { display:inline-flex; align-items:center; gap:6px; flex-shrink:0; min-height:40px; color:var(--accent-text); font-size:13px; text-decoration:none; border-radius:var(--radius-sm); }
a:hover { text-decoration:underline; } a:focus-visible { outline:var(--focus-ring); outline-offset:3px; }
ol { display:flex; flex-wrap:wrap; gap:12px 32px; margin:20px 0 0; padding-left:20px; font-size:13px; color:var(--text); }
.surface-guide-disclosure { margin-bottom:16px; }
.surface-guide-disclosure summary { cursor:pointer; min-height:32px; display:flex; align-items:center; color:var(--text-muted); font-size:13px; border-radius:var(--radius-sm); }
.surface-guide-disclosure summary:focus-visible { outline:var(--focus-ring); outline-offset:2px; }
.surface-guide-disclosure .surface-guide { padding-top:8px; }
@media(max-width:700px) { .surface-guide { flex-direction:column; gap:8px; } ol { flex-direction:column; } }
</style>
