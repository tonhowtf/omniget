<script lang="ts">
  /**
   * Catalog item detail: own viewers (Markdown with TOC and Cmd+F, JSON tree,
   * multi-file explorer with slides and token counts), metadata (licence,
   * author, source with link, version), referenced components (loops),
   * "How it looks in each tool", the guard report, and the stack / install
   * buttons. Files are fetched and checked (sha256 / git blob) by the
   * backend the first time the page opens them.
   */
  import { untrack } from "svelte";
  import { page } from "$app/state";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import GuardBadge from "$components/central/guard/GuardBadge.svelte";
  import GuardReportDialog from "$components/central/guard/GuardReportDialog.svelte";
  import CompatPanel from "$components/central/catalog/CompatPanel.svelte";
  import JsonView from "$components/central/catalog/JsonView.svelte";
  import KindIcon from "$components/central/catalog/KindIcon.svelte";
  import MarkdownView from "$components/central/catalog/MarkdownView.svelte";
  import SaveToCollection from "$components/central/catalog/SaveToCollection.svelte";
  import SkillExplorer from "$components/central/catalog/SkillExplorer.svelte";
  import RunAsLoop from "$components/central/run/RunAsLoop.svelte";
  import RunNow from "$components/central/run/RunNow.svelte";
  import { guardScanComponent, type GuardReport } from "$lib/central/guard";
  import {
    FOLDER_KINDS,
    agentkitDetect,
    agentkitTargets,
    catalogItem,
    catalogItemFiles,
    compactNumber,
    errText,
    formatBytes,
    itemHref,
    refKind,
    refToId,
    resolveComponent,
    sourceLabel,
    textFiles,
    type CatalogItem,
    type Component,
    type DetectedTarget,
    type ItemFiles,
    type TargetAdapter,
  } from "$lib/central/catalog";
  import { guardSeals, stack } from "$lib/central/stack-store.svelte";

  let id = $derived.by(() => {
    const raw = page.params.id ?? "";
    try {
      return decodeURIComponent(raw);
    } catch {
      return raw;
    }
  });

  let item = $state<CatalogItem | null>(null);
  let files = $state<ItemFiles | null>(null);
  let component = $state<Component | null>(null);
  let report = $state<GuardReport | null>(null);
  let error = $state("");
  let filesError = $state("");
  let componentError = $state("");
  let guardError = $state("");
  let scanning = $state(false);
  let showReport = $state(false);
  let tab = $state<"content" | "tools" | "security" | "refs">("content");
  let targets = $state<TargetAdapter[]>([]);
  let detected = $state<DetectedTarget[]>([]);

  $effect(() => {
    const current = id;
    let dead = false;
    item = null;
    files = null;
    component = null;
    report = untrack(() => guardSeals[current] ?? null);
    error = filesError = componentError = guardError = "";
    tab = "content";
    catalogItem(current)
      .then((it) => {
        if (dead) return;
        item = it;
        return catalogItemFiles(current).then((f) => {
          if (dead) return;
          files = f;
          void scan(it, f);
          resolveComponent(current)
            .then((c) => !dead && (component = c))
            .catch((e) => !dead && (componentError = errText(e)));
        }, (e) => !dead && (filesError = errText(e)));
      })
      .catch((e) => !dead && (error = errText(e)));
    return () => {
      dead = true;
    };
  });

  $effect(() => {
    agentkitTargets().then((x) => (targets = x)).catch(() => {});
    agentkitDetect(stack.scope === "global" ? null : stack.projectDir).then((d) => (detected = d)).catch(() => {});
  });

  async function scan(it: CatalogItem, f: ItemFiles) {
    scanning = true;
    guardError = "";
    try {
      const r = await guardScanComponent({
        kind: it.kind,
        files: textFiles(f),
        originTool: it.origin_tool,
        entry: f.entry || it.entry,
        label: it.id,
        options: {
          provenance: {
            repo: it.source.repo ?? null,
            url: it.source.url ?? null,
            commit: it.source.commit ?? null,
            author: it.author ?? null,
            license: it.license ?? null,
            version: typeof fm(it).version === "string" ? (fm(it).version as string) : null,
          },
          expected_sha256: Object.fromEntries(it.files.map((x) => [x.path, x.sha256])),
        },
      });
      report = r;
      guardSeals[it.id] = r;
    } catch (e) {
      guardError = errText(e);
    } finally {
      scanning = false;
    }
  }

  function fm(it: CatalogItem): Record<string, unknown> {
    return it.frontmatter && typeof it.frontmatter === "object" && !Array.isArray(it.frontmatter)
      ? (it.frontmatter as Record<string, unknown>)
      : {};
  }

  const META_KEYS = [
    "tools",
    "allowed-tools",
    "model",
    "argument-hint",
    "interval",
    "events",
    "module",
    "stop-condition",
    "version",
    "color",
    "tags",
  ];
  let chips = $derived.by(() => {
    if (!item) return [] as [string, string][];
    const f = fm(item);
    const out: [string, string][] = [];
    for (const k of META_KEYS) {
      const v = f[k];
      if (v == null || v === "") continue;
      if (Array.isArray(v) && v.every((x) => typeof x !== "object")) out.push([k, v.join(", ")]);
      else if (typeof v !== "object") out.push([k, String(v)]);
    }
    return out;
  });

  let entryPath = $derived(files?.entry || item?.entry || "");
  let entryText = $derived.by(() => {
    if (!files) return "";
    const f = files.files[entryPath] ?? Object.values(files.files)[0];
    return f && f.encoding === "utf8" ? f.content : "";
  });
  let multi = $derived(!!item && !!files && (FOLDER_KINDS.has(item.kind) || Object.keys(files.files).length > 1));
  let isJson = $derived(entryPath.toLowerCase().endsWith(".json"));
  let inStack = $derived(item ? stack.has(item.id) : false);
  let refs = $derived(item?.references ?? []);
  let version = $derived(item ? (fm(item).version as string | undefined) : undefined);
  let detectedIds = $derived(detected.filter((d) => d.installed).map((d) => d.id));

  function toStack(open: boolean) {
    if (!item) return;
    stack.add({ id: item.id, kind: item.kind, name: item.name, category: item.category, description: item.description });
    if (open) stack.open = true;
  }

  function addRefs() {
    if (!item) return;
    const n = stack.addMany(
      refs.map((r) => {
        const rid = refToId(r, item!.source.id);
        return { id: rid, kind: refKind(r), name: rid.split("/").pop() ?? rid };
      }),
    );
    showToast("success", $t("llm.central.catalog.stack.added_n", { count: String(n) }));
  }

  async function copyId() {
    try {
      await navigator.clipboard.writeText(item?.id ?? "");
      showToast("success", $t("llm.central.catalog.copied"));
    } catch {
      /* ignore */
    }
  }
</script>

<div class="detail">
  <div class="scroll">
    <nav class="crumbs">
      <a href="/llm/catalog">← {$t("llm.central.catalog.back")}</a>
    </nav>

    {#if error}
      <p class="err" role="alert">{error}</p>
    {:else if !item}
      <p class="muted">{$t("llm.central.catalog.loading")}</p>
    {:else}
      <header class="hero">
        <KindIcon kind={item.kind} size={64} />
        <div class="titles">
          <p class="eyebrow">{$t(`llm.central.catalog.kind.${item.kind}`)} · {item.category}</p>
          <h1>{item.name}</h1>
          {#if item.description}<p class="desc">{item.description}</p>{/if}
          <div class="badges">
            <GuardBadge {report} onclick={report ? () => (showReport = true) : undefined} />
            {#if scanning}<span class="muted small">{$t("llm.central.catalog.guard.scanning")}</span>{/if}
            {#if item.license}<span class="pill">{item.license}</span>{:else}<span class="pill warn">{$t("llm.central.catalog.meta.no_license")}</span>{/if}
            {#if item.stars}<span class="pill">★ {compactNumber(item.stars)}</span>{/if}
            {#if item.collides_with?.length}
              <span class="pill" title={item.collides_with.join(", ")}>{$t("llm.central.catalog.meta.installs_as", { name: item.install_name ?? item.name })}</span>
            {/if}
          </div>
        </div>
        <div class="actions">
          <button type="button" class="btn" class:on={inStack} aria-pressed={inStack} onclick={() => (inStack ? stack.remove(item!.id) : toStack(false))}>
            {inStack ? $t("llm.central.catalog.stack.in_stack") : $t("llm.central.catalog.stack.add")}
          </button>
          <button type="button" class="btn primary" onclick={() => toStack(true)}>{$t("llm.central.catalog.install")}</button>
          <SaveToCollection itemId={item.id} />
          {#if item.kind === "loop"}
            <RunAsLoop itemId={item.id} itemName={item.name} workspace={stack.scope === "global" ? null : stack.projectDir} />
          {:else if item.kind === "agent" || item.kind === "command" || item.kind === "skill"}
            <RunNow agentId={item.id} itemName={item.name} workspace={stack.scope === "global" ? null : stack.projectDir} />
          {/if}
        </div>
      </header>

      <div class="body">
        <main class="content">
          <div class="tabs" role="tablist">
            <button type="button" role="tab" aria-selected={tab === "content"} onclick={() => (tab = "content")}>{$t("llm.central.catalog.detail.content")}</button>
            <button type="button" role="tab" aria-selected={tab === "tools"} onclick={() => (tab = "tools")}>{$t("llm.central.catalog.detail.tools")}</button>
            <button type="button" role="tab" aria-selected={tab === "security"} onclick={() => (tab = "security")}>{$t("llm.central.catalog.detail.security")}</button>
            {#if refs.length}
              <button type="button" role="tab" aria-selected={tab === "refs"} onclick={() => (tab = "refs")}>{$t("llm.central.catalog.detail.refs")} ({refs.length})</button>
            {/if}
          </div>

          {#if tab === "content"}
            {#if filesError}
              <p class="err">{filesError}</p>
            {:else if !files}
              <p class="muted">{$t("llm.central.catalog.fetching_files")}</p>
            {:else if multi}
              <SkillExplorer {files} title={item.name} />
            {:else if isJson}
              <JsonView text={entryText} />
            {:else}
              <MarkdownView text={entryText} />
            {/if}
          {:else if tab === "tools"}
            {#if componentError}
              <p class="err">{componentError}</p>
            {:else if !component}
              <p class="muted">{filesError || $t("llm.central.catalog.compat.parsing")}</p>
            {:else}
              <CompatPanel
                {component}
                original={entryText}
                originalPath={entryPath}
                {targets}
                detected={detectedIds}
                projectDir={stack.scope === "global" ? null : stack.projectDir}
              />
            {/if}
          {:else if tab === "security"}
            {#if guardError}
              <p class="err">{guardError}</p>
            {:else if !report}
              <p class="muted">{scanning ? $t("llm.central.catalog.guard.scanning") : $t("llm.central.catalog.guard.waiting")}</p>
            {:else}
              <section class="guard">
                <div class="guard-head">
                  <GuardBadge {report} />
                  <span class="muted small">{$t("llm.central.catalog.guard.summary", { findings: String(report.findings.length), commands: String(report.commands.length) })}</span>
                  <button type="button" class="btn" onclick={() => (showReport = true)}>{$t("llm.central.catalog.guard.open")}</button>
                  <button type="button" class="btn" onclick={() => item && files && scan(item, files)} disabled={scanning}>{$t("llm.central.catalog.guard.rescan")}</button>
                </div>
                {#if report.commands.length}
                  <div class="cmds">
                    <strong>{$t("llm.central.catalog.plan.commands")}</strong>
                    {#each report.commands as c, i (i)}<code>{c.command}</code>{/each}
                  </div>
                {/if}
                <ul class="findings">
                  {#each report.findings.filter((f) => f.severity !== "info").slice(0, 30) as f, i (i)}
                    <li class={f.severity}><span class="sev">{f.severity}</span> <span class="mono">{f.code}</span> {f.detail} <span class="muted mono">{f.file}{f.line ? `:${f.line}` : ""}</span></li>
                  {/each}
                </ul>
              </section>
            {/if}
          {:else if tab === "refs"}
            <div class="refs-head">
              <p class="muted small">{$t("llm.central.catalog.detail.refs_hint")}</p>
              <button type="button" class="btn" onclick={addRefs}>{$t("llm.central.catalog.detail.add_refs")}</button>
            </div>
            <ul class="refs">
              {#each refs as r (r)}
                {@const rid = refToId(r, item.source.id)}
                <li>
                  <KindIcon kind={refKind(r)} size={24} />
                  <a href={itemHref(rid)}>{r}</a>
                  <button type="button" class="link" onclick={() => stack.add({ id: rid, kind: refKind(r), name: rid.split("/").pop() ?? rid })} disabled={stack.has(rid)}>
                    {stack.has(rid) ? $t("llm.central.catalog.stack.in_stack") : $t("llm.central.catalog.stack.add")}
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </main>

        <aside class="meta" aria-label={$t("llm.central.catalog.detail.meta")}>
          <dl>
            <div><dt>{$t("llm.central.catalog.meta.source")}</dt>
              <dd>
                {#if item.source.url}
                  <button type="button" class="link" onclick={() => item?.source.url && openUrl(item.source.url)}>{sourceLabel(item.source.id)}</button>
                {:else}{sourceLabel(item.source.id)}{/if}
                {#if item.source.repo}<span class="muted mono small">{item.source.repo}{item.source.commit ? `@${item.source.commit.slice(0, 7)}` : ""}</span>{/if}
              </dd>
            </div>
            {#if item.source.marketplace}<div><dt>{$t("llm.central.catalog.marketplace")}</dt><dd>{item.source.marketplace_name ?? item.source.marketplace}</dd></div>{/if}
            <div><dt>{$t("llm.central.catalog.meta.license")}</dt><dd>{item.license ?? $t("llm.central.catalog.meta.no_license")}</dd></div>
            {#if item.author}<div><dt>{$t("llm.central.catalog.meta.author")}</dt><dd>{item.author}</dd></div>{/if}
            {#if version}<div><dt>{$t("llm.central.catalog.meta.version")}</dt><dd>{version}</dd></div>{/if}
            {#if item.updated}<div><dt>{$t("llm.central.catalog.meta.updated")}</dt><dd>{item.updated}</dd></div>{/if}
            <div><dt>{$t("llm.central.catalog.meta.origin")}</dt><dd>{targets.find((x) => x.id === item?.origin_tool)?.name ?? item.origin_tool}</dd></div>
            {#if files}
              <div><dt>{$t("llm.central.catalog.meta.files")}</dt><dd>{Object.keys(files.files).length} · {formatBytes(files.total_bytes)}</dd></div>
              <div><dt>{$t("llm.central.catalog.meta.verified")}</dt><dd>{$t(`llm.central.catalog.verified.${files.verified}`)}</dd></div>
            {/if}
            {#each chips as [k, v] (k)}
              <div><dt>{k}</dt><dd class="mono small">{v}</dd></div>
            {/each}
            {#if item.tags.length}
              <div><dt>{$t("llm.central.catalog.meta.tags")}</dt><dd class="tags">{#each item.tags as tg (tg)}<span class="pill">{tg}</span>{/each}</dd></div>
            {/if}
            {#if item.normalization.length}
              <div><dt>{$t("llm.central.catalog.meta.normalization")}</dt><dd class="small muted">{item.normalization.join(", ")}</dd></div>
            {/if}
          </dl>
          <button type="button" class="link small" onclick={copyId}>{$t("llm.central.catalog.meta.copy_id")}</button>
        </aside>
      </div>
    {/if}
  </div>
</div>

{#if showReport && report}
  <GuardReportDialog {report} onclose={() => (showReport = false)} />
{/if}

<style>
  .detail {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .scroll {
    flex: 1;
    overflow: auto;
    padding: var(--space-3) var(--space-5) calc(var(--space-9) + var(--space-4));
  }
  .crumbs a {
    font-size: var(--text-sm);
    color: var(--accent-hi);
    text-decoration: none;
  }
  .hero {
    display: flex;
    align-items: flex-start;
    gap: var(--space-4);
    margin: var(--space-3) 0 var(--space-4);
    flex-wrap: wrap;
  }
  .titles {
    flex: 1;
    min-width: 260px;
  }
  .eyebrow {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
  }
  h1 {
    margin: 2px 0 var(--space-1);
    font-size: var(--text-xl);
    letter-spacing: var(--track-tight);
    word-break: break-word;
  }
  .desc {
    margin: 0 0 var(--space-2);
    color: var(--text-muted);
    max-width: 72ch;
  }
  .badges {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .pill {
    padding: 2px 8px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    font-size: var(--text-xs);
  }
  .pill.warn {
    color: var(--warning);
  }
  .actions {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    flex-wrap: wrap;
  }
  .btn {
    height: var(--control-h-lg);
    padding: 0 var(--space-4);
    border: none;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-base);
    font-weight: 500;
    cursor: pointer;
  }
  .btn:hover:not(:disabled) {
    background: var(--fill-2);
  }
  .btn.on {
    background: var(--accent-soft);
    color: var(--accent-hi);
  }
  .btn.primary {
    background: var(--cta);
    color: var(--on-cta);
  }
  .btn:disabled {
    opacity: 0.5;
  }
  .body {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 260px;
    gap: var(--space-5);
  }
  .content {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .tabs {
    display: flex;
    gap: var(--space-1);
    border-bottom: 1px solid var(--separator);
  }
  .tabs button {
    padding: 8px 10px;
    border: none;
    border-bottom: 2px solid transparent;
    background: none;
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: 500;
    cursor: pointer;
  }
  .tabs button[aria-selected="true"] {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .meta dl {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .meta dt {
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
  }
  .meta dd {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    display: flex;
    flex-direction: column;
    gap: 2px;
    word-break: break-word;
  }
  .meta .tags {
    flex-direction: row;
    flex-wrap: wrap;
    gap: 4px;
  }
  .link {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent-hi);
    font-size: inherit;
    text-align: left;
    cursor: pointer;
  }
  .link:disabled {
    color: var(--text-muted);
    cursor: default;
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: var(--text-xs);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .err {
    color: var(--error);
  }
  .guard {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .guard-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .cmds {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--warning) 10%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--warning) 40%, transparent);
    font-size: var(--text-sm);
  }
  .cmds code {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    word-break: break-all;
  }
  .findings {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--text-sm);
  }
  .sev {
    display: inline-block;
    min-width: 64px;
    font-size: var(--text-xs);
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .findings .critical .sev,
  .findings .high .sev {
    color: var(--error);
  }
  .findings .medium .sev {
    color: var(--warning);
  }
  .refs-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .refs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .refs li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .refs a {
    flex: 1;
    color: var(--text);
    font-family: var(--font-mono);
  }
  @media (max-width: 980px) {
    .body {
      grid-template-columns: 1fr;
    }
  }
</style>
