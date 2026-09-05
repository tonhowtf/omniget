<script lang="ts">
  /** Busca avançada e trends (estudo 67): monta a query com os operadores do X e roda no FxTwitter. */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { openUrl, reveal, saveAs } from "$lib/tools/rt";
  import { extOf, xErr, type ExportFormat, type XPost } from "$lib/tools/x";
  import PostCard from "./PostCard.svelte";

  type Page = { posts: XPost[]; cursor: string | null; source: string };
  type Trend = { name: string; context: string; rank: number | null };

  let words = $state("");
  let phrase = $state("");
  let any = $state("");
  let none = $state("");
  let from = $state("");
  let to = $state("");
  let mention = $state("");
  let since = $state("");
  let until = $state("");
  let minFaves = $state("");
  let minReposts = $state("");
  let lang = $state("");
  let media = $state<"" | "media" | "images" | "videos" | "links">("");
  let noReplies = $state(false);
  let noReposts = $state(true);
  let verified = $state(false);
  let feed = $state<"latest" | "top">("latest");
  let raw = $state("");
  let useRaw = $state(false);

  let busy = $state(false);
  let posts = $state<XPost[]>([]);
  let cursor = $state<string | null>(null);
  let source = $state("");
  let trends = $state<Trend[]>([]);

  let query = $derived.by(() => {
    if (useRaw) return raw.trim();
    const parts: string[] = [];
    if (words.trim()) parts.push(words.trim());
    if (phrase.trim()) parts.push(`"${phrase.trim().replace(/"/g, "")}"`);
    if (any.trim()) parts.push(`(${any.trim().split(/\s+/).join(" OR ")})`);
    if (none.trim()) parts.push(none.trim().split(/\s+/).map((w) => `-${w}`).join(" "));
    if (from.trim()) parts.push(`from:${from.trim().replace(/^@/, "")}`);
    if (to.trim()) parts.push(`to:${to.trim().replace(/^@/, "")}`);
    if (mention.trim()) parts.push(`@${mention.trim().replace(/^@/, "")}`);
    if (since) parts.push(`since:${since}`);
    if (until) parts.push(`until:${until}`);
    if (minFaves) parts.push(`min_faves:${minFaves}`);
    if (minReposts) parts.push(`min_retweets:${minReposts}`);
    if (lang.trim()) parts.push(`lang:${lang.trim()}`);
    if (media) parts.push(`filter:${media}`);
    if (noReplies) parts.push("-filter:replies");
    if (noReposts) parts.push("-filter:retweets");
    if (verified) parts.push("filter:blue_verified");
    return parts.join(" ");
  });

  async function run(more = false) {
    if (!query || busy) return;
    busy = true;
    if (!more) {
      posts = [];
      cursor = null;
    }
    try {
      const page = await invoke<Page>("tool_x_search", { query, feed, cursor: more ? cursor : null });
      posts = more ? [...posts, ...page.posts] : page.posts;
      cursor = page.cursor;
      source = page.source;
      if (!more && page.posts.length === 0) showToast("info", $t("tools.x.no_results") as string);
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = false;
    }
  }

  async function exportAs(format: ExportFormat) {
    if (!posts.length) return;
    const dest = await saveAs(`x-search.${extOf(format)}`);
    if (!dest) return;
    try {
      const path = await invoke<string>("tool_x_export_posts", { posts, format, dest, title: query });
      await reveal(path);
    } catch (e) {
      showToast("error", xErr(e));
    }
  }

  async function loadTrends() {
    try {
      trends = await invoke<Trend[]>("tool_x_trends");
    } catch {
      trends = [];
    }
  }

  function useTrend(name: string) {
    useRaw = false;
    words = name;
    run();
  }

  onMount(loadTrends);
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.x.search_builder")}</div><div class="group-row-sub">{$t("tools.x.search_intro")}</div></div>
        <div class="group-row-trailing"><label class="opt"><input class="checkbox" type="checkbox" bind:checked={useRaw} /> {$t("tools.x.raw_query")}</label></div>
      </div>
      {#if useRaw}
        <div class="group-row"><div class="group-row-content"><input class="input" type="text" bind:value={raw} placeholder="from:nasa filter:videos since:2026-01-01" onkeydown={(e) => e.key === "Enter" && run()} /></div></div>
      {:else}
        <div class="group-row"><div class="group-row-content grid">
          <label>{$t("tools.x.q_words")}<input class="input" type="text" bind:value={words} /></label>
          <label>{$t("tools.x.q_phrase")}<input class="input" type="text" bind:value={phrase} /></label>
          <label>{$t("tools.x.q_any")}<input class="input" type="text" bind:value={any} /></label>
          <label>{$t("tools.x.q_none")}<input class="input" type="text" bind:value={none} /></label>
          <label>{$t("tools.x.q_from")}<input class="input" type="text" bind:value={from} placeholder="@" /></label>
          <label>{$t("tools.x.q_to")}<input class="input" type="text" bind:value={to} placeholder="@" /></label>
          <label>{$t("tools.x.q_mention")}<input class="input" type="text" bind:value={mention} placeholder="@" /></label>
          <label>{$t("tools.x.q_lang")}<input class="input" type="text" bind:value={lang} placeholder="pt, en…" /></label>
          <label>{$t("tools.x.q_since")}<input class="input" type="date" bind:value={since} /></label>
          <label>{$t("tools.x.q_until")}<input class="input" type="date" bind:value={until} /></label>
          <label>{$t("tools.x.q_min_faves")}<input class="input" type="number" min="0" bind:value={minFaves} /></label>
          <label>{$t("tools.x.q_min_reposts")}<input class="input" type="number" min="0" bind:value={minReposts} /></label>
        </div></div>
        <div class="group-row">
          <div class="group-row-content opts">
            <select class="input" bind:value={media}>
              <option value="">{$t("tools.x.q_any_media")}</option>
              <option value="media">{$t("tools.x.q_with_media")}</option>
              <option value="images">{$t("tools.x.q_images")}</option>
              <option value="videos">{$t("tools.x.q_videos")}</option>
              <option value="links">{$t("tools.x.q_links")}</option>
            </select>
            <label class="opt"><input class="checkbox" type="checkbox" bind:checked={noReplies} /> {$t("tools.x.q_no_replies")}</label>
            <label class="opt"><input class="checkbox" type="checkbox" bind:checked={noReposts} /> {$t("tools.x.q_no_reposts")}</label>
            <label class="opt"><input class="checkbox" type="checkbox" bind:checked={verified} /> {$t("tools.x.q_verified")}</label>
          </div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-sub mono">{query || "…"}</div></div>
        <div class="group-row-trailing btn-row">
          <div class="segmented"><button class="segmented-btn" class:active={feed === "latest"} type="button" onclick={() => (feed = "latest")}>{$t("tools.x.feed_latest")}</button><button class="segmented-btn" class:active={feed === "top"} type="button" onclick={() => (feed = "top")}>{$t("tools.x.feed_top")}</button></div>
          <button class="btn btn-secondary btn-sm" type="button" disabled={!query} onclick={() => openUrl(`https://x.com/search?q=${encodeURIComponent(query)}&f=${feed === "latest" ? "live" : "top"}`)}>{$t("tools.x.open_on_x")}</button>
          <button class="btn btn-primary" type="button" disabled={busy || !query} onclick={() => run()}>{busy ? $t("tools.common.working") : $t("tools.x.search")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if trends.length && !posts.length}
    <section>
      <span class="group-label">{$t("tools.x.trends")}</span>
      <div class="group"><div class="group-row"><div class="group-row-content chips">
        {#each trends as tr (tr.name)}<button class="tag chip" type="button" title={tr.context} onclick={() => useTrend(tr.name)}>{tr.name}</button>{/each}
      </div></div></div>
    </section>
  {/if}

  {#if posts.length}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{posts.length} {$t("tools.x.posts")}</div><div class="group-row-sub">{$t("tools.x.source")}: {source}</div></div>
          <div class="group-row-trailing btn-row">
            {#each ["csv", "json", "md"] as f (f)}<button class="btn btn-secondary btn-sm" type="button" onclick={() => exportAs(f as ExportFormat)}>{f.toUpperCase()}</button>{/each}
          </div>
        </div>
        {#each posts as p (p.id)}<PostCard post={p} compact />{/each}
        {#if cursor}
          <div class="group-row"><div class="group-row-content"></div><div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={() => run(true)}>{busy ? $t("tools.common.working") : $t("tools.x.load_more")}</button></div></div>
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .opt { display: inline-flex; align-items: center; gap: var(--space-1); font-size: var(--text-sm); white-space: nowrap; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: var(--space-2); width: 100%; }
  .grid label { display: flex; flex-direction: column; gap: 2px; font-size: var(--text-xs); color: var(--text-muted); }
  .opts { display: flex; flex-direction: row; flex-wrap: wrap; align-items: center; gap: var(--space-3); }
  .opts select { max-width: 200px; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .chips { display: flex; flex-wrap: wrap; gap: var(--space-1); }
  .chip { cursor: pointer; border: 0; font: inherit; font-size: var(--text-xs); }
</style>
