<script lang="ts">
  /**
   * Chat de VOD: puxa o replay inteiro pelo GraphQL público da Twitch e
   * exporta em JSON, CSV e legenda (SRT/ASS) já no tempo do vídeo. De brinde,
   * o mapa de picos de chat — o atalho para achar os melhores momentos.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtSecs, onToolProgress, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Peak = { start_seconds: number; clock: string; count: number };
  type Chatter = { author: string; count: number };
  type Message = { offset: number; display: string; color: string; text: string };
  type Result = {
    video_id: string;
    title: string;
    channel: string;
    duration_seconds: number;
    messages: number;
    pages: number;
    top_peaks: Peak[];
    top_chatters: Chatter[];
    files: string[];
    offset_fallback: boolean;
    sample: Message[];
  };

  const FORMATS = ["json", "csv", "srt", "ass"] as const;

  let input = $state("");
  let dest = $state("");
  let formats = $state<string[]>(["json", "csv", "srt"]);
  let windowSeconds = $state(5);
  let maxLines = $state(6);
  let maxMessages = $state(0);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let out = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "tw-chat") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  function toggle(f: string) {
    formats = formats.includes(f) ? formats.filter((x) => x !== f) : [...formats, f];
  }

  const wantsSubtitle = $derived(formats.includes("srt") || formats.includes("ass"));

  async function run() {
    if (busy || !input.trim() || !formats.length) return;
    if (!dest) {
      const d = await pickDir();
      if (!d) return;
      dest = d;
    }
    busy = true;
    out = null;
    progress = null;
    try {
      out = await invoke<Result>("tool_tw_chat_replay", {
        opts: {
          input,
          out_dir: dest,
          formats,
          window_seconds: windowSeconds,
          max_lines: maxLines,
          max_messages: maxMessages,
        },
      });
      showToast("success", `${out.messages} ${$t("tools.twchat.messages")}`);
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
          <input class="input" type="text" bind:value={input} placeholder={$t("tools.twchat.vod_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} />
          <div class="group-row-sub">{$t("tools.twchat.vod_hint")}</div>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.twchat.formats")}</div></div>
        <div class="group-row-trailing btn-row">
          {#each FORMATS as f (f)}
            <button class="btn btn-sm" class:btn-primary={formats.includes(f)} class:btn-secondary={!formats.includes(f)} type="button" onclick={() => toggle(f)}>{f.toUpperCase()}</button>
          {/each}
        </div>
      </div>
      {#if wantsSubtitle}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.twchat.window")} {windowSeconds}s</div><div class="group-row-sub">{$t("tools.twchat.window_hint")}</div></div>
          <div class="group-row-trailing"><input type="range" min="2" max="20" step="1" bind:value={windowSeconds} /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.twchat.lines")}</div><div class="group-row-sub">{$t("tools.twchat.lines_hint")}</div></div>
          <div class="group-row-trailing"><input class="input" type="number" min="1" max="20" step="1" bind:value={maxLines} style:width="5em" /></div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.twchat.limit")}</div><div class="group-row-sub">{$t("tools.twchat.limit_hint")}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="0" step="1000" bind:value={maxMessages} style:width="7em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{dest || $t("tools.common.ask_on_run")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !input.trim() || !formats.length} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.twchat.run")}</button></div>
      </div>
    </div>
  </section>

  {#if out}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{out.title || out.video_id}</div>
            <div class="group-row-sub">{out.channel} · {fmtSecs(out.duration_seconds)} · {out.messages} {$t("tools.twchat.messages")} · {out.pages} {$t("tools.twchat.pages")}</div>
          </div>
        </div>
        {#each out.files as f (f)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub mono">{f}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(f)}>{$t("tools.common.reveal")}</button></div>
          </div>
        {/each}
      </div>
    </section>

    <section>
      <span class="group-label">{$t("tools.twchat.peaks")}</span>
      <div class="group">
        {#each out.top_peaks as p (p.start_seconds)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title mono">{p.clock}</div></div>
            <div class="group-row-trailing"><span class="tag tag-success">{p.count} {$t("tools.twchat.per_minute")}</span></div>
          </div>
        {/each}
      </div>
    </section>

    <section>
      <span class="group-label">{$t("tools.twchat.chatters")}</span>
      <div class="group">
        {#each out.top_chatters as c (c.author)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{c.author}</div></div>
            <div class="group-row-trailing"><span class="tag">{c.count}</span></div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
