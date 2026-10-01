<script lang="ts">
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import type { CanvasNode } from "$lib/canvas/model/types";
  import { useEngine } from "$lib/canvas/engine.svelte";

  let { node }: { node: CanvasNode } = $props();
  const engine = useEngine();

  const path = $derived(String(node.data.path ?? ""));
  const name = $derived(path.split(/[\\/]/).filter(Boolean).pop() ?? node.title ?? "");
  const ext = $derived(name.includes(".") ? name.split(".").pop()?.toUpperCase() : "");
  const IMAGE = /\.(png|jpe?g|gif|webp|svg|bmp|ico|avif)$/i;
  const isImage = $derived(node.data.isImage === true || IMAGE.test(path));

  let preview = $state<string | null>(null);

  const MEDIA = /\.(mp4|mov|mkv|webm|avi|m4v|mpg|mpeg|wmv|flv|ts|mp3|m4a|aac|wav|flac|ogg|opus|wma|aiff)$/i;
  const VIDEO = /\.(mp4|mov|mkv|webm|avi|m4v|mpg|mpeg|wmv|flv|ts)$/i;
  const isMedia = $derived(MEDIA.test(path));
  const isVideo = $derived(VIDEO.test(path));
  let busy = $state(false);
  let status = $state("");
  let jobId = $state<string | null>(null);

  type Job = { jobId: string; stage: string; progress: number; error?: string | null };

  function describe(j: Job): string {
    if (j.stage === "failed") return $t("canvas.media.failed", { error: j.error ?? "" }) as string;
    if (j.stage === "transcribing") return $t("canvas.media.stage.transcribing", { pct: Math.round((j.progress ?? 0) * 100) }) as string;
    return $t(`canvas.media.stage.${j.stage}`) as string;
  }

  $effect(() => {
    const id = jobId;
    if (!id) return;
    let off: (() => void) | null = null;
    let alive = true;
    listen<Job>("canvas://transcribe", (e) => {
      if (e.payload.jobId !== id) return;
      status = describe(e.payload);
      if (e.payload.stage === "done" || e.payload.stage === "failed") busy = false;
    }).then((u) => {
      if (alive) off = u;
      else u();
    });
    return () => {
      alive = false;
      off?.();
    };
  });

  async function extractAudio() {
    busy = true;
    status = $t("canvas.media.stage.audio") as string;
    try {
      await invoke("canvas_audio_extract", { workspaceId: engine.store.workspaceId, path });
      status = $t("canvas.media.audioReady") as string;
    } catch (e) {
      status = $t("canvas.media.failed", { error: String(e) }) as string;
    } finally {
      busy = false;
    }
  }

  async function transcribe() {
    busy = true;
    status = $t("canvas.media.stage.placing") as string;
    try {
      const r = await invoke<{ jobId: string }>("canvas_transcribe_start", { workspaceId: engine.store.workspaceId, path });
      jobId = r.jobId;
    } catch (e) {
      busy = false;
      status = $t("canvas.media.failed", { error: String(e) }) as string;
    }
  }

  $effect(() => {
    const p = path;
    const img = isImage;
    const ws = engine.store.workspaceId;
    const floor = engine.store.floorId;
    const id = node.id;
    preview = null;
    if (!img || !p || engine.syncMode !== "rust") return;
    let alive = true;
    invoke<string | null>("canvas_file_node_preview", { workspaceId: ws, floorId: floor, nodeId: id })
      .then((thumb) => {
        if (alive) preview = !thumb ? null : thumb.startsWith("data:") ? thumb : convertFileSrc(thumb);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  });
</script>

<div class="file" class:has-preview={!!preview}>
  {#if preview}
    <img class="preview" src={preview} alt={name} draggable="false" />
    <div class="caption"><bdi>{name}</bdi></div>
  {:else}
    <div class="badge">{ext || "FILE"}</div>
    <div class="meta">
      <div class="name">{name || $t("canvas.engine.fileMissing", { default: "No file" })}</div>
      {#if path}<div class="path" title={path}><bdi dir="ltr">{path}</bdi></div>{/if}
      {#if isMedia}
        <div class="actions" role="group" onpointerdown={(e) => e.stopPropagation()}>
          {#if isVideo}<button type="button" class="act" disabled={busy} onclick={extractAudio}>{$t("canvas.media.extractAudio")}</button>{/if}
          <button type="button" class="act" disabled={busy} onclick={transcribe}>{$t("canvas.media.transcribe")}</button>
        </div>
        {#if status}<div class="status" role="status">{status}</div>{/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .file {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 100%;
    padding: 8px 12px;
    color: var(--text);
    min-width: 0;
    overflow: hidden;
  }
  .file.has-preview {
    flex-direction: column;
    align-items: stretch;
    gap: 4px;
    padding: 6px;
  }
  .preview {
    flex: 1;
    min-height: 0;
    width: 100%;
    object-fit: contain;
    user-select: none;
  }
  .caption {
    flex: none;
    font-size: 12px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: center;
  }
  .badge {
    flex: none;
    min-width: 40px;
    height: 48px;
    border-radius: var(--radius-sm);
    background: var(--fill-2, rgba(127, 127, 127, 0.18));
    display: grid;
    place-items: center;
    font: 600 10px/1 var(--font-mono, monospace);
    padding: 0 4px;
  }
  .meta {
    min-width: 0;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .actions {
    display: flex;
    gap: 6px;
    margin-top: 6px;
  }
  .act {
    font-size: 12px;
    padding: 3px 8px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--separator, rgba(127, 127, 127, 0.3));
    background: var(--fill-2, rgba(127, 127, 127, 0.12));
    color: var(--text);
    cursor: pointer;
  }
  .act:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .status {
    margin-top: 4px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .path {
    color: var(--text-muted);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
</style>
