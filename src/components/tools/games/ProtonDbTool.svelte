<script lang="ts">
  /** Tier do ProtonDB por appid, link da loja, nome ou biblioteca Steam inteira. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openUrl, pct, pickDir, saveAs, type ToolProgress } from "$lib/tools/rt";

  type Entry = {
    app_id: number;
    name: string;
    tier: string;
    confidence: string;
    score: number;
    total: number;
    trending_tier: string;
    best_tier: string;
    rank: number;
    url: string;
    cached: boolean;
    ok: boolean;
    error: string | null;
  };
  type Result = { entries: Entry[]; checked: number; from_cache: number; unresolved: string[]; exported: string | null };

  const TIERS = ["platinum", "gold", "silver", "bronze", "borked", "pending"];
  /** Cor do degrau. Fica em JS porque a classe é montada em tempo de execução. */
  const TIER_COLOR: Record<string, [string, string]> = {
    platinum: ["#d5dbe6", "#17181c"],
    gold: ["#ffcc4d", "#17181c"],
    silver: ["#c3c8d1", "#17181c"],
    bronze: ["#d1954b", "#17181c"],
    borked: ["#ff6b60", "#ffffff"],
    pending: ["var(--surface-hi)", "var(--text-dim)"],
  };
  const tierName = (tier: string) => `tools.protondb.tier_${TIERS.includes(tier) ? tier : "pending"}`;
  const tierBg = (tier: string) => (TIER_COLOR[tier] ?? TIER_COLOR.pending)[0];
  const tierFg = (tier: string) => (TIER_COLOR[tier] ?? TIER_COLOR.pending)[1];

  let raw = $state("");
  let scanLibrary = $state(false);
  let steamDirs = $state<string[]>([]);
  let delayMs = $state(250);
  let refresh = $state(false);
  let sortBy = $state("tier");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "protondb") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  let inputs = $derived(raw.split(/[\n,]/).map((s) => s.trim()).filter(Boolean));
  let ready = $derived(inputs.length > 0 || scanLibrary);

  function opts(exportPath = "", format = "csv") {
    return {
      inputs,
      scan_library: scanLibrary,
      steam_dirs: steamDirs,
      delay_ms: delayMs,
      refresh,
      export_path: exportPath,
      export_format: format,
    };
  }

  async function run() {
    if (busy || !ready) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_protondb", { opts: opts() });
      showToast("success", `${result.entries.length} ${$t("tools.protondb.results")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  /** A segunda consulta sai do cache do dia, então exportar não repete a rede. */
  async function exportAs(format: "csv" | "md") {
    if (busy || !result) return;
    const path = await saveAs(`protondb.${format}`, [{ name: format.toUpperCase(), extensions: [format] }]);
    if (!path) return;
    busy = true;
    try {
      const r = await invoke<Result>("tool_protondb", { opts: opts(path, format) });
      result = r;
      showToast("success", $t("tools.protondb.exported") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  let rows = $derived(
    result
      ? [...result.entries].sort((a, b) => {
          if (sortBy === "name") return a.name.localeCompare(b.name);
          if (sortBy === "reports") return b.total - a.total;
          return b.rank - a.rank || b.total - a.total;
        })
      : [],
  );
  let counts = $derived.by(() => {
    const entries = result?.entries ?? [];
    return TIERS.map((tier) => ({ tier, n: entries.filter((e) => e.tier === tier).length })).filter((c) => c.n > 0);
  });
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.protondb.inputs")}</div>
          <div class="group-row-sub">{$t("tools.protondb.inputs_hint")}</div>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <textarea class="input area" bind:value={raw} rows="3" placeholder="570"></textarea>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.protondb.scan_library")}</div><div class="group-row-sub">{$t("tools.protondb.scan_library_hint")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={scanLibrary} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.protondb.steam_dirs")}</div>
          <div class="group-row-sub mono">{steamDirs.join(" · ") || $t("tools.protondb.steam_dirs_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if steamDirs.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (steamDirs = [])}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d && !steamDirs.includes(d)) steamDirs = [...steamDirs, d]; }}>{$t("tools.common.add")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.protondb.delay")}</div><div class="group-row-sub">{$t("tools.protondb.delay_hint")}</div></div>
        <div class="group-row-trailing btn-row"><input class="input" type="number" min="0" step="50" bind:value={delayMs} style:width="6em" /><span class="dim">ms</span></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.protondb.refresh")}</div><div class="group-row-sub">{$t("tools.protondb.refresh_hint")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={refresh} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-primary" type="button" disabled={busy || !ready} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.protondb.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.entries.length} {$t("tools.protondb.results")}</div>
            <div class="group-row-sub">
              {#each counts as c (c.tier)}<span class="tier" style:background={tierBg(c.tier)} style:color={tierFg(c.tier)}>{$t(tierName(c.tier))} {c.n}</span>{/each}
            </div>
          </div>
          <div class="group-row-trailing btn-row">
            <select class="input" bind:value={sortBy}>
              <option value="tier">{$t("tools.protondb.sort_tier")}</option>
              <option value="name">{$t("tools.protondb.sort_name")}</option>
              <option value="reports">{$t("tools.protondb.sort_reports")}</option>
            </select>
            <button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={() => exportAs("csv")}>CSV</button>
            <button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={() => exportAs("md")}>Markdown</button>
          </div>
        </div>
        {#if result.unresolved.length}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{$t("tools.protondb.unresolved")}</div>
              <div class="group-row-sub mono">{result.unresolved.join(" · ")}</div>
            </div>
          </div>
        {/if}
        {#if result.exported}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub mono">{result.exported}</div></div>
          </div>
        {/if}
      </div>
    </section>

    <section>
      <div class="group">
        {#each rows as e (e.app_id)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">
                <span class="tier" style:background={tierBg(e.tier)} style:color={tierFg(e.tier)}>{$t(tierName(e.tier))}</span>
                {e.name}
              </div>
              <div class="group-row-sub">
                {e.total} {$t("tools.protondb.reports")}
                {#if e.confidence}· {e.confidence}{/if}
                {#if e.trending_tier}· {$t("tools.protondb.trending")} {$t(tierName(e.trending_tier))}{/if}
                {#if e.best_tier}· {$t("tools.protondb.best")} {$t(tierName(e.best_tier))}{/if}
                {#if e.cached}<span class="tag">{$t("tools.protondb.cached")}</span>{/if}
                {#if !e.ok}<span class="tag">{e.error}</span>{/if}
              </div>
            </div>
            <div class="group-row-trailing btn-row">
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(`https://www.protondb.com/app/${e.app_id}`)}>ProtonDB</button>
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(e.url)}>Steam</button>
            </div>
          </div>
        {/each}
        {#if !rows.length}
          <div class="group-row"><div class="group-row-sub">{$t("tools.protondb.empty")}</div></div>
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .area { width: 100%; resize: vertical; font-family: var(--font-mono); font-size: var(--text-xs); }
  .tier {
    display: inline-block;
    padding: 1px 8px;
    margin-right: var(--space-2);
    border-radius: 999px;
    font-size: var(--text-xs);
    font-weight: 600;
  }
</style>
