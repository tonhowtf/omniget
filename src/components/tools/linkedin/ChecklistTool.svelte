<script lang="ts">
  /** Score offline de completude do perfil a partir do export oficial. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, pct, pickDir, pickFile, type ToolProgress } from "$lib/tools/rt";

  type Item = { id: string; state: string; weight: number; value: string; target: string; source: string };
  type Result = { name: string; headline: string; score: number; max: number; percent: number; items: Item[]; missing: string[]; unknown: string[] };

  let path = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const ZIP = [{ name: "Zip", extensions: ["zip"] }];
  const sorted = $derived(result ? [...result.items].sort((a, b) => (a.state === b.state ? 0 : a.state === "miss" ? -1 : b.state === "miss" ? 1 : a.state === "warn" ? 1 : -1)) : []);

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "li-checklist") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!path || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_li_checklist", { opts: { path } });
      showToast("success", `${result.score}/${result.max}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.licheck.source")}</div>
          <div class="group-row-sub mono">{path || $t("tools.licheck.source_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(ZIP); if (f) path = f; }}>{$t("tools.licheck.pick_zip")}</button>
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) path = d; }}>{$t("tools.licheck.pick_folder")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !path} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.licheck.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title score">{result.percent}%</div>
            <div class="group-row-sub">{result.score} / {result.max} · {result.name}</div>
            <div class="bar"><div class="bar-fill" style:width="{result.percent}%"></div></div>
            {#if result.headline}<div class="group-row-sub">{result.headline}</div>{/if}
          </div>
        </div>
      </div>
    </section>

    <section>
      <div class="group">
        {#each sorted as it (it.id)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">
                {$t(`tools.licheck.item_${it.id}`)}
                {#if it.state === "ok"}<span class="tag tag-success">{it.weight}</span>{:else if it.state === "warn"}<span class="tag">?</span>{:else}<span class="tag">{it.weight}</span>{/if}
              </div>
              {#if it.state === "miss"}<div class="group-row-sub">{$t(`tools.licheck.tip_${it.id}`)}</div>{/if}
              {#if it.state === "warn"}<div class="group-row-sub">{$t("tools.licheck.unknown")}</div>{/if}
              <div class="group-row-sub mono">{it.source}{#if it.value} · {it.value}{/if}{#if it.target} / {it.target}{/if}</div>
            </div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .score { font-size: var(--text-2xl, 28px); }
  .bar { height: 8px; border-radius: 4px; background: rgba(127, 127, 127, 0.18); overflow: hidden; margin: var(--space-2) 0; max-width: 420px; }
  .bar-fill { height: 100%; background: #0a66c2; }
</style>
