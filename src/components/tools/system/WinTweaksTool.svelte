<script lang="ts">
  /**
   * ⚠️ Só Windows. Ajustes de privacidade/interface (Sophia Script) e
   * endurecimento (hardentools) como regras de registro com reversão.
   * `group` escolhe que subconjunto a tool mostra.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/tools/rt";

  let { group = "privacy" }: { group?: string } = $props();

  type Rule = { id: string; group: string; source: string; key: string; name: string; requires_admin: boolean; restart: boolean; applied: boolean | null };
  type Status = { supported: boolean; is_admin: boolean; rules: Rule[] };
  let status = $state<Status | null>(null);
  let busy = $state<string | null>(null);

  const GROUPS: Record<string, string[]> = { privacy: ["privacy", "ui", "context"], harden: ["harden"] };
  let rules = $derived((status?.rules ?? []).filter((r) => (GROUPS[group] ?? [group]).includes(r.group)));

  async function refresh() { status = await invoke<Status>("tool_win_tweaks_status"); }
  onMount(refresh);

  async function toggle(r: Rule) {
    if (busy) return;
    busy = r.id;
    try {
      await invoke("tool_win_tweak_apply", { id: r.id, enable: !r.applied });
      if (r.restart) showToast("info", $t("tools.wintweaks.restart") as string);
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; await refresh(); }
  }

  let byGroup = $derived(Object.entries(rules.reduce<Record<string, Rule[]>>((acc, r) => { (acc[r.group] ??= []).push(r); return acc; }, {})));
</script>

<div class="tool">
  {#if status && !status.supported}
    <div class="notice notice-warning"><div class="notice-text">{$t("tools.wintweaks.only_windows")}</div></div>
  {:else if status && !status.is_admin}
    <div class="notice notice-info"><div class="notice-text">{$t("tools.wintweaks.admin_hint")}</div></div>
  {/if}
  {#each byGroup as [g, list] (g)}
    <section>
      <span class="group-label">{$t(`tools.wintweaks.group_${g}`)}</span>
      <div class="group">
        {#each list as r (r.id)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{$t(`tools.wintweaks.rules.${r.id}`)} {#if r.requires_admin}<span class="tag tag-warning">admin</span>{/if}{#if r.restart}<span class="tag">↻</span>{/if}</div>
              <div class="group-row-sub mono">{r.key}\{r.name || "(default)"} · {r.source}</div>
            </div>
            <div class="group-row-trailing">
              <button class="toggle" class:on={r.applied === true} type="button" role="switch" aria-checked={r.applied === true} aria-label={$t(`tools.wintweaks.rules.${r.id}`)} disabled={!status?.supported || busy !== null} onclick={() => toggle(r)}><span class="toggle-knob"></span></button>
            </div>
          </div>
        {/each}
      </div>
    </section>
  {/each}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
