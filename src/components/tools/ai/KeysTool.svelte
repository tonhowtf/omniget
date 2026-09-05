<script lang="ts">
  /** Chaves de API (estudo 24, All API Hub): cofre local, teste, saldo, exportação. */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/tools/rt";

  type Kind = { id: string; name: string; base_url: string; balance: boolean; env: string };
  type View = { id: string; name: string; kind: string; base_url: string; key_hint: string; has_key: boolean; has_access_token: boolean; user_id: string; model: string; notes: string; created: number; last_ok: boolean | null; last_checked: number | null; balance: string | null; models: number; error: string | null };

  let kinds = $state<Kind[]>([]);
  let keys = $state<View[]>([]);
  let busy = $state<string | null>(null);
  let editing = $state(false);
  let form = $state({ id: "", name: "", kind: "openai", base_url: "", key: "", access_token: "", user_id: "", model: "", notes: "" });
  let models = $state<Record<string, string[]>>({});
  let exportFormat = $state("env");
  let exportIds = $state<Set<string>>(new Set());
  let exported = $state("");

  onMount(async () => {
    try { kinds = await invoke<Kind[]>("tool_keys_kinds"); await refresh(); } catch (e) { showToast("error", errText(e)); }
  });
  async function refresh() { keys = await invoke<View[]>("tool_keys_list"); }
  function kindOf(id: string) { return kinds.find((k) => k.id === id); }

  function startNew() { form = { id: "", name: "", kind: "openai", base_url: kindOf("openai")?.base_url ?? "", key: "", access_token: "", user_id: "", model: "", notes: "" }; editing = true; }
  function startEdit(k: View) { form = { id: k.id, name: k.name, kind: k.kind, base_url: k.base_url, key: "", access_token: "", user_id: k.user_id, model: k.model, notes: k.notes }; editing = true; }
  function onKind() { const k = kindOf(form.kind); if (k && (!form.base_url || kinds.some((x) => x.base_url === form.base_url))) form.base_url = k.base_url; }
  async function save() {
    if (busy) return;
    busy = "save";
    try { await invoke("tool_keys_save", { entry: { ...form, created: 0, models: 0 } }); editing = false; await refresh(); showToast("success", $t("tools.common.done") as string); }
    catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }
  async function remove(k: View) {
    try { await invoke("tool_keys_delete", { id: k.id }); await refresh(); } catch (e) { showToast("error", errText(e)); }
  }
  async function test(k: View) {
    busy = `test:${k.id}`;
    try { await invoke("tool_keys_test", { id: k.id }); await refresh(); showToast("success", "OK"); }
    catch (e) { await refresh(); showToast("error", errText(e)); } finally { busy = null; }
  }
  async function balance(k: View) {
    busy = `bal:${k.id}`;
    try { await invoke("tool_keys_balance", { id: k.id }); await refresh(); }
    catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }
  async function listModels(k: View) {
    busy = `models:${k.id}`;
    try { models = { ...models, [k.id]: await invoke<string[]>("tool_keys_models", { entry: { id: k.id, name: k.name, kind: k.kind, base_url: k.base_url, key: "", created: 0, models: 0 } }) }; }
    catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }
  async function useInApp(k: View) {
    try { await invoke("tool_keys_use", { id: k.id }); showToast("success", $t("tools.keys.used") as string); } catch (e) { showToast("error", errText(e)); }
  }
  async function doExport() {
    try { exported = await invoke<string>("tool_keys_export", { format: exportFormat, ids: [...exportIds] }); }
    catch (e) { showToast("error", errText(e)); }
  }
  async function copy(text: string) { await navigator.clipboard.writeText(text); showToast("success", $t("tools.common.copied") as string); }
  function toggleExport(id: string) { const s = new Set(exportIds); if (s.has(id)) s.delete(id); else s.add(id); exportIds = s; exported = ""; }
  const when = (ts: number | null) => (ts ? new Date(ts * 1000).toLocaleString() : "");
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{keys.length} {$t("tools.keys.accounts")}</div><div class="group-row-sub">{$t("tools.keys.hint")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-primary btn-sm" type="button" onclick={startNew}>{$t("tools.common.add")}</button></div>
      </div>
      {#if editing}
        <div class="group-row form">
          <div class="grid">
            <label><span>{$t("tools.keys.provider")}</span><select class="input" bind:value={form.kind} onchange={onKind}>{#each kinds as k (k.id)}<option value={k.id}>{k.name}</option>{/each}</select></label>
            <label><span>{$t("tools.keys.name")}</span><input class="input" type="text" bind:value={form.name} placeholder={kindOf(form.kind)?.name} /></label>
            <label class="wide"><span>Base URL</span><input class="input mono" type="text" bind:value={form.base_url} /></label>
            <label class="wide"><span>API key {#if form.id}<em class="dim">· {$t("tools.keys.keep_hint")}</em>{/if}</span><input class="input mono" type="password" bind:value={form.key} autocomplete="off" /></label>
            {#if form.kind === "newapi"}
              <label><span>{$t("tools.keys.access_token")}</span><input class="input mono" type="password" bind:value={form.access_token} autocomplete="off" /></label>
              <label><span>{$t("tools.keys.user_id")}</span><input class="input mono" type="text" bind:value={form.user_id} /></label>
            {/if}
            <label><span>{$t("tools.keys.model")}</span><input class="input mono" type="text" bind:value={form.model} placeholder="gpt-4.1-mini" /></label>
            <label><span>{$t("tools.keys.notes")}</span><input class="input" type="text" bind:value={form.notes} /></label>
          </div>
          <div class="btn-row end">
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => (editing = false)}>{$t("tools.keys.cancel")}</button>
            <button class="btn btn-primary btn-sm" type="button" disabled={busy !== null || (!form.id && !form.key && form.kind !== "ollama")} onclick={save}>{$t("tools.common.save")}</button>
          </div>
        </div>
      {/if}
    </div>
  </section>

  <section>
    <div class="group">
      {#each keys as k (k.id)}
        <div class="group-row">
          <input type="checkbox" class="pick" checked={exportIds.has(k.id)} onchange={() => toggleExport(k.id)} title={$t("tools.keys.export")} />
          <div class="group-row-content">
            <div class="group-row-title">
              <span class="dot" class:ok={k.last_ok === true} class:bad={k.last_ok === false}></span>
              {k.name} <span class="tag">{kindOf(k.kind)?.name ?? k.kind}</span>{#if k.model}<span class="tag">{k.model}</span>{/if}
            </div>
            <div class="group-row-sub mono">{k.base_url} · {k.key_hint || "—"}</div>
            <div class="group-row-sub">
              {#if k.balance}<strong>{k.balance}</strong> · {/if}{#if k.models}{k.models} {$t("tools.keys.models")} · {/if}{#if k.last_checked}{when(k.last_checked)}{/if}{#if k.error} · <span class="err">{k.error}</span>{/if}
            </div>
            {#if models[k.id]}<div class="group-row-sub mono models">{models[k.id].join(", ")}</div>{/if}
          </div>
          <div class="group-row-trailing btn-row wrap">
            <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={() => test(k)}>{busy === `test:${k.id}` ? "…" : $t("tools.keys.test")}</button>
            {#if kindOf(k.kind)?.balance}<button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={() => balance(k)}>{busy === `bal:${k.id}` ? "…" : $t("tools.keys.balance")}</button>{/if}
            <button class="btn btn-ghost btn-sm" type="button" disabled={busy !== null} onclick={() => listModels(k)}>{$t("tools.keys.models")}</button>
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => useInApp(k)}>{$t("tools.keys.use")}</button>
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => startEdit(k)}>{$t("tools.keys.edit")}</button>
            <button class="btn btn-ghost btn-sm danger" type="button" onclick={() => remove(k)}>{$t("tools.common.remove")}</button>
          </div>
        </div>
      {/each}
      {#if !keys.length}<div class="group-row"><div class="group-row-sub">{$t("tools.keys.none")}</div></div>{/if}
    </div>
  </section>

  {#if keys.length}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.keys.export")}</div><div class="group-row-sub">{exportIds.size ? `${exportIds.size} ${$t("tools.sysclean.selected")}` : $t("tools.keys.export_hint")}</div></div>
          <div class="group-row-trailing btn-row">
            <select class="input" bind:value={exportFormat} onchange={() => (exported = "")}>
              <option value="env">.env</option>
              <option value="claude-code">Claude Code (settings.json)</option>
              <option value="codex">Codex (config.toml)</option>
              <option value="cherry">Cherry Studio</option>
              <option value="opencode">opencode</option>
              <option value="json">JSON</option>
            </select>
            <button class="btn btn-secondary btn-sm" type="button" onclick={doExport}>{$t("tools.keys.generate")}</button>
          </div>
        </div>
        {#if exported}
          <div class="group-row"><div class="group-row-content"><pre class="code">{exported}</pre></div><div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(exported)}>{$t("tools.common.copy")}</button></div></div>
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; font-style: normal; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .form { flex-direction: column; align-items: stretch; gap: var(--space-3); }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-2) var(--space-3); }
  .grid label { display: flex; flex-direction: column; gap: 4px; font-size: var(--text-sm); color: var(--text-muted); }
  .grid label.wide { grid-column: 1 / -1; }
  .end { justify-content: flex-end; }
  .pick { accent-color: var(--accent); margin-right: var(--space-2); }
  .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: var(--text-dim); margin-right: 6px; }
  .dot.ok { background: var(--success); }
  .dot.bad { background: var(--danger); }
  .err { color: var(--danger); }
  .models { max-height: 4.5em; overflow: auto; }
  .wrap { flex-wrap: wrap; justify-content: flex-end; }
  .danger { color: var(--danger); }
  .code { margin: 0; white-space: pre-wrap; font-family: var(--font-mono); font-size: var(--text-xs); max-height: 320px; overflow: auto; }
</style>
