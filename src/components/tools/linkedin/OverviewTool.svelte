<script lang="ts">
  /** Panorama do export oficial do LinkedIn: inventario, conexoes, mensagens, posts e anuncios. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openPath, pct, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Count = { name: string; count: number };
  type Bucket = { period: string; count: number };
  type Simple = { total: number; by_year: Bucket[]; kinds: Count[] };
  type AdItem = { field: string; values: string[]; count: number };
  type Overview = {
    path: string;
    files: { name: string; rows: number }[];
    entries: number;
    missing: string[];
    connections: { total: number; with_email: number; without_email: number; first: string; last: string; by_year: Bucket[]; top_companies: Count[]; top_positions: Count[] };
    messages: { total: number; conversations: number; sent: number; received: number; me: string; by_year: Bucket[]; top_contacts: Count[] };
    posts: { total: number; with_media: number; first: string; last: string; by_year: Bucket[]; visibility: Count[] };
    reactions: Simple;
    comments: Simple;
    invitations: { sent: number; received: number; by_year: Bucket[] };
    follows: Simple;
    ad_targeting: AdItem[];
    ad_targeting_values: number;
    exports: string[];
  };

  let path = $state("");
  let exportJson = $state(true);
  let exportMarkdown = $state(true);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Overview | null>(null);
  let unlisten: (() => void) | null = null;

  const ZIP = [{ name: "Zip", extensions: ["zip"] }];
  const peak = (b: Bucket[]) => b.reduce((m, x) => Math.max(m, x.count), 1);

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "li-overview") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!path || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Overview>("tool_li_overview", {
        opts: { path, export_json: exportJson, export_markdown: exportMarkdown },
      });
      showToast("success", `${result.connections.total} · ${result.messages.total}`);
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
          <div class="group-row-title">{$t("tools.lidata.source")}</div>
          <div class="group-row-sub mono">{path || $t("tools.lidata.source_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(ZIP); if (f) path = f; }}>{$t("tools.lidata.pick_zip")}</button>
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) path = d; }}>{$t("tools.lidata.pick_folder")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.lidata.exports")}</div><div class="group-row-sub">{$t("tools.lidata.exports_sub")}</div></div>
        <div class="group-row-trailing btn-row">
          <label class="check"><input type="checkbox" bind:checked={exportJson} />JSON</label>
          <label class="check"><input type="checkbox" bind:checked={exportMarkdown} />Markdown</label>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !path} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.lidata.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="cards">
        <div class="card"><b>{result.connections.total}</b><span>{$t("tools.lidata.connections")}</span></div>
        <div class="card"><b>{result.messages.total}</b><span>{$t("tools.lidata.messages")}</span></div>
        <div class="card"><b>{result.messages.conversations}</b><span>{$t("tools.lidata.conversations")}</span></div>
        <div class="card"><b>{result.posts.total}</b><span>{$t("tools.lidata.posts")}</span></div>
        <div class="card"><b>{result.reactions.total}</b><span>{$t("tools.lidata.reactions")}</span></div>
        <div class="card"><b>{result.comments.total}</b><span>{$t("tools.lidata.comments")}</span></div>
        <div class="card"><b>{result.invitations.sent}</b><span>{$t("tools.lidata.invites_sent")}</span></div>
        <div class="card"><b>{result.invitations.received}</b><span>{$t("tools.lidata.invites_received")}</span></div>
        <div class="card"><b>{result.follows.total}</b><span>{$t("tools.lidata.follows")}</span></div>
      </div>
    </section>

    {#if result.ad_targeting.length}
      <section>
        <div class="group ads">
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{$t("tools.lidata.ads_title")}</div>
              <div class="group-row-sub">{$t("tools.lidata.ads_note")} · {result.ad_targeting_values} {$t("tools.lidata.ads_values")}</div>
            </div>
          </div>
          {#each result.ad_targeting as a (a.field)}
            <div class="group-row">
              <div class="group-row-content">
                <div class="group-row-title">{a.field} <span class="dim">{a.count}</span></div>
                <div class="group-row-sub">{a.values.join(" · ")}</div>
              </div>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.lidata.connections")}</div>
            <div class="group-row-sub">{result.connections.first} → {result.connections.last} · {result.connections.without_email} {$t("tools.lidata.without_email")}</div>
          </div>
        </div>
        {#each result.connections.by_year as b (b.period)}
          <div class="group-row bar-row">
            <div class="group-row-content"><span class="mono">{b.period}</span><div class="bar"><div class="bar-fill" style:width="{(b.count / peak(result.connections.by_year)) * 100}%"></div></div></div>
            <div class="group-row-trailing">{b.count}</div>
          </div>
        {/each}
      </div>
    </section>

    <section>
      <div class="cols">
        <div class="group">
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.lidata.top_companies")}</div></div></div>
          {#each result.connections.top_companies.slice(0, 12) as c (c.name)}
            <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{c.name}</div></div><div class="group-row-trailing">{c.count}</div></div>
          {/each}
        </div>
        <div class="group">
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.lidata.top_positions")}</div></div></div>
          {#each result.connections.top_positions.slice(0, 12) as c (c.name)}
            <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{c.name}</div></div><div class="group-row-trailing">{c.count}</div></div>
          {/each}
        </div>
      </div>
    </section>

    {#if result.messages.total}
      <section>
        <div class="group">
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{$t("tools.lidata.messages")}</div>
              <div class="group-row-sub">{result.messages.sent} {$t("tools.lidata.sent")} · {result.messages.received} {$t("tools.lidata.received")}{#if result.messages.me} · {result.messages.me}{/if}</div>
            </div>
          </div>
          {#each result.messages.by_year as b (b.period)}
            <div class="group-row bar-row">
              <div class="group-row-content"><span class="mono">{b.period}</span><div class="bar"><div class="bar-fill" style:width="{(b.count / peak(result.messages.by_year)) * 100}%"></div></div></div>
              <div class="group-row-trailing">{b.count}</div>
            </div>
          {/each}
          {#each result.messages.top_contacts.slice(0, 10) as c (c.name)}
            <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{c.name}</div></div><div class="group-row-trailing">{c.count}</div></div>
          {/each}
        </div>
      </section>
    {/if}

    <section>
      <div class="group">
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.lidata.files")}</div><div class="group-row-sub">{result.entries}</div></div></div>
        {#each result.files as f (f.name)}
          <div class="group-row"><div class="group-row-content"><div class="group-row-sub mono">{f.name}</div></div><div class="group-row-trailing">{f.rows}</div></div>
        {/each}
        {#if result.missing.length}
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.lidata.missing")}</div><div class="group-row-sub mono">{result.missing.join(" · ")}</div></div></div>
        {/if}
      </div>
    </section>

    {#if result.exports.length}
      <section>
        <div class="group">
          {#each result.exports as e (e)}
            <div class="group-row">
              <div class="group-row-content"><div class="group-row-sub mono">{e}</div></div>
              <div class="group-row-trailing btn-row">
                <button class="btn btn-ghost btn-sm" type="button" onclick={() => openPath(e)}>{$t("tools.common.open")}</button>
                <button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(e)}>{$t("tools.common.reveal")}</button>
              </div>
            </div>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .check { display: inline-flex; align-items: center; gap: var(--space-2); font-size: var(--text-sm); }
  .cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); gap: var(--space-3); }
  .card { background: var(--surface-2, rgba(127, 127, 127, 0.08)); border-radius: var(--radius-md, 10px); padding: var(--space-3); display: flex; flex-direction: column; gap: 2px; }
  .card b { font-size: var(--text-xl); }
  .card span { font-size: var(--text-xs); color: var(--text-dim); }
  .cols { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: var(--space-4); }
  .bar-row .group-row-content { display: flex; align-items: center; gap: var(--space-3); }
  .bar { flex: 1; height: 6px; border-radius: 3px; background: rgba(127, 127, 127, 0.18); overflow: hidden; }
  .bar-fill { height: 100%; background: #0a66c2; }
  .ads { border: 1px solid #0a66c2; }
</style>
