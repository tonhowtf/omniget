<script lang="ts">
  /** Capa real do vídeo (estudo 42): os frames hq1/hq2/hq3 e todas as variantes de thumbnail do CDN. */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, saveAs } from "$lib/tools/rt";

  let url = $state("");
  let id = $state<string | null>(null);

  const FRAMES = [
    { k: "hq1", l: "25%" }, { k: "hq2", l: "50%" }, { k: "hq3", l: "75%" },
    { k: "maxresdefault", l: "capa (max)" }, { k: "hqdefault", l: "capa (hq)" }, { k: "sddefault", l: "capa (sd)" }, { k: "maxres1", l: "25% max" }, { k: "maxres2", l: "50% max" }, { k: "maxres3", l: "75% max" },
  ];

  async function resolve() {
    id = await invoke<string | null>("tool_yt_video_id", { url });
    if (!id) showToast("error", $t("tools.frames.bad_url") as string);
  }

  function src(k: string): string {
    return `https://i.ytimg.com/vi/${id}/${k}.jpg`;
  }

  async function save(k: string) {
    const dest = await saveAs(`${id}-${k}.jpg`, [{ name: "JPEG", extensions: ["jpg"] }]);
    if (!dest) return;
    try {
      await invoke("tool_save_url", { url: src(k), dest });
      showToast("success", $t("tools.common.done") as string);
    } catch (e) {
      showToast("error", errText(e));
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder={$t("tools.common.yt_url")} onkeydown={(e) => e.key === "Enter" && resolve()} /></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={!url.trim()} onclick={resolve}>{$t("tools.frames.show")}</button></div>
      </div>
      <div class="group-row"><div class="group-row-sub">{$t("tools.frames.intro")}</div></div>
    </div>
  </section>
  {#if id}
    <section>
      <div class="grid">
        {#each FRAMES as f (f.k)}
          <figure class="card">
            <img src={src(f.k)} alt={f.l} loading="lazy" onerror={(e) => ((e.currentTarget as HTMLImageElement).style.opacity = "0.25")} />
            <figcaption><span>{f.l} <span class="mono">{f.k}</span></span><button class="btn btn-ghost btn-sm" type="button" onclick={() => save(f.k)}>{$t("tools.common.save")}</button></figcaption>
          </figure>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); color: var(--text-dim); }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: var(--space-3); }
  .card { margin: 0; border-radius: var(--radius-lg); overflow: hidden; background: var(--surface); box-shadow: inset 0 0 0 var(--hairline) var(--content-border); }
  .card img { display: block; width: 100%; aspect-ratio: 16/9; object-fit: cover; background: #000; }
  .card figcaption { display: flex; align-items: center; justify-content: space-between; padding: 6px 8px; font-size: var(--text-sm); }
</style>
