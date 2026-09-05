<script lang="ts">
  /** Return YouTube Dislike (estudo 44): likes, dislikes e proporção. */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/tools/rt";

  type Votes = { id: string; date_created: string; likes: number; dislikes: number; rating: number; view_count: number; deleted: boolean };

  let url = $state("");
  let busy = $state(false);
  let votes = $state<Votes | null>(null);

  async function lookup() {
    if (!url.trim() || busy) return;
    busy = true;
    votes = null;
    try {
      votes = await invoke<Votes>("tool_ryd", { url });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  let ratio = $derived(votes && votes.likes + votes.dislikes > 0 ? (votes.likes / (votes.likes + votes.dislikes)) * 100 : 0);
  const n = (v: number) => v.toLocaleString();
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder={$t("tools.common.yt_url")} onkeydown={(e) => e.key === "Enter" && lookup()} /></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !url.trim()} onclick={lookup}>{busy ? $t("tools.common.working") : $t("tools.ryd.lookup")}</button></div>
      </div>
    </div>
  </section>
  {#if votes}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="stats"><div class="stat"><div class="stat-v">👍 {n(votes.likes)}</div></div><div class="stat"><div class="stat-v">👎 {n(votes.dislikes)}</div></div><div class="stat"><div class="stat-v">★ {votes.rating.toFixed(2)}</div></div><div class="stat"><div class="stat-v">{n(votes.view_count)} views</div></div></div>
            <div class="ratio"><div class="ratio-like" style:width="{ratio}%"></div></div>
            <div class="group-row-sub">{ratio.toFixed(1)}% {$t("tools.ryd.positive")} · {$t("tools.ryd.note")}{#if votes.deleted} · {$t("tools.ryd.deleted")}{/if}</div>
          </div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .stats { display: flex; flex-wrap: wrap; gap: var(--space-4); margin-bottom: var(--space-2); }
  .stat-v { font-family: var(--font-display); font-size: var(--text-lg); font-weight: 700; }
  .ratio { width: 100%; height: 6px; border-radius: 3px; background: #e0303a; overflow: hidden; margin-bottom: var(--space-1); }
  .ratio-like { height: 100%; background: #2aa845; }
</style>
