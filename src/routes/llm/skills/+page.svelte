<script lang="ts">
  import SurfaceGuide from "$components/llm/SurfaceGuide.svelte";
  /**
   * Skills tab: the installed Agent Skills (origin, allowed tools, agents
   * using each one), the three install sources and the OpenRouterTeam
   * showcase. Every read is one command on mount; nothing polls and nothing
   * reaches the network until the user clicks an install.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getAgents, loadRoster } from "$lib/stores/llm-store.svelte";
  import {
    clearSkillsNotice,
    confirmPendingInstall,
    discardPendingInstall,
    getBusySkill,
    getCatalog,
    getLastInstalled,
    getPendingInstall,
    getSkills,
    getSkillsErrorKey,
    installCatalogEntry,
    installFromDir,
    installFromGit,
    installFromRepo,
    installFromZip,
    isDemoSkills,
    isScannerAvailable,
    isSkillsAvailable,
    isSkillsLoading,
    loadCatalog,
    loadPendingInstall,
    loadScanner,
    loadSkills,
    removeSkill,
    type SkillCatalogEntry,
  } from "$lib/stores/llm-skills-store.svelte";
  import SkillList from "$components/llm/skills/SkillList.svelte";
  import SkillCatalog from "$components/llm/skills/SkillCatalog.svelte";
  import SkillInstallDialog from "$components/llm/skills/SkillInstallDialog.svelte";
  import SkillScanDialog from "$components/llm/skills/SkillScanDialog.svelte";
  import PackImport from "$components/llm/skills/PackImport.svelte";

  let skills = $derived(getSkills());
  let catalog = $derived(getCatalog());
  let agents = $derived(getAgents());
  let busy = $derived(getBusySkill());
  let errorKey = $derived(getSkillsErrorKey());
  let pending = $derived(getPendingInstall());
  let section = $state<"installed" | "explore">("installed");
  let query = $state("");
  let installing = $state(false);
  let repoSpec = $state("");
  let repoBusy = $state(false);

  async function addFromRepo() {
    const spec = repoSpec.trim();
    if (!spec || repoBusy) return;
    repoBusy = true;
    try {
      if (await installFromRepo(spec)) repoSpec = "";
    } finally {
      repoBusy = false;
    }
  }

  onMount(() => {
    void loadSkills();
    void loadCatalog();
    void loadRoster();
    void loadScanner();
    // An install the user left undecided comes back instead of rotting in a
    // quarantine folder nobody can name again.
    void loadPendingInstall();
  });

  function openInstall() {
    clearSkillsNotice();
    installing = true;
  }

  /**
   * An install that came back parked is not a failure and not a success: the
   * source dialog closes so the scan dialog is the only thing on screen, but
   * nothing has been installed.
   */
  function settle(installed: boolean) {
    if (installed || getPendingInstall()) installing = false;
  }

  async function fromDir(path: string) {
    settle(await installFromDir(path));
  }

  async function fromZip(path: string) {
    settle(await installFromZip(path));
  }

  async function fromGit(url: string) {
    settle(await installFromGit(url));
  }

  function fromCatalog(entry: SkillCatalogEntry) {
    void installCatalogEntry(entry);
  }
</script>

<svelte:head><title>{$t("llm.tab.skills")}</title></svelte:head>

<div class="page page-wide skills-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("llm.skills.title")}</h1>
      <p class="page-lede">{$t("llm.skills.lede")}</p>
    </div>
    <button type="button" class="button primary" onclick={openInstall}>
      {$t("llm.skills.install_title")}
    </button>
  </header>
  <SurfaceGuide text={$t("llm.surface.skills_hint")} href="/help?article=skills#guide" />

  {#if isDemoSkills()}
    <p class="notice" role="status">{$t("llm.skills.demo")}</p>
  {:else if !isSkillsAvailable()}
    <p class="notice" role="status">{$t("llm.skills.err_unavailable")}</p>
  {/if}

  {#if isScannerAvailable() === false}
    <!-- Say it plainly: with no scanner installed nothing is checked, and the
         "not scanned" badges on the cards below mean exactly that. -->
    <p class="notice" role="status">{$t("llm.skills.scanner_missing")}</p>
  {/if}

  {#if errorKey && !installing}
    <p class="notice error" role="alert">{$t(errorKey)}</p>
    <button type="button" class="button" disabled={isSkillsLoading()} onclick={() => { void loadSkills(true); void loadCatalog(true); }}>{$t("llm.surface.retry")}</button>
  {/if}

  {#if getLastInstalled()}
    <p class="notice" role="status">
      {$t("llm.skills.installed_ok", { name: getLastInstalled() })}
      <a href="/llm/roster">{$t("llm.skills.assign")} →</a>
    </p>
  {/if}

  <div class="skill-tabs" role="group" aria-label={$t("llm.skills.title")}>
    <button class="button" type="button" aria-pressed={section === "installed"} onclick={() => section = "installed"}>{$t("llm.skills.installed_title")} · {skills.length}</button>
    <button class="button" type="button" aria-pressed={section === "explore"} onclick={() => section = "explore"}>{$t("llm.skills.explore")}</button>
  </div>
  {#if section === "explore"}
  <details class="repo-settings"><summary>{$t("llm.skills.repo_title")}</summary>
  <section class="block">
    <form class="repo-add" onsubmit={(e) => { e.preventDefault(); void addFromRepo(); }}>
      <input
        class="repo-input"
        type="text"
        aria-label={$t("llm.skills.repo_title")}
        bind:value={repoSpec}
        placeholder="owner/repo · owner/repo/path · owner/repo@skill"
        spellcheck="false"
        autocapitalize="off"
      />
      <button type="submit" class="button" disabled={repoBusy || !repoSpec.trim()}>
        {repoBusy ? $t("llm.skills.repo_installing") : $t("llm.skills.repo_add")}
      </button>
    </form>
    <p class="dim">{$t("llm.skills.repo_hint")}</p>
  </section>
  </details>

  {/if}
  {#if section === "installed"}
  <section class="block">
    <label class="field"><span class="field-label">{$t("llm.skills.search_installed")}</span><input class="input" type="search" bind:value={query} /></label>
    <h2 class="block-title">
      {$t("llm.skills.installed_title")}
      <span class="count">{skills.length}</span>
    </h2>
    {#if isSkillsLoading() && skills.length === 0}
      <p class="dim">{$t("llm.skills.loading")}</p>
    {:else}
      <SkillList skills={skills.filter(s => `${s.name} ${s.description}`.toLowerCase().includes(query.trim().toLowerCase()))} {agents} {busy} onremove={(name) => void removeSkill(name)} />
    {/if}
  </section>

  {:else}
  <section class="block">
    <SkillCatalog entries={catalog} installed={skills} {busy} oninstall={fromCatalog} />
  </section>
  {/if}

  <section class="block"><PackImport /></section>
</div>

{#if pending}
  <SkillScanDialog
    {pending}
    busy={busy !== null}
    onconfirm={() => void confirmPendingInstall()}
    ondiscard={() => void discardPendingInstall()}
  />
{/if}

{#if installing}
  <SkillInstallDialog
    busy={busy !== null}
    {errorKey}
    oninstalldir={(p) => void fromDir(p)}
    oninstallzip={(p) => void fromZip(p)}
    oninstallgit={(u) => void fromGit(u)}
    onclose={() => (installing = false)}
  />
{/if}

<style>
  .repo-settings { margin:20px 0; } .repo-settings summary { cursor:pointer; padding:12px 0; font-weight:600; color:var(--text); }
  .skill-tabs { display: flex; gap: var(--space-3); margin-bottom: var(--space-5); }
  .skill-tabs [aria-pressed="true"] { background: var(--accent-soft); color: var(--accent-hi); }
  .repo-add {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }
  .repo-input {
    flex: 1;
    min-width: 0;
    padding: 6px 10px;
    border-radius: var(--radius-md, 8px);
    border: 1px solid var(--separator, rgba(127, 127, 127, 0.3));
    background: var(--fill-quaternary, rgba(127, 127, 127, 0.08));
    color: var(--text);
    font: inherit;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-sm);
  }

  /* Block layout on purpose, same reason as the roster page: as a flex column
     the grids would shrink to the viewport and clip their own cards. */
  .skills-page {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .block {
    margin-bottom: var(--space-5);
  }

  .block-title {
    margin: 0 0 var(--space-3);
    font-size: var(--text-md);
    font-weight: 600;
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
  }

  .count {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-dim);
  }

  .notice {
    margin: 0 0 var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .notice.error {
    color: var(--danger);
  }

  .dim {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
