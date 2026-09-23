<script lang="ts">
  // Um cartão da grade de ferramentas: estado, versão instalada × mais nova,
  // dono provado e as ações (o comando aparece antes, no painel de plano).
  import { t } from "$lib/i18n";
  import type { Latest, ToolRow } from "$lib/central/clitools";
  import type { ToolUpdate } from "./config-health";

  let {
    row,
    latest = null,
    busy = false,
    running = false,
    oninstall,
    onupdate,
    onlogin,
    ondoctor,
    update = null,
    onconfig,
  }: {
    row: ToolRow;
    latest?: Latest | null;
    busy?: boolean;
    running?: boolean;
    oninstall: () => void;
    onupdate: () => void;
    onlogin: () => void;
    ondoctor: () => void;
    update?: ToolUpdate | null;
    onconfig?: () => void;
  } = $props();

  let det = $derived(row.detection);
  let tool = $derived(row.tool);
  let installed = $derived(det?.installed ?? false);
  let hasCli = $derived(tool.binaries.length > 0 && (det?.binary ?? null) !== null);
  let canLogin = $derived(installed && hasCli);
  let behind = $derived(latest?.state === "behind_latest" || !!update?.update_available);

  function hue(id: string): number {
    let h = 0;
    for (const c of id) h = (h * 31 + c.charCodeAt(0)) % 360;
    return h;
  }
  function initials(name: string): string {
    const parts = name.replace(/[()]/g, "").split(/\s+/).filter(Boolean);
    return ((parts[0]?.[0] ?? "?") + (parts[1]?.[0] ?? "")).toUpperCase();
  }
  const statusTag: Record<string, string> = {
    active: "tag-success",
    maintenance: "tag-warning",
    discontinued: "tag-danger",
  };
</script>

<article class="surface-card tool-card" class:installed class:off={tool.status === "discontinued"}>
  <header class="head">
    <span class="symbol-tile symbol-tile-lg mono" style={`--h:${hue(tool.id)}`} aria-hidden="true">
      {initials(tool.name)}
    </span>
    <div class="title">
      <h3>{tool.name}</h3>
      <div class="tags">
        <span class={`tag ${statusTag[tool.status] ?? ""}`}>{$t(`llm.central.tools.status.${tool.status}`)}</span>
        <span class="tag">{$t(`llm.central.tools.kind.${tool.kind === "cli+app" ? "cli_app" : tool.kind}`)}</span>
        {#if tool.acp.registry_id || tool.acp.native_args || tool.acp.adapter}
          <span class="tag tag-info">ACP</span>
        {/if}
      </div>
    </div>
  </header>

  <dl class="facts">
    <div>
      <dt>{$t("llm.central.tools.installed")}</dt>
      <dd>
        {#if det === null}
          <span class="dim">—</span>
        {:else if installed}
          <code>{det.version ?? det.app_version ?? "?"}</code>
        {:else}
          <span class="dim">{$t("llm.central.tools.not_installed")}</span>
        {/if}
      </dd>
    </div>
    <div>
      <dt>{$t("llm.central.tools.latest")}</dt>
      <dd>
        {#if latest?.latest}
          <code class:behind>{latest.latest}</code>
          {#if behind}<span class="tag tag-warning">{$t("llm.central.tools.behind")}</span>{/if}
          {#if latest.state === "current"}<span class="tag tag-success">{$t("llm.central.tools.current")}</span>{/if}
        {:else}
          <span class="dim">—</span>
        {/if}
      </dd>
    </div>
    {#if det?.installed && det.binary}
      <div class="wide">
        <dt>{$t("llm.central.tools.owner")}</dt>
        <dd>
          <span class={`tag ${det.owner.proven ? "tag-accent" : ""}`}>
            {$t(`llm.central.tools.owner_method.${det.owner.method}`)}
            {det.owner.proven ? "✓" : "?"}
          </span>
          <span class="evidence" title={det.real_path ?? ""}>{det.owner.evidence}</span>
        </dd>
      </div>
    {/if}
  </dl>

  {#if tool.status_note}<p class="note">{tool.status_note}</p>{/if}

  {#if update?.update_available}
    <div class="update">
      <span class="tag tag-warning">{$t("llm.central.health.update_available")}</span>
      {#if update.latest}<code>{update.installed ?? "?"} → {update.latest}</code>{/if}
      {#if update.changelog}
        <details class="changelog">
          <summary>{$t("llm.central.health.changelog")} · {update.changelog.name ?? update.changelog.tag}</summary>
          <pre>{update.changelog.excerpt}{update.changelog.truncated ? "\n…" : ""}</pre>
          {#if update.changelog.url}
            <a href={update.changelog.url} target="_blank" rel="noreferrer">{$t("llm.central.health.changelog_full")}</a>
          {/if}
        </details>
      {/if}
    </div>
  {/if}

  <footer class="actions">
    {#if !installed}
      <button class="button active" type="button" disabled={busy || running} onclick={oninstall}>
        {$t("llm.central.tools.install")}
      </button>
    {:else if hasCli}
      <button class="button" class:active={behind} type="button" disabled={busy || running} onclick={onupdate}>
        {$t("llm.central.tools.update")}
      </button>
    {/if}
    {#if canLogin}
      <button class="button" type="button" disabled={busy} onclick={onlogin}>{$t("llm.central.tools.login")}</button>
    {/if}
    {#if installed && onconfig}
      <button class="button" type="button" disabled={busy} onclick={onconfig}>{$t("llm.central.health.tab.config")}</button>
    {/if}
    <button class="button" type="button" disabled={busy} onclick={ondoctor}>{$t("llm.central.tools.doctor")}</button>
    <a class="docs" href={tool.docs} target="_blank" rel="noreferrer">{$t("llm.central.tools.docs")}</a>
    {#if running}<span class="tag tag-info running">{$t("llm.central.tools.running")}</span>{/if}
  </footer>
</article>

<style>
  .tool-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    min-width: 0;
  }
  .tool-card.off {
    opacity: 0.72;
  }
  .head {
    display: flex;
    gap: var(--space-3);
    align-items: center;
  }
  .mono {
    background: hsl(var(--h) 55% 45%);
    color: #fff;
    font-weight: 700;
    font-size: var(--text-sm);
    letter-spacing: 0.02em;
  }
  .title {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  h3 {
    margin: 0;
    font-size: var(--text-md);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .facts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2) var(--space-3);
    margin: 0;
  }
  .facts .wide {
    grid-column: 1 / -1;
  }
  dt {
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  dd {
    margin: 2px 0 0;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    min-width: 0;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }
  code.behind {
    color: var(--warning);
  }
  .dim {
    color: var(--text-dim);
  }
  .evidence {
    font-size: var(--text-caption);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .note {
    margin: 0;
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .update {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
  }
  .changelog {
    flex-basis: 100%;
    font-size: var(--text-caption);
    color: var(--text-muted);
  }
  .changelog summary {
    cursor: pointer;
  }
  .changelog pre {
    margin: 6px 0;
    white-space: pre-wrap;
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    max-height: 180px;
    overflow: auto;
    user-select: text;
  }
  .actions {
    margin-top: auto;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .docs {
    font-size: var(--text-caption);
    color: var(--text-muted);
  }
  .running {
    margin-left: auto;
  }
</style>
