<script lang="ts">
  /** Desenrolar thread (estudo 67): FxTwitter público, GraphQL da sessão como reserva. */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { reveal, saveAs } from "$lib/tools/rt";
  import { extOf, xErr, type ExportFormat, type XPost } from "$lib/tools/x";
  import PostCard from "./PostCard.svelte";

  type Thread = { focal: XPost; posts: XPost[]; truncated: boolean; source: string };

  let url = $state("");
  let busy = $state<string | null>(null);
  let thread = $state<Thread | null>(null);

  async function run() {
    if (!url.trim() || busy) return;
    busy = "run";
    thread = null;
    try {
      thread = await invoke<Thread>("tool_x_thread", { input: url });
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = null;
    }
  }

  function title(): string {
    const first = thread?.posts[0] ?? thread?.focal;
    const line = (first?.text ?? "").split("\n")[0].trim().slice(0, 80);
    return line || `Thread @${first?.author.handle ?? ""}`;
  }

  async function exportAs(format: ExportFormat) {
    if (!thread) return;
    const handle = thread.posts[0]?.author.handle ?? "x";
    const dest = await saveAs(`thread-${handle}-${thread.focal.id}.${extOf(format)}`);
    if (!dest) return;
    busy = format;
    try {
      const path = await invoke<string>("tool_x_export_posts", { posts: thread.posts, format, dest, title: title() });
      showToast("success", $t("tools.common.done") as string);
      await reveal(path);
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = null;
    }
  }

  async function copyMd() {
    if (!thread) return;
    try {
      const md = await invoke<string>("tool_x_render_posts", { posts: thread.posts, format: "md", title: title() });
      await navigator.clipboard.writeText(md);
      showToast("success", $t("tools.common.copied") as string);
    } catch (e) {
      showToast("error", xErr(e));
    }
  }

  let words = $derived(thread ? thread.posts.reduce((n, p) => n + p.text.split(/\s+/).filter(Boolean).length, 0) : 0);
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder={$t("tools.x.post_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} /></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy !== null || !url.trim()} onclick={run}>{busy === "run" ? $t("tools.common.working") : $t("tools.x.unroll")}</button></div>
      </div>
      <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{$t("tools.x.thread_intro")}</div></div></div>
    </div>
  </section>

  {#if thread}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{thread.posts.length} {$t("tools.x.posts")} · {words} {$t("tools.x.words")} · @{thread.posts[0]?.author.handle}</div>
            <div class="group-row-sub">{$t("tools.x.source")}: {thread.source}{#if thread.truncated} · {$t("tools.x.truncated")}{/if}</div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={copyMd}>{$t("tools.x.copy_md")}</button>
            {#each ["md", "html", "txt", "json"] as f (f)}
              <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={() => exportAs(f as ExportFormat)}>{busy === f ? "…" : f.toUpperCase()}</button>
            {/each}
          </div>
        </div>
      </div>
    </section>
    <section>
      <div class="group posts">
        {#each thread.posts as p, i (p.id)}<PostCard post={p} index={i} total={thread.posts.length} />{/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
</style>
