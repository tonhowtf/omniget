<script lang="ts">
  /**
   * Plugins tab of `/llm/installed`: every plugin/extension the coding tools
   * have (Claude, Copilot CLI, Droid, Cursor, Gemini, Qwen, OpenCode), grouped
   * by tool, with an on/off switch written in the tool's own file (per scope),
   * where it came from (marketplace/source) and what it brings.
   */
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/central/catalog";
  import ToolDot from "./ToolDot.svelte";
  import {
    COMPONENT_KEYS,
    defaultScope,
    pluginSetEnabled,
    pluginsList,
    type PluginInfo,
    type PluginScope,
  } from "./plugins";

  let {
    projectDir = null,
    toolName = (id: string) => id,
    onchanged,
  }: {
    projectDir?: string | null;
    toolName?: (id: string) => string;
    onchanged?: () => void;
  } = $props();

  let plugins = $state<PluginInfo[]>([]);
  let loading = $state(false);
  let error = $state("");
  let busy = $state<string | null>(null);
  let query = $state("");
  let open = $state<Set<string>>(new Set());
  let scopes = $state<Record<string, PluginScope>>({});

  const key = (p: PluginInfo) => `${p.tool}:${p.id}:${p.scope}`;

  let filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return plugins;
    return plugins.filter((p) =>
      [p.name, p.id, p.description, p.marketplace ?? "", p.source ?? ""].some((s) => s.toLowerCase().includes(q)),
    );
  });
  let groups = $derived.by(() => {
    const m = new Map<string, PluginInfo[]>();
    for (const p of filtered) m.set(p.tool, [...(m.get(p.tool) ?? []), p]);
    return [...m.entries()];
  });
  let onCount = $derived(plugins.filter((p) => p.enabled === true).length);

  async function load() {
    loading = true;
    error = "";
    try {
      plugins = await pluginsList(projectDir);
    } catch (e) {
      error = errText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void projectDir;
    void load();
  });

  function scopeOf(p: PluginInfo): PluginScope {
    const s = scopes[key(p)];
    return s && p.toggle_scopes.includes(s) ? s : defaultScope(p, projectDir);
  }

  /** State in the scope the switch writes to (falls back to the effective one). */
  function stateIn(p: PluginInfo, s: PluginScope): boolean {
    return p.enabled_in?.[s] ?? p.enabled ?? false;
  }

  async function toggle(p: PluginInfo) {
    const s = scopeOf(p);
    const next = !stateIn(p, s);
    busy = key(p);
    try {
      const r = await pluginSetEnabled(p.tool, p.id, next, s, projectDir);
      if (r.plugin) plugins = plugins.map((x) => (key(x) === key(p) ? r.plugin! : x));
      else await load();
      showToast(
        "success",
        $t(next ? "llm.central.plugins.turned_on" : "llm.central.plugins.turned_off", {
          name: p.name,
          scope: $t(`llm.central.plugins.scope.${s}`),
        }),
      );
      onchanged?.();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }

  function toggleOpen(k: string) {
    const s = new Set(open);
    if (s.has(k)) s.delete(k);
    else s.add(k);
    open = s;
  }

  function total(p: PluginInfo): number {
    return COMPONENT_KEYS.reduce((n, k) => n + (p.components[k]?.length ?? 0), 0);
  }
</script>

<div class="plugins">
  <div class="bar">
    <input
      type="search"
      class="search"
      placeholder={$t("llm.central.plugins.search")}
      aria-label={$t("llm.central.plugins.search")}
      bind:value={query}
    />
    <span class="muted small tabular">{$t("llm.central.plugins.count", { count: String(plugins.length), on: String(onCount) })}</span>
    <button type="button" class="btn" onclick={load} disabled={loading}>{$t("llm.central.installed.refresh")}</button>
  </div>

  {#if error}<p class="err" role="alert">{error}</p>{/if}

  {#if !loading && !plugins.length && !error}
    <p class="muted empty">{$t("llm.central.plugins.empty")}</p>
  {/if}

  {#each groups as [tool, list] (tool)}
    <section class="group">
      <h2>
        <ToolDot id={tool} name={toolName(tool)} compat={{ status: "native" }} size={22} />
        {toolName(tool)}
        <span class="n tabular">{list.length}</span>
      </h2>
      <ul class="list">
        {#each list as p (key(p))}
          {@const k = key(p)}
          {@const s = scopeOf(p)}
          {@const on = stateIn(p, s)}
          {@const n = total(p)}
          <li class="item" class:off={p.enabled === false}>
            <div class="row">
              <div class="info">
                <p class="name">
                  {p.name}
                  {#if p.version}<span class="muted small mono">{p.version}</span>{/if}
                  {#if !p.installed}<span class="pill warn">{$t("llm.central.plugins.not_installed")}</span>{/if}
                </p>
                <p class="sub">
                  {#if p.marketplace}{$t("llm.central.plugins.from", { marketplace: p.marketplace })}{/if}
                  {#if p.source}<span class="mono">{p.marketplace ? ` · ${p.source}` : p.source}</span>{/if}
                  · {$t(`llm.central.plugins.scope.${p.scope}`)}
                  {#each Object.entries(p.enabled_in ?? {}) as [sc, v] (sc)}
                    <span class="pill" class:on={v}>{$t(`llm.central.plugins.scope.${sc}`)}: {v ? $t("llm.central.plugins.on") : $t("llm.central.plugins.off")}</span>
                  {/each}
                </p>
                {#if p.description}<p class="desc">{p.description}</p>{/if}
              </div>
              {#if n}
                <button type="button" class="chip" aria-expanded={open.has(k)} onclick={() => toggleOpen(k)}>
                  {$t("llm.central.plugins.brings", { count: String(n) })}
                </button>
              {/if}
              <div class="acts">
                {#if p.toggle_scopes.length > 1}
                  <select
                    value={s}
                    aria-label={$t("llm.central.plugins.scope_label")}
                    onchange={(e) => (scopes = { ...scopes, [k]: (e.currentTarget as HTMLSelectElement).value as PluginScope })}
                  >
                    {#each p.toggle_scopes as sc (sc)}
                      <option value={sc}>{$t(`llm.central.plugins.scope.${sc}`)}</option>
                    {/each}
                  </select>
                {/if}
                {#if p.toggle_scopes.length && p.enabled !== undefined}
                  <button
                    type="button"
                    class="switch"
                    class:on
                    role="switch"
                    aria-checked={on}
                    aria-label={$t("llm.central.plugins.toggle", { name: p.name })}
                    disabled={busy === k}
                    onclick={() => toggle(p)}
                  >
                    <span class="knob"></span>
                  </button>
                {:else}
                  <span class="muted small">{$t("llm.central.plugins.managed_in_tool")}</span>
                {/if}
              </div>
            </div>
            {#if open.has(k)}
              <dl class="comps">
                {#each COMPONENT_KEYS as ck (ck)}
                  {#if p.components[ck]?.length}
                    <dt>{$t(`llm.central.plugins.kind.${ck}`)} <span class="n tabular">{p.components[ck]!.length}</span></dt>
                    <dd>
                      {#each p.components[ck]!.slice(0, 60) as c (c)}<span class="tag mono">{c}</span>{/each}
                      {#if p.components[ck]!.length > 60}<span class="muted small">+{p.components[ck]!.length - 60}</span>{/if}
                    </dd>
                  {/if}
                {/each}
                {#if p.path}
                  <dt>{$t("llm.central.plugins.path")}</dt>
                  <dd class="mono small">{p.path}</dd>
                {/if}
              </dl>
            {/if}
            {#if p.notes?.length}
              <p class="notes muted small">{p.notes.join(" · ")}</p>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/each}
</div>

<style>
  .plugins {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .search {
    flex: 1;
    min-width: 200px;
    max-width: 360px;
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .empty {
    padding: var(--space-6) 0;
    text-align: center;
  }
  .group h2 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 var(--space-2);
    font-size: var(--text-md);
  }
  .n {
    font-size: var(--text-xs);
    color: var(--text-faint);
    font-weight: 400;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .item {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .item.off .name {
    color: var(--text-muted);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .info {
    flex: 1;
    min-width: 220px;
  }
  .name {
    margin: 0;
    font-weight: 600;
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .sub {
    margin: 2px 0 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
    display: flex;
    gap: 4px;
    align-items: center;
    flex-wrap: wrap;
  }
  .desc {
    margin: 4px 0 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .pill {
    padding: 0 6px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    font-size: var(--text-xs);
    font-weight: 400;
  }
  .pill.on {
    background: color-mix(in srgb, var(--success) 16%, transparent);
    color: var(--success);
  }
  .pill.warn {
    background: color-mix(in srgb, var(--warning) 18%, transparent);
    color: var(--warning);
  }
  .chip {
    height: 24px;
    padding: 0 10px;
    border: none;
    border-radius: var(--radius-full);
    background: var(--accent-soft);
    color: var(--accent-hi);
    font-size: var(--text-xs);
    font-weight: 600;
    cursor: pointer;
  }
  .acts {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  select {
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .switch {
    position: relative;
    width: 38px;
    height: 22px;
    flex: none;
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    cursor: pointer;
    transition: background 0.15s;
  }
  .switch.on {
    background: var(--success);
  }
  .switch .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #fff;
    box-shadow: var(--elev-1);
    transition: transform 0.15s;
  }
  .switch.on .knob {
    transform: translateX(16px);
  }
  .switch:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .switch:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .comps {
    margin: var(--space-2) 0 0;
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px var(--space-3);
    font-size: var(--text-sm);
  }
  .comps dt {
    color: var(--text-muted);
  }
  .comps dd {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    min-width: 0;
    word-break: break-all;
  }
  .tag {
    padding: 0 6px;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    font-size: var(--text-xs);
  }
  .notes {
    margin: 4px 0 0;
  }
  .btn {
    display: inline-flex;
    align-items: center;
    height: var(--control-h);
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .btn:hover:not(:disabled) {
    background: var(--fill-2);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: var(--text-xs);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .err {
    color: var(--error);
  }
</style>
