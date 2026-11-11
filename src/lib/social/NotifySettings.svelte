<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SocialIcon from "./SocialIcon.svelte";
  import { social, type Platform } from "./api";
  import { hasExtras, type NotifyState } from "./second";

  let { platform, scope, monitors = [] }: { platform: Platform; scope: "monitor" | "audience"; monitors?: { id: number; name: string }[] } = $props();

  let prefs = $state<NotifyState | null>(null);
  let open = $state(false);
  let busy = $state(false);

  async function load() {
    if (!hasExtras(platform)) return;
    try {
      prefs = await social.call<NotifyState>("social_notifications", { platform, op: "get" });
    } catch {
      prefs = null;
    }
  }

  async function change(args: Record<string, unknown>) {
    if (busy) return;
    busy = true;
    try {
      prefs = await social.call<NotifyState>("social_notifications", { platform, op: "set", ...args });
    } catch {
      showToast("error", $t("social_monitor.notify.failed") as string);
      void load();
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    void platform;
    untrack(() => {
      prefs = null;
      void load();
    });
  });
</script>

{#if prefs && scope === "audience"}
  <label class="ns-check">
    <input type="checkbox" class="checkbox" checked={prefs.unfollowers} disabled={busy} onchange={(e) => void change({ unfollowers: e.currentTarget.checked })} />
    {$t("social_audience.notify_unfollowers")}
  </label>
{:else if prefs}
  <div class="ns">
    <button class="btn btn-ghost btn-sm" aria-expanded={open} aria-controls="monitor-notify" onclick={() => (open = !open)}>
      <SocialIcon name="radar" /> {$t("social_monitor.notify.title")}
    </button>
    {#if open}
      <div class="ns-box" id="monitor-notify">
        <label class="ns-check">
          <input type="checkbox" class="checkbox" checked={prefs.surge} disabled={busy} onchange={(e) => void change({ surge: e.currentTarget.checked })} />
          {$t("social_monitor.notify.surge")}
        </label>
        <span class="ns-dim">{$t("social_monitor.notify.new_hits")}</span>
        {#each monitors as m (m.id)}
          <label class="ns-check ns-in">
            <input type="checkbox" class="checkbox" checked={prefs.monitors.includes(m.id)} disabled={busy} onchange={(e) => void change({ monitor: m.id, new_hits: e.currentTarget.checked })} />
            {m.name}
          </label>
        {:else}
          <span class="ns-dim ns-in">{$t("social_monitor.notify.none")}</span>
        {/each}
        <p class="ns-dim">{$t("social_monitor.notify.when")}</p>
      </div>
    {/if}
  </div>
{/if}

<style>
  .ns {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
  }

  .ns-box {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ns-check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .ns-in {
    padding-left: var(--space-3);
  }

  .ns-dim {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-xs);
  }
</style>
