<script lang="ts">
  /** Grok (estudo 67): API oficial da xAI com busca no X, ou o Grok do X pela sessão. */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { openUrl } from "$lib/tools/rt";
  import { postIdFrom, xErr, type XPost, type XSession } from "$lib/tools/x";
  import XSessionRow from "./XSession.svelte";

  type Cfg = { has_xai_key: boolean; xai_model: string; x_model: string };
  type Answer = { text: string; citations: { url: string; title: string }[]; model: string; backend: string; input_tokens: number; output_tokens: number };

  let cfg = $state<Cfg | null>(null);
  let sess = $state<XSession | null>(null);
  let keyInput = $state("");
  let xaiModel = $state("");
  let xModel = $state("");
  let backend = $state<"auto" | "xai" | "x">("auto");
  let xSearch = $state(true);
  let webSearch = $state(false);
  let handles = $state("");
  let fromDate = $state("");
  let toDate = $state("");
  let prompt = $state("");
  let context = $state("");
  let contextUrl = $state("");
  let busy = $state<string | null>(null);
  let answer = $state<Answer | null>(null);
  let showSettings = $state(false);

  async function load() {
    cfg = await invoke<Cfg>("tool_x_grok_config");
    xaiModel = cfg.xai_model;
    xModel = cfg.x_model;
    showSettings = !cfg.has_xai_key;
  }

  async function saveCfg(clearKey = false) {
    busy = "cfg";
    try {
      cfg = await invoke<Cfg>("tool_x_grok_config_set", { xaiKey: clearKey ? "" : keyInput.trim() || null, xaiModel, xModel });
      keyInput = "";
      showToast("success", $t("tools.common.done") as string);
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = null;
    }
  }

  async function loadContext() {
    const id = postIdFrom(contextUrl);
    if (!id) return;
    busy = "ctx";
    try {
      const th = await invoke<{ posts: XPost[] }>("tool_x_thread", { input: contextUrl });
      context = th.posts.map((p, i) => `[${i + 1}] @${p.author.handle}: ${p.text}`).join("\n\n");
      if (!prompt.trim()) prompt = $t("tools.x.grok_summarize_prompt") as string;
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = null;
    }
  }

  async function ask() {
    if (!prompt.trim() || busy) return;
    busy = "ask";
    answer = null;
    try {
      const full = context.trim() ? `${prompt.trim()}\n\n---\n${context.trim()}` : prompt.trim();
      answer = await invoke<Answer>("tool_x_grok_ask", {
        request: { prompt: full, system: "", backend, x_search: xSearch, web_search: webSearch, handles: handles.split(/[\s,]+/).filter(Boolean), from_date: fromDate, to_date: toDate, model: "" },
      });
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = null;
    }
  }

  async function copy() {
    if (!answer) return;
    await navigator.clipboard.writeText(answer.text);
    showToast("success", $t("tools.common.copied") as string);
  }

  onMount(load);
  let effective = $derived(backend === "auto" ? (cfg?.has_xai_key ? "xai" : "x") : backend);
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.x.grok_backend")}</div>
          <div class="group-row-sub">{effective === "xai" ? $t("tools.x.grok_backend_xai") : $t("tools.x.grok_backend_x")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <div class="segmented">{#each ["auto", "xai", "x"] as b (b)}<button class="segmented-btn" class:active={backend === b} type="button" onclick={() => (backend = b as typeof backend)}>{b === "auto" ? $t("tools.x.grok_auto") : b === "xai" ? "xAI API" : "X"}</button>{/each}</div>
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => (showSettings = !showSettings)}>{$t("tools.x.grok_settings")}</button>
        </div>
      </div>
      {#if showSettings}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">xAI API key</div><div class="group-row-sub">{cfg?.has_xai_key ? $t("tools.x.grok_key_set") : $t("tools.x.grok_key_hint")} <button class="link" type="button" onclick={() => openUrl("https://console.x.ai/")}>console.x.ai</button></div></div>
          <div class="group-row-trailing btn-row">
            <input class="input" type="password" bind:value={keyInput} placeholder="xai-…" autocomplete="off" />
            {#if cfg?.has_xai_key}<button class="btn btn-ghost btn-sm" type="button" onclick={() => saveCfg(true)}>{$t("tools.common.remove")}</button>{/if}
            <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={() => saveCfg()}>{$t("tools.common.save")}</button>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.x.grok_models")}</div><div class="group-row-sub">{$t("tools.x.grok_models_hint")}</div></div>
          <div class="group-row-trailing btn-row">
            <label class="opt">xAI <input class="input" type="text" bind:value={xaiModel} /></label>
            <label class="opt">X <input class="input" type="text" bind:value={xModel} /></label>
          </div>
        </div>
      {/if}
      {#if effective === "x"}<XSessionRow required onchange={(s) => (sess = s)} />{/if}
    </div>
  </section>

  <section>
    <div class="group">
      {#if effective === "xai"}
        <div class="group-row">
          <div class="group-row-content btn-row wrap">
            <label class="opt"><input class="checkbox" type="checkbox" bind:checked={xSearch} /> {$t("tools.x.grok_x_search")}</label>
            <label class="opt"><input class="checkbox" type="checkbox" bind:checked={webSearch} /> {$t("tools.x.grok_web_search")}</label>
            {#if xSearch}
              <input class="input" type="text" bind:value={handles} placeholder={$t("tools.x.grok_handles")} />
              <input class="input date" type="date" bind:value={fromDate} title="from" />
              <input class="input date" type="date" bind:value={toDate} title="to" />
            {/if}
          </div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.x.grok_context")}</div>
          <div class="group-row-sub">{$t("tools.x.grok_context_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="url" bind:value={contextUrl} placeholder={$t("tools.x.post_placeholder")} onkeydown={(e) => e.key === "Enter" && loadContext()} />
          <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null || !postIdFrom(contextUrl)} onclick={loadContext}>{busy === "ctx" ? "…" : $t("tools.x.load_post")}</button>
          {#if context}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (context = "")}>×</button>{/if}
        </div>
      </div>
      {#if context}
        <div class="group-row"><div class="group-row-content"><div class="ctx">{context}</div></div></div>
      {/if}
      <div class="group-row">
        <div class="group-row-content"><textarea class="input area" rows="4" bind:value={prompt} placeholder={$t("tools.x.grok_prompt")} onkeydown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && ask()}></textarea></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-sub">{effective === "xai" ? cfg?.xai_model : cfg?.x_model} · ⌘↩</div></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy !== null || !prompt.trim() || (effective === "x" && !sess?.logged_in) || (effective === "xai" && !cfg?.has_xai_key)} onclick={ask}>{busy === "ask" ? $t("tools.common.working") : $t("tools.x.grok_ask")}</button></div>
      </div>
    </div>
  </section>

  {#if answer}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="answer">{answer.text}</div></div>
        </div>
        {#if answer.citations.length}
          <div class="group-row"><div class="group-row-content cites">
            {#each answer.citations as c (c.url)}<button class="tag chip" type="button" title={c.url} onclick={() => openUrl(c.url)}>{c.title || c.url.replace(/^https?:\/\//, "").slice(0, 60)}</button>{/each}
          </div></div>
        {/if}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-sub">{answer.backend} · {answer.model}{#if answer.input_tokens} · {answer.input_tokens} in / {answer.output_tokens} out{/if}</div></div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={copy}>{$t("tools.common.copy")}</button></div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .opt { display: inline-flex; align-items: center; gap: var(--space-1); font-size: var(--text-sm); white-space: nowrap; }
  .wrap { flex-wrap: wrap; gap: var(--space-2); }
  .area { width: 100%; resize: vertical; }
  .date { max-width: 150px; }
  .ctx { max-height: 160px; overflow: auto; white-space: pre-wrap; font-size: var(--text-xs); color: var(--text-muted); font-family: var(--font-mono); }
  .answer { white-space: pre-wrap; line-height: 1.5; color: var(--text); }
  .cites { display: flex; flex-wrap: wrap; gap: var(--space-1); }
  .chip { cursor: pointer; border: 0; font: inherit; font-size: var(--text-xs); }
  .link { background: none; border: 0; color: var(--accent-hi); cursor: pointer; font: inherit; padding: 0; }
</style>
