<script lang="ts">
  /** Modelos locais com Ollama (estudo 18): detectar, listar, baixar, remover. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtBytes, onToolProgress, openUrl, pct, type ToolProgress } from "$lib/tools/rt";

  type Model = { name: string; size: number; modified_at: string; details: { family: string; parameter_size: string; quantization_level: string }; capabilities: string[] };
  type Status = { running: boolean; version: string | null; host: string; models: Model[]; loaded: string[]; download_url: string };
  type Rec = { name: string; size_gb: number; use_case: string };

  let host = $state("");
  let status = $state<Status | null>(null);
  let recs = $state<Rec[]>([]);
  let custom = $state("");
  let busy = $state<string | null>(null);
  let progress = $state<Record<string, ToolProgress>>({});
  let unlisten: (() => void) | null = null;

  async function refresh() {
    try {
      status = await invoke<Status>("tool_ollama_status", { host: host || null });
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function pull(name: string) {
    if (busy || !name.trim()) return;
    busy = `pull:${name}`;
    try {
      await invoke("tool_ollama_pull", { host: host || null, name: name.trim() });
      showToast("success", `${name} ${$t("tools.common.installed")}`);
      custom = "";
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
      await refresh();
    }
  }

  async function remove(name: string) {
    if (busy) return;
    busy = `rm:${name}`;
    try {
      await invoke("tool_ollama_delete", { host: host || null, name });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
      await refresh();
    }
  }

  onMount(async () => {
    recs = await invoke<Rec[]>("tool_ollama_recommended");
    await refresh();
    unlisten = await onToolProgress((p) => {
      if (p.id.startsWith("ollama-pull:")) progress = { ...progress, [p.id.slice(12)]: p };
    });
  });
  onDestroy(() => unlisten?.());

  let installedNames = $derived(new Set(status?.models.map((m) => m.name) ?? []));
</script>

<div class="tool">
  <section>
    <span class="group-label">Ollama</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.ollama.daemon")}</div>
          <div class="group-row-sub">
            {#if !status}…{:else if status.running}<span class="tag tag-success">{$t("tools.ollama.running")}</span> v{status.version} · <span class="mono">{status.host}</span>{:else}<span class="tag tag-warning">{$t("tools.ollama.stopped")}</span> {$t("tools.ollama.stopped_hint")}{/if}
          </div>
        </div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="text" bind:value={host} placeholder="http://127.0.0.1:11434" style:width="14em" onchange={refresh} />
          {#if status && !status.running}
            <button class="btn btn-primary btn-sm" type="button" onclick={() => openUrl(status!.download_url)}>{$t("tools.ollama.get")}</button>
          {/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={refresh}>{$t("tools.common.refresh")}</button>
        </div>
      </div>
      {#if status?.running}
        <div class="group-row"><div class="group-row-sub">{$t("tools.ollama.use_hint")} <span class="mono">{status.host}/v1</span></div></div>
      {/if}
    </div>
  </section>

  {#if status?.running}
    <section>
      <span class="group-label">{$t("tools.ollama.installed")} ({status.models.length})</span>
      <div class="group">
        {#if status.models.length === 0}
          <div class="group-row"><div class="group-row-sub">{$t("tools.ollama.none")}</div></div>
        {/if}
        {#each status.models as m (m.name)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{m.name} {#if status.loaded.includes(m.name)}<span class="tag tag-success">{$t("tools.ollama.loaded")}</span>{/if}</div>
              <div class="group-row-sub">{fmtBytes(m.size)} · {m.details.family} {m.details.parameter_size} {m.details.quantization_level} {#if m.capabilities?.length}· {m.capabilities.join(", ")}{/if}</div>
            </div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" disabled={busy !== null} onclick={() => remove(m.name)}>{$t("tools.common.remove")}</button></div>
          </div>
        {/each}
      </div>
    </section>

    <section>
      <span class="group-label">{$t("tools.ollama.recommended")}</span>
      <div class="group">
        {#each recs as r (r.name)}
          {@const p = progress[r.name]}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{r.name} <span class="dim">· ~{r.size_gb} GB</span></div>
              <div class="group-row-sub">{r.use_case}</div>
              {#if busy === `pull:${r.name}` && p}
                <div class="group-row-sub">{p.message}</div>
                <div class="progress"><div class="progress-fill" style:width="{pct(p) ?? 0}%"></div></div>
              {/if}
            </div>
            <div class="group-row-trailing">
              {#if installedNames.has(r.name)}<span class="tag tag-success">{$t("tools.common.installed")}</span>
              {:else}<button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={() => pull(r.name)}>{busy === `pull:${r.name}` ? $t("tools.common.downloading") : $t("tools.common.download")}</button>{/if}
            </div>
          </div>
        {/each}
        <div class="group-row">
          <div class="group-row-content">
            <input class="input" type="text" bind:value={custom} placeholder={$t("tools.ollama.custom_placeholder")} />
            {#if custom && progress[custom] && busy === `pull:${custom}`}
              <div class="progress"><div class="progress-fill" style:width="{pct(progress[custom]) ?? 0}%"></div></div>
            {/if}
          </div>
          <div class="group-row-trailing"><button class="btn btn-primary btn-sm" type="button" disabled={busy !== null || !custom.trim()} onclick={() => pull(custom)}>{$t("tools.common.download")}</button></div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); }
</style>
