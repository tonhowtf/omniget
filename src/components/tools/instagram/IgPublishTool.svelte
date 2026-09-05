<script lang="ts">
  /**
   * Publicar (foto, vídeo, reel, story, carrossel) pela sessão web ou pela
   * API oficial (Graph, URLs públicas), agora ou agendado. `mode` =
   * "publish" | "schedule".
   */
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openUrl, pickFiles, type ToolProgress } from "$lib/tools/rt";
  import { fmtDate, igState, jobId, recall, remember, slugArg, type GraphAuth, type PublishRequest, type PublishResult, type ScheduledPost } from "$lib/tools/ig.svelte";
  import IgAccountRow from "./IgAccountRow.svelte";

  let { mode = "publish" }: { mode?: "publish" | "schedule" } = $props();
  let req = $state<PublishRequest>({ kind: "photo", files: [], caption: "", share_to_feed: true, disable_comments: false, hide_like_counts: false, alt_text: "" });
  let via = $state<"web" | "graph">((recall("publish_via", "web") as "web" | "graph") || "web");
  let graph = $state<GraphAuth>({ access_token: recall("graph_token"), ig_user_id: recall("graph_user") });
  let urlsText = $state("");
  let when = $state("");
  let busy = $state(false);
  let job = $state("");
  let progress = $state<ToolProgress | null>(null);
  let result = $state<PublishResult | null>(null);
  let schedule = $state<ScheduledPost[]>([]);
  let unlisten: (() => void)[] = [];

  onMount(async () => {
    unlisten.push(await onToolProgress((p) => {
      if (job && p.id === `ig:${job}`) progress = p;
    }));
    unlisten.push(await listen("ig-schedule-changed", () => loadSchedule()));
    await loadSchedule();
    const d = new Date(Date.now() + 3600 * 1000);
    d.setSeconds(0, 0);
    when = new Date(d.getTime() - d.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
  });
  onDestroy(() => unlisten.forEach((u) => u()));

  async function loadSchedule() {
    schedule = (await invoke<{ posts: ScheduledPost[] }>("tool_ig_schedule_list")).posts;
  }

  const FILTERS = { photo: [{ name: "Image", extensions: ["jpg", "jpeg", "png", "webp", "heic"] }], video: [{ name: "Video", extensions: ["mp4", "mov", "m4v"] }], any: [{ name: "Media", extensions: ["jpg", "jpeg", "png", "webp", "heic", "mp4", "mov", "m4v"] }] };

  async function choose() {
    const f = await pickFiles(req.kind === "photo" ? FILTERS.photo : req.kind === "carousel" || req.kind === "story" ? FILTERS.any : FILTERS.video);
    if (f.length) req.files = req.kind === "carousel" ? [...req.files, ...f].slice(0, 20) : [f[0]];
  }

  function request(): PublishRequest {
    const r = $state.snapshot(req);
    if (via === "graph") r.files = urlsText.split(/\s+/).map((s) => s.trim()).filter(Boolean);
    return r;
  }

  function persist() {
    remember("publish_via", via);
    remember("graph_token", graph.access_token);
    remember("graph_user", graph.ig_user_id);
  }

  async function publish() {
    if (busy) return;
    persist();
    const r = request();
    if (!r.files.length) return showToast("error", $t("tools.ig.publish.no_files") as string);
    busy = true;
    result = null;
    progress = null;
    job = jobId("publish");
    try {
      result = via === "graph" ? await invoke<PublishResult>("tool_ig_publish_graph", { auth: $state.snapshot(graph), request: r, job }) : await invoke<PublishResult>("tool_ig_publish", { slug: slugArg(), request: r, job });
      showToast("success", $t("tools.ig.publish.published") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function addSchedule() {
    persist();
    const r = request();
    if (!r.files.length) return showToast("error", $t("tools.ig.publish.no_files") as string);
    const run_at = Math.floor(new Date(when).getTime() / 1000);
    if (!run_at || run_at < Date.now() / 1000) return showToast("error", $t("tools.ig.publish.bad_time") as string);
    try {
      const post: ScheduledPost = { id: "", run_at, request: r, mode: via, account_slug: slugArg(), graph: via === "graph" ? $state.snapshot(graph) : null, status: "pending", result: null, error: null, created_at: 0 };
      schedule = (await invoke<{ posts: ScheduledPost[] }>("tool_ig_schedule_add", { post })).posts;
      showToast("success", $t("tools.ig.publish.scheduled") as string);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function removeSchedule(id: string) {
    schedule = (await invoke<{ posts: ScheduledPost[] }>("tool_ig_schedule_remove", { id })).posts;
  }

  let stageLabel = $derived(progress ? ($t(`tools.ig.publish.stage_${progress.stage}`) as string) : "");
</script>

<div class="tool">
  {#if via === "web"}<IgAccountRow />{/if}
  <section>
    <span class="group-label">{$t("tools.ig.publish.how")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ig.publish.via")}</div><div class="group-row-sub">{via === "web" ? $t("tools.ig.publish.via_web_hint") : $t("tools.ig.publish.via_graph_hint")}</div></div>
        <div class="group-row-trailing">
          <div class="segmented"><button class="segmented-btn" class:active={via === "web"} type="button" onclick={() => (via = "web")}>{$t("tools.ig.publish.via_web")}</button><button class="segmented-btn" class:active={via === "graph"} type="button" onclick={() => (via = "graph")}>{$t("tools.ig.publish.via_graph")}</button></div>
        </div>
      </div>
      {#if via === "graph"}
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.ig.publish.token")}</div><div class="group-row-sub">{$t("tools.ig.publish.token_hint")}</div></div><div class="group-row-trailing"><input class="input" type="password" bind:value={graph.access_token} placeholder="EAAB…" /></div></div>
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.ig.publish.ig_user")}</div></div><div class="group-row-trailing btn-row"><input class="input" type="text" bind:value={graph.ig_user_id} placeholder="1784…" /><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl("https://developers.facebook.com/docs/instagram-platform/instagram-graph-api/content-publishing")}>{$t("tools.ig.publish.docs")}</button></div></div>
      {/if}
    </div>
  </section>

  <section>
    <span class="group-label">{$t("tools.ig.publish.what")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ig.publish.kind")}</div></div>
        <div class="group-row-trailing">
          <select class="select" bind:value={req.kind} onchange={() => (req.files = [])}>
            <option value="photo">{$t("tools.ig.publish.kind_photo")}</option>
            <option value="carousel">{$t("tools.ig.publish.kind_carousel")}</option>
            <option value="reel">{$t("tools.ig.publish.kind_reel")}</option>
            <option value="video">{$t("tools.ig.publish.kind_video")}</option>
            <option value="story">{$t("tools.ig.publish.kind_story")}</option>
          </select>
        </div>
      </div>
      {#if via === "graph"}
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.ig.publish.urls")}</div><textarea class="input" rows="3" bind:value={urlsText} placeholder="https://…/foto.jpg"></textarea></div></div>
      {:else}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.common.file")}</div><div class="group-row-sub mono">{req.files.length ? req.files.map(baseName).join(" · ") : $t("tools.ig.publish.no_files")}</div></div>
          <div class="group-row-trailing btn-row">{#if req.files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (req.files = [])}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={choose}>{$t("tools.common.choose")}</button></div>
        </div>
      {/if}
      {#if req.kind !== "story"}
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.ig.publish.caption")}</div><textarea class="input" rows="4" bind:value={req.caption} placeholder={$t("tools.ig.publish.caption_placeholder")}></textarea><div class="group-row-sub">{req.caption.length}/2200 · {(req.caption.match(/#\w+/g) ?? []).length} #</div></div></div>
        <div class="group-row"><div class="group-row-content"><div class="opts">
          {#if req.kind === "reel"}<label class="chk"><input type="checkbox" bind:checked={req.share_to_feed} /> {$t("tools.ig.publish.share_feed")}</label>{/if}
          <label class="chk"><input type="checkbox" bind:checked={req.disable_comments} /> {$t("tools.ig.publish.no_comments")}</label>
          <label class="chk"><input type="checkbox" bind:checked={req.hide_like_counts} /> {$t("tools.ig.publish.hide_likes")}</label>
        </div></div></div>
        {#if req.kind === "photo" || req.kind === "carousel"}<div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.ig.publish.alt")}</div></div><div class="group-row-trailing"><input class="input" type="text" bind:value={req.alt_text} /></div></div>{/if}
      {/if}
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}<div class="group-row-sub">{stageLabel} {progress?.done ?? 0}/{progress?.total ?? ""}</div>
          {:else if result}<div class="group-row-title">{$t("tools.ig.publish.published")}</div><div class="group-row-sub mono">{result.url || result.media_id}</div>{/if}
        </div>
        <div class="group-row-trailing btn-row">
          {#if mode === "schedule"}
            <input class="input" type="datetime-local" bind:value={when} />
            <button class="btn btn-primary" type="button" disabled={busy} onclick={addSchedule}>{$t("tools.ig.publish.schedule")}</button>
          {:else}
            {#if result?.url}<button class="btn btn-secondary btn-sm" type="button" onclick={() => openUrl(result!.url)}>{$t("tools.common.open")}</button>{/if}
            <button class="btn btn-primary" type="button" disabled={busy || (via === "web" && !igState.me)} onclick={publish}>{busy ? $t("tools.common.working") : $t("tools.ig.publish.publish")}</button>
          {/if}
        </div>
      </div>
    </div>
    <p class="group-footer">{$t("tools.ig.publish.beta")}</p>
  </section>

  {#if mode === "schedule" || schedule.length}
    <section>
      <span class="group-label">{$t("tools.ig.publish.queue")} · {schedule.length}</span>
      <div class="group">
        {#if !schedule.length}<div class="group-row"><div class="group-row-sub">{$t("tools.ig.publish.queue_empty")}</div></div>{/if}
        {#each schedule as p (p.id)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{fmtDate(p.run_at)} · {$t(`tools.ig.publish.kind_${p.request.kind}`)} <span class="tag" class:tag-success={p.status === "done"} class:tag-warning={p.status === "failed"} class:tag-accent={p.status === "running"}>{$t(`tools.ig.publish.status_${p.status}`)}</span></div>
              <div class="group-row-sub">{p.request.files.map(baseName).join(" · ")}{#if p.request.caption} · {p.request.caption.slice(0, 80)}{/if}{#if p.error} · {p.error}{/if}</div>
            </div>
            <div class="group-row-trailing btn-row">
              {#if p.result?.url}<button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(p.result!.url)}>{$t("tools.common.open")}</button>{/if}
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => removeSchedule(p.id)}>×</button>
            </div>
          </div>
        {/each}
      </div>
      <p class="group-footer">{$t("tools.ig.publish.queue_hint")}</p>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  textarea.input { width: 100%; resize: vertical; margin-top: var(--space-1); }
  .opts { display: flex; flex-wrap: wrap; gap: var(--space-2) var(--space-4); }
  .chk { display: inline-flex; align-items: center; gap: var(--space-1); font-size: var(--text-sm); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
