<script lang="ts">
  /**
   * Emotes da Twitch: baixa em lote os emotes e badges nativos do canal e os
   * de BTTV, FFZ e 7TV. Tudo público, sem login. Sai uma pasta por provedor,
   * um index.json e um contact-sheet HTML para olhar tudo de uma vez.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openPath, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Emote = { code: string; provider: string; kind: string; scope: string; animated: boolean; file: string; also_in?: string[] };
  type ProviderCount = { provider: string; emotes: number; badges: number };
  type Result = {
    channel: string;
    dir: string;
    index_path: string;
    sheet_path: string | null;
    total: number;
    downloaded: number;
    skipped: number;
    failed: number;
    duplicates: number;
    by_provider: ProviderCount[];
    items: Emote[];
  };

  let channel = $state("");
  let dest = $state("");
  let twitch = $state(true);
  let bttv = $state(true);
  let ffz = $state(true);
  let seventv = $state(true);
  let global = $state(true);
  let badges = $state(true);
  let sheet = $state(true);
  let overwrite = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let out = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const anyProvider = $derived(twitch || bttv || ffz || seventv);

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "tw-emotes") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (busy || !anyProvider) return;
    if (!dest) {
      const d = await pickDir();
      if (!d) return;
      dest = d;
    }
    busy = true;
    out = null;
    progress = null;
    try {
      out = await invoke<Result>("tool_tw_emotes", {
        opts: {
          channel,
          out_dir: dest,
          twitch,
          bttv,
          ffz,
          seventv,
          global,
          badges,
          contact_sheet: sheet,
          overwrite,
        },
      });
      showToast("success", `${out.downloaded} ${$t("tools.twemotes.emotes")}`);
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
          <input class="input" type="text" bind:value={channel} placeholder={$t("tools.twemotes.channel_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} />
          <div class="group-row-sub">{$t("tools.twemotes.channel_hint")}</div>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.twemotes.providers")}</div></div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-sm" class:btn-primary={twitch} class:btn-secondary={!twitch} type="button" onclick={() => (twitch = !twitch)}>Twitch</button>
          <button class="btn btn-sm" class:btn-primary={bttv} class:btn-secondary={!bttv} type="button" onclick={() => (bttv = !bttv)}>BTTV</button>
          <button class="btn btn-sm" class:btn-primary={ffz} class:btn-secondary={!ffz} type="button" onclick={() => (ffz = !ffz)}>FFZ</button>
          <button class="btn btn-sm" class:btn-primary={seventv} class:btn-secondary={!seventv} type="button" onclick={() => (seventv = !seventv)}>7TV</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.twemotes.globals")}</div><div class="group-row-sub">{$t("tools.twemotes.globals_hint")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={global} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.twemotes.badges")}</div><div class="group-row-sub">{$t("tools.twemotes.badges_hint")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={badges} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.twemotes.sheet")}</div><div class="group-row-sub">{$t("tools.twemotes.sheet_hint")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={sheet} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.twemotes.overwrite")}</div><div class="group-row-sub">{$t("tools.twemotes.overwrite_hint")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={overwrite} /></div>
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
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !anyProvider} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.twemotes.run")}</button></div>
      </div>
    </div>
  </section>

  {#if out}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{out.channel} · {out.total} {$t("tools.twemotes.emotes")}</div>
            <div class="group-row-sub">
              {out.downloaded} {$t("tools.twemotes.downloaded")} · {out.skipped} {$t("tools.twemotes.skipped")} · {out.failed} {$t("tools.common.failed")} · {out.duplicates} {$t("tools.twemotes.duplicates")}
            </div>
          </div>
          <div class="group-row-trailing btn-row">
            {#if out.sheet_path}<button class="btn btn-secondary btn-sm" type="button" onclick={() => openPath(out?.sheet_path ?? "")}>{$t("tools.twemotes.open_sheet")}</button>{/if}
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(out?.dir ?? "")}>{$t("tools.common.reveal")}</button>
          </div>
        </div>
        {#each out.by_provider as p (p.provider)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{p.provider}</div></div>
            <div class="group-row-trailing"><span class="tag">{p.emotes} {$t("tools.twemotes.emotes")}</span>{#if p.badges}<span class="tag">{p.badges} {$t("tools.twemotes.badges")}</span>{/if}</div>
          </div>
        {/each}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-sub codes">
              {#each out.items.slice(0, 200) as e (e.provider + e.code + e.kind)}
                <span class="tag" class:tag-success={e.animated}>{e.code}</span>
              {/each}
            </div>
          </div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .codes { display: flex; flex-wrap: wrap; gap: var(--space-1); }
</style>
