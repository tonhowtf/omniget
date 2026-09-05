<script lang="ts">
  /**
   * Ideias de palavras-chave (estudo 67): typeahead do Pinterest, os
   * refinamentos da barra de busca e as palavras mais comuns nos pins do
   * topo. Para quem cria pins e quer ser encontrado.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, openUrl } from "$lib/tools/rt";
  import type { KeywordsOut } from "$lib/tools/pinterest";

  let term = $state("");
  let busy = $state(false);
  let out = $state<KeywordsOut | null>(null);
  let history = $state<KeywordsOut[]>([]);

  async function run(t0?: string) {
    const q = (t0 ?? term).trim();
    if (!q || busy) return;
    term = q; busy = true;
    try {
      out = await invoke<KeywordsOut>("tool_pin_keywords", { term: q, cookies: null });
      if (!history.some((h) => h.term === out!.term)) history = [out!, ...history].slice(0, 12);
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; }
  }
  async function copyAll() {
    if (!out) return;
    const text = [...out.suggestions, ...out.guides, ...out.common.map(([w]) => w)].filter((v, i, a) => a.indexOf(v) === i).join("\n");
    await navigator.clipboard.writeText(text);
    showToast("success", $t("tools.common.copied") as string);
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="text" bind:value={term} placeholder={$t("tools.pinterest.term_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} /></div>
        <div class="group-row-trailing btn-row">
          {#if out}<button class="btn btn-ghost btn-sm" type="button" onclick={copyAll}>{$t("tools.common.copy")}</button>{/if}
          <button class="btn btn-primary" type="button" disabled={busy || !term.trim()} onclick={() => run()}>{busy ? $t("tools.common.working") : $t("tools.pinterest.search")}</button>
        </div>
      </div>
      {#if history.length > 1}
        <div class="group-row"><div class="chips">{#each history as h (h.term)}<button class="btn btn-ghost btn-sm" class:on={h.term === out?.term} type="button" onclick={() => (out = h)}>{h.term}</button>{/each}</div></div>
      {/if}
    </div>
  </section>

  {#if out}
    {#if out.suggestions.length}
      <section>
        <span class="group-label">{$t("tools.pinterest.suggestions")}</span>
        <div class="group"><div class="group-row"><div class="chips">{#each out.suggestions as s (s)}<button class="btn btn-secondary btn-sm" type="button" onclick={() => run(s)}>{s}</button>{/each}</div></div></div>
      </section>
    {/if}
    {#if out.guides.length}
      <section>
        <span class="group-label">{$t("tools.pinterest.guides")}</span>
        <div class="group"><div class="group-row"><div class="chips">{#each out.guides as g (g)}<button class="btn btn-secondary btn-sm" type="button" onclick={() => run(`${out!.term} ${g}`)}>{g}</button>{/each}</div></div></div>
      </section>
    {/if}
    {#if out.common.length}
      <section>
        <span class="group-label">{$t("tools.pinterest.common")} · {out.sample} {$t("tools.pinterest.sample")}</span>
        <div class="group"><div class="group-row"><div class="cloud">{#each out.common as [w, n] (w)}<button class="btn btn-ghost btn-sm" type="button" style:font-size="{Math.min(20, 11 + n)}px" onclick={() => run(w.startsWith("#") ? w.slice(1) : w)}>{w} <span class="dim">{n}</span></button>{/each}</div></div></div>
      </section>
    {/if}
    <section>
      <div class="group"><div class="group-row"><div class="group-row-content"><div class="group-row-sub">{out.term}</div></div><div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => openUrl(`https://www.pinterest.com/search/pins/?q=${encodeURIComponent(out!.term)}`)}>{$t("tools.pinterest.open_pinterest")}</button></div></div></div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .chips, .cloud { display: flex; flex-wrap: wrap; gap: var(--space-1); align-items: baseline; }
  .on { background: var(--fill-1); }
  .dim { color: var(--text-dim); font-size: var(--text-xs); }
</style>
