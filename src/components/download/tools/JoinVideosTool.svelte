<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Check = { lossless: boolean; reason: string | null; can_reencode: boolean; total_secs: number };

  let files = $state<string[]>([]);
  let verdict = $state<Check | null>(null);
  let checking = $state(false);
  let joining = $state(false);
  let error = $state<string | null>(null);
  let result = $state<string | null>(null);

  const base = (p: string) => p.split(/[\\/]/).pop() ?? p;
  const errText = (e: any) => {
    const msg = typeof e === "string" ? e : (e?.message ?? "");
    return msg ? translateBackendError(msg, $t) : $t("common.error");
  };

  function clock(s: number): string {
    const total = Math.round(s);
    const h = Math.floor(total / 3600);
    const m = Math.floor((total / 60) % 60);
    const sec = String(total % 60).padStart(2, "0");
    return h ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
  }

  async function recheck() {
    verdict = null;
    result = null;
    error = null;
    if (files.length < 2) return;
    checking = true;
    try {
      verdict = await invoke<Check>("join_check", { paths: files });
    } catch (e) {
      error = errText(e);
    } finally {
      checking = false;
    }
  }

  async function add() {
    const selected = await open({
      title: $t("downloads.tools.join_videos.add"),
      filters: [{ name: "Media", extensions: ["mp4", "mkv", "webm", "mov", "m4v", "ts", "avi", "mp3", "m4a", "opus", "flac", "wav"] }],
      multiple: true,
    });
    if (!selected) return;
    const picked = Array.isArray(selected) ? selected : [selected];
    const fresh = picked.filter((p) => !files.includes(p)).sort((a, b) => base(a).localeCompare(base(b), undefined, { numeric: true }));
    files = [...files, ...fresh];
    await recheck();
  }

  function move(i: number, by: number) {
    const j = i + by;
    if (j < 0 || j >= files.length) return;
    const next = [...files];
    [next[i], next[j]] = [next[j], next[i]];
    files = next;
    result = null;
  }

  async function remove(i: number) {
    files = files.filter((_, k) => k !== i);
    await recheck();
  }

  async function join(reencode: boolean) {
    joining = true;
    result = null;
    try {
      const r = await invoke<{ output_path: string }>("join_videos", { paths: files, reencode });
      result = r.output_path;
      showToast("success", $t("downloads.tools.join_videos.saved"));
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      joining = false;
    }
  }
</script>

<div class="jv" data-agent-readiness={checking || joining ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.join_videos.lead")}</p>
  <div class="row">
    <button class="button" onclick={add} disabled={joining}>{$t("downloads.tools.join_videos.add")}</button>
    {#if files.length}
      <span class="dim">{$t("downloads.tools.join_videos.count", { count: files.length })}</span>
    {/if}
  </div>

  {#if files.length}
    <ol class="list">
      {#each files as f, i (f)}
        <li class="item">
          <span class="num">{i + 1}</span>
          <span class="name" title={f}>{base(f)}</span>
          <button class="icon" onclick={() => move(i, -1)} disabled={i === 0} aria-label={$t("downloads.tools.join_videos.up")}>↑</button>
          <button class="icon" onclick={() => move(i, 1)} disabled={i === files.length - 1} aria-label={$t("downloads.tools.join_videos.down")}>↓</button>
          <button class="icon" onclick={() => remove(i)} aria-label={$t("downloads.tools.join_videos.remove")}>×</button>
        </li>
      {/each}
    </ol>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if checking}
    <p class="dim">{$t("downloads.tools.join_videos.checking")}</p>
  {:else if verdict}
    <p class={verdict.lossless ? "ok" : "dim"}>
      {verdict.lossless
        ? $t("downloads.tools.join_videos.lossless", { time: clock(verdict.total_secs) })
        : $t("downloads.tools.join_videos.needs_reencode", { reason: verdict.reason ?? "" })}
    </p>
    <div class="row">
      {#if verdict.lossless}
        <button class="button" onclick={() => join(false)} disabled={joining}>
          {joining ? $t("downloads.tools.join_videos.joining") : $t("downloads.tools.join_videos.join")}
        </button>
      {/if}
      {#if verdict.can_reencode}
        <button class={verdict.lossless ? "link" : "button"} onclick={() => join(true)} disabled={joining}>
          {joining && !verdict.lossless ? $t("downloads.tools.join_videos.joining") : $t("downloads.tools.join_videos.join_reencode")}
        </button>
      {/if}
      {#if result}
        <button class="link" onclick={() => void invoke("reveal_file", { path: result })}>{$t("downloads.tools.join_videos.reveal")}</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .jv {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .ok {
    margin: 0;
    color: var(--accent);
    font-size: var(--text-sm);
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-size: var(--text-sm);
  }
  .num {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    min-width: 1.5em;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .icon {
    border: none;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 2px 6px;
    border-radius: var(--radius-full);
  }
  .icon:hover:not(:disabled) {
    background: var(--fill-1);
    color: var(--text);
  }
  .icon:disabled {
    opacity: 0.3;
    cursor: default;
  }
  .link {
    border: none;
    background: none;
    color: var(--accent);
    font-size: var(--text-sm);
    cursor: pointer;
    padding: 2px var(--space-1);
    border-radius: var(--radius-full);
  }
  .link:hover {
    background: var(--accent-soft);
  }
</style>
