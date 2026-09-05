<script lang="ts">
  /**
   * Duplicados no board (estudo 67): o programa que a thread do Reddit pediu
   * durante dois anos. Iguais por `image_signature`, parecidos por dHash;
   * com cookies dá para desfazer o save dos escolhidos.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openUrl, pct, type ToolProgress } from "$lib/tools/rt";
  import { fmtCount, loadCookies, preview, type DupesOut } from "$lib/tools/pinterest";
  import PinCookies from "./PinCookies.svelte";

  let url = $state("");
  let cookies = $state(loadCookies());
  let threshold = $state(6);
  let limit = $state(0);
  let busy = $state<string | null>(null);
  let out = $state<DupesOut | null>(null);
  let selected = $state<Set<string>>(new Set());
  let progress = $state<ToolProgress | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => { unlisten = await onToolProgress((p) => { if (p.id === "pinterest:dupes") progress = p; }); });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!url.trim() || busy) return;
    busy = "scan"; out = null; selected = new Set(); progress = null;
    try {
      out = await invoke<DupesOut>("tool_pin_dupes", { url, cookies: cookies || null, limit, threshold });
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }

  function toggle(id: string) { const s = new Set(selected); if (s.has(id)) s.delete(id); else s.add(id); selected = s; }
  function keepFirst() {
    const s = new Set<string>();
    for (const g of out?.groups ?? []) g.pins.slice(1).forEach((p) => s.add(p.id));
    selected = s;
  }

  async function unsave() {
    if (!selected.size || busy) return;
    if (!cookies) { showToast("error", $t("tools.pinterest.unsave_needs_cookies") as string); return; }
    if (!confirm(`${$t("tools.pinterest.unsave_confirm")} (${selected.size})`)) return;
    busy = "unsave";
    try {
      const r = await invoke<{ done: string[]; failed: [string, string][] }>("tool_pin_unsave", { ids: [...selected], cookies });
      showToast(r.failed.length ? "info" : "success", `${r.done.length} ${$t("tools.pinterest.unsaved")}${r.failed.length ? ` · ${r.failed.length} ${$t("tools.pinterest.failed")}: ${r.failed[0][1]}` : ""}`);
      if (out) {
        const gone = new Set(r.done);
        out = { ...out, groups: out.groups.map((g) => ({ ...g, pins: g.pins.filter((p) => !gone.has(p.id)) })).filter((g) => g.pins.length > 1) };
        selected = new Set([...selected].filter((id) => !gone.has(id)));
      }
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }

  let stage = $derived(progress ? ($t(progress.stage === "hash" ? "tools.pinterest.stage_hash" : "tools.pinterest.stage_list") as string) : "");
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="text" bind:value={url} placeholder={$t("tools.pinterest.board_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} /></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy !== null || !url.trim()} onclick={run}>{busy === "scan" ? $t("tools.common.working") : $t("tools.pinterest.scan")}</button></div>
      </div>
      <PinCookies bind:value={cookies} />
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.threshold")}</div><div class="group-row-sub">{$t("tools.pinterest.threshold_hint")}</div></div>
        <div class="group-row-trailing btn-row"><input type="range" min="0" max="16" step="1" bind:value={threshold} /> <span class="mono">{threshold}</span></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.limit")}</div><div class="group-row-sub">{$t("tools.pinterest.limit_hint")}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="0" step="100" bind:value={limit} style:width="7em" /></div>
      </div>
      {#if busy === "scan"}
        <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{stage} {progress?.done ?? 0}{#if progress?.total}/{progress.total}{/if}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div></div></div>
      {/if}
    </div>
  </section>

  {#if out}
    <section>
      <span class="group-label">{out.groups.length} {$t("tools.pinterest.groups")} · {out.scanned} {$t("tools.pinterest.scanned")}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-sub">{selected.size} {$t("tools.pinterest.selected")}{#if !out.has_session} · {$t("tools.pinterest.session_off")}{/if}</div></div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary btn-sm" type="button" disabled={!out.groups.length} onclick={keepFirst}>{$t("tools.pinterest.keep_first")}</button>
            <button class="btn btn-destructive btn-sm" type="button" disabled={!selected.size || busy !== null} onclick={unsave}>{$t("tools.pinterest.unsave")}</button>
          </div>
        </div>
        {#if !out.groups.length}<div class="group-row"><div class="group-row-sub">{$t("tools.pinterest.no_dupes")}</div></div>{/if}
        {#each out.groups.slice(0, 200) as g, i (i)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-sub"><span class="tag" class:tag-success={g.kind === "exact"} class:tag-warning={g.kind === "near"}>{g.kind === "exact" ? $t("tools.pinterest.exact") : `${$t("tools.pinterest.near")} · ${$t("tools.pinterest.distance")} ${g.distance}`}</span> · {g.pins.length} {$t("tools.pinterest.pins")}</div>
              <div class="row">
                {#each g.pins as p (p.id)}
                  <div class="dupe" class:on={selected.has(p.id)}>
                    <button class="dupe-img" type="button" onclick={() => toggle(p.id)} title={p.title}><img src={preview(p)} alt={p.title} loading="lazy" style:background={p.dominant_color ?? "var(--fill-1)"} /><span class="check">{selected.has(p.id) ? "✓" : ""}</span></button>
                    <div class="dupe-meta"><span>{p.section ?? p.board?.name ?? ""}</span><span>{fmtCount(p.saves)} {$t("tools.pinterest.saves")}</span><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(p.url)}>↗</button></div>
                  </div>
                {/each}
              </div>
            </div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); }
  .row { display: flex; flex-wrap: wrap; gap: var(--space-2); margin-top: var(--space-1); }
  .dupe { width: 132px; border-radius: var(--radius-md); overflow: hidden; background: var(--fill-1); box-shadow: inset 0 0 0 var(--hairline) var(--content-border); }
  .dupe.on { box-shadow: 0 0 0 2px var(--accent); }
  .dupe-img { position: relative; display: block; width: 100%; padding: 0; border: 0; background: none; cursor: pointer; }
  .dupe-img img { display: block; width: 100%; height: 132px; object-fit: cover; }
  .check { position: absolute; right: 5px; top: 5px; width: 20px; height: 20px; border-radius: 50%; background: rgba(255, 255, 255, 0.85); color: var(--accent); font-size: 12px; font-weight: 700; display: flex; align-items: center; justify-content: center; }
  .dupe.on .check { background: var(--accent); color: #fff; }
  .dupe-meta { display: flex; align-items: center; gap: 4px; padding: 3px 6px; font-size: var(--text-xs); color: var(--text-dim); }
  .dupe-meta span:first-child { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
