<script lang="ts">
  /**
   * Companies of agents: the list, "new company" (templates or blank) and
   * importing a package. Templates and packages go through the same
   * preview → apply pair so the owner sees the plan before anything exists.
   */
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { errorText, onCompanyChange, rpc, type Company, type ImportPlan, type Staff, type TemplateInfo } from "$lib/company/rpc";
  import { pickFolder } from "$lib/stores/llm-jobs-store.svelte";
  import ImportPlanView from "$components/company/ImportPlanView.svelte";
  import { avatarColor } from "$components/company/company-ui";

  type Row = { company: Company; agents: number; running: number };

  let rows = $state<Row[]>([]);
  let loaded = $state(false);
  let loadError = $state("");
  let templates = $state<TemplateInfo[]>([]);

  // New company sheet
  let creating = $state(false);
  let choice = $state<string>("blank");
  let name = $state("");
  let mission = $state("");
  let workspace = $state<string | null>(null);
  let plan = $state<ImportPlan | null>(null);
  /** Package read from a file (import), null for templates and blank. */
  let pkg = $state<unknown>(null);
  let pkgName = $state("");
  let busy = $state(false);
  let formError = $state("");
  let fileInput: HTMLInputElement | undefined = $state();

  async function load() {
    try {
      const list = await rpc<Company[]>("company.list");
      const staff = await Promise.all(list.map((c) => rpc<Staff[]>("staff.list", { company: c.id }).catch(() => [] as Staff[])));
      rows = list.map((company, i) => ({
        company,
        agents: staff[i].filter((s) => s.status !== "terminated").length,
        running: staff[i].filter((s) => s.status === "running").length,
      }));
      loadError = "";
    } catch (e) {
      loadError = errorText(e);
    } finally {
      loaded = true;
    }
  }

  onMount(() => {
    void load();
    rpc<TemplateInfo[]>("company.templates").then((l) => (templates = l)).catch(() => {});
    const un = onCompanyChange(null, () => void load());
    return () => void un.then((f) => f());
  });

  let visible = $derived(rows.filter((r) => r.company.status !== "archived"));
  let archived = $derived(rows.filter((r) => r.company.status === "archived"));

  function openCreate() {
    creating = true;
    choice = "blank";
    name = "";
    mission = "";
    workspace = null;
    plan = null;
    pkg = null;
    formError = "";
  }

  function close() {
    if (busy) return;
    creating = false;
    plan = null;
    pkg = null;
  }

  function pickTemplate(id: string) {
    choice = id;
    plan = null;
    formError = "";
  }

  function options() {
    return {
      name: name.trim() || null,
      mission: mission.trim() || null,
      workspace,
    };
  }

  function source(): Record<string, unknown> {
    return pkg ? { package: pkg } : { template: choice };
  }

  async function preview() {
    busy = true;
    formError = "";
    try {
      plan = await rpc<ImportPlan>("import.preview", { ...source(), options: options() });
    } catch (e) {
      formError = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function apply() {
    if (!plan) return;
    busy = true;
    formError = "";
    try {
      const c = await rpc<Company>("import.apply", { ...source(), options: options(), plan_hash: plan.plan_hash });
      creating = false;
      await goto(`/llm/company/${c.id}`);
    } catch (e) {
      formError = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function createBlank() {
    if (!name.trim()) return;
    busy = true;
    formError = "";
    try {
      const c = await rpc<Company>("company.create", {
        company: { name: name.trim(), mission: mission.trim(), workspace },
      });
      creating = false;
      await goto(`/llm/company/${c.id}`);
    } catch (e) {
      formError = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function onFile(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    openCreate();
    choice = "package";
    pkgName = file.name;
    try {
      pkg = JSON.parse(await file.text());
    } catch {
      pkg = null;
      formError = $t("company.list.import_bad_json");
      return;
    }
    await preview();
  }
</script>

<svelte:head><title>{$t("company.list.title")}</title></svelte:head>

<input bind:this={fileInput} type="file" accept=".json,application/json" hidden onchange={onFile} />

<div class="page company-list-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("company.list.title")}</h1>
      <p class="page-lede">{$t("company.list.lede")}</p>
    </div>
    {#if rows.length}
      <div class="head-actions">
        <button type="button" class="button" onclick={() => fileInput?.click()}>{$t("company.list.import")}</button>
        <button type="button" class="button active" onclick={openCreate}>{$t("company.list.new")}</button>
      </div>
    {/if}
  </header>

  {#if loadError}
    <p class="field-error">{loadError}</p>
  {/if}

  {#if !loaded}
    <div class="group">
      {#each [0, 1] as i (i)}<div class="group-row"><div class="skeleton" style="height: 36px; flex: 1"></div></div>{/each}
    </div>
  {:else if rows.length === 0}
    <div class="empty-state">
      <img class="empty-state-art" src="/emoji/paperclip.png" alt="" width="72" height="72" draggable="false" />
      <div class="empty-state-title">{$t("company.list.empty_title")}</div>
      <p class="empty-state-hint">{$t("company.list.empty_hint")}</p>
      <div class="empty-actions">
        <button type="button" class="button active" onclick={openCreate}>{$t("company.list.empty_cta")}</button>
        <button type="button" class="button" onclick={() => fileInput?.click()}>{$t("company.list.import")}</button>
      </div>
    </div>
  {:else}
    <div class="group">
      {#each visible as r (r.company.id)}
        {@render row(r)}
      {/each}
    </div>
    {#if archived.length}
      <h2 class="group-label archived-label">{$t("company.list.archived")}</h2>
      <div class="group">
        {#each archived as r (r.company.id)}
          {@render row(r)}
        {/each}
      </div>
    {/if}
  {/if}
</div>

{#snippet row(r: Row)}
  <a class="group-row company-row" href="/llm/company/{r.company.id}">
    <span class="tile" style:background={avatarColor(r.company.name)} aria-hidden="true">{r.company.issue_prefix}</span>
    <span class="group-row-content">
      <span class="group-row-title">{r.company.name}</span>
      <span class="group-row-sub mission">{r.company.mission || $t("company.list.no_mission")}</span>
    </span>
    <span class="group-row-trailing">
      {#if r.running > 0}
        <span class="tag tag-success"><span class="live"></span>{$t("company.list.running", { count: r.running })}</span>
      {/if}
      <span class="meta">{$t("company.list.agents", { count: r.agents })}</span>
      {#if r.company.status !== "active"}
        <span class="tag {r.company.status === 'paused' ? 'tag-warning' : ''}">{$t(`company.common.company_status.${r.company.status}`)}</span>
      {/if}
      <span class="chevron" aria-hidden="true">›</span>
    </span>
  </a>
{/snippet}

{#if creating}
  <div class="dialog-backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && close()} onkeydown={(e) => e.key === "Escape" && close()}>
    <div class="dialog-card create-card" role="dialog" aria-modal="true" aria-labelledby="create-title">
      <h2 class="dialog-title" id="create-title">
        {choice === "package" ? $t("company.list.import_title", { file: pkgName }) : $t("company.list.new_title")}
      </h2>

      {#if choice !== "package"}
        <div class="gallery" role="radiogroup" aria-label={$t("company.list.templates")}>
          <button type="button" class="tpl" role="radio" aria-checked={choice === "blank"} onclick={() => pickTemplate("blank")}>
            <span class="tpl-name">{$t("company.list.blank")}</span>
            <span class="tpl-summary">{$t("company.list.blank_hint")}</span>
          </button>
          {#each templates as tpl (tpl.id)}
            <button type="button" class="tpl" role="radio" aria-checked={choice === tpl.id} onclick={() => pickTemplate(tpl.id)}>
              <span class="tpl-name">{tpl.name}</span>
              <span class="tpl-summary">{tpl.summary}</span>
              <span class="tpl-roles">{tpl.roles.join(" · ")}</span>
            </button>
          {/each}
        </div>
      {/if}

      <label class="field">
        <span class="field-label">{$t("company.list.name")}</span>
        <input
          class="input"
          type="text"
          bind:value={name}
          placeholder={choice === "blank" ? "" : (templates.find((x) => x.id === choice)?.name ?? "")}
          oninput={() => (plan = null)}
        />
      </label>
      <label class="field">
        <span class="field-label">{$t("company.list.mission")}</span>
        <textarea class="input" rows="2" bind:value={mission} placeholder={$t("company.list.mission_placeholder")} oninput={() => (plan = null)}></textarea>
      </label>
      <div class="field">
        <span class="field-label">{$t("company.list.workspace")}</span>
        <div class="folder">
          <button type="button" class="button" onclick={async () => { workspace = (await pickFolder()) ?? workspace; plan = null; }}>{$t("company.common.pick_folder")}</button>
          {#if workspace}
            <code class="path" title={workspace}>{workspace}</code>
            <button type="button" class="button" onclick={() => (workspace = null)}>{$t("company.common.clear")}</button>
          {:else}
            <span class="hint">{$t("company.list.workspace_hint")}</span>
          {/if}
        </div>
      </div>

      {#if plan}
        <ImportPlanView {plan} />
      {/if}

      {#if formError}<p class="field-error">{formError}</p>{/if}

      <div class="dialog-actions">
        <button type="button" class="button" disabled={busy} onclick={close}>{$t("company.common.cancel")}</button>
        {#if choice === "blank"}
          <button type="button" class="button active" disabled={busy || !name.trim()} onclick={createBlank}>{$t("company.list.create")}</button>
        {:else if plan}
          <button type="button" class="button active" disabled={busy} onclick={apply}>{$t("company.list.apply")}</button>
        {:else}
          <button type="button" class="button active" disabled={busy || (choice === "package" && !pkg)} onclick={preview}>{$t("company.list.review")}</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .company-list-page { flex: 1; min-height: 0; overflow-y: auto; }
  .head-actions, .empty-actions { display: flex; gap: var(--space-2); }
  .archived-label { margin-top: var(--space-5); }
  .company-row { text-decoration: none; color: inherit; }
  @media (hover: hover) { .company-row:hover { background: var(--fill-1); } }
  .company-row:focus-visible { outline: var(--focus-ring); outline-offset: -2px; }
  .tile {
    display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0;
    width: 36px; height: 36px; border-radius: 9px; color: #fff;
    font-size: 11px; font-weight: 700; letter-spacing: 0.02em; font-family: var(--font-mono);
  }
  .mission { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { font-size: var(--text-sm); color: var(--text-dim); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .live { width: 6px; height: 6px; border-radius: 50%; background: currentColor; animation: live 1.6s ease-in-out infinite; }
  @keyframes live { 50% { opacity: 0.3; } }
  @media (prefers-reduced-motion: reduce) { .live { animation: none; } }

  .create-card { width: min(640px, 100%); max-height: calc(100vh - 64px); overflow-y: auto; }
  .gallery { display: grid; grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); gap: var(--space-2); }
  .tpl {
    display: flex; flex-direction: column; gap: 4px; padding: var(--space-3); text-align: left;
    border: 0; border-radius: var(--radius-md); background: var(--fill-1); color: inherit; font: inherit; cursor: pointer;
    box-shadow: inset 0 0 0 var(--hairline) var(--separator);
  }
  @media (hover: hover) { .tpl:hover { background: var(--fill-2); } }
  .tpl[aria-checked="true"] { box-shadow: inset 0 0 0 2px var(--accent); background: var(--accent-soft); }
  .tpl:focus-visible { outline: var(--focus-ring); }
  .tpl-name { font-weight: 600; font-size: var(--text-sm); }
  .tpl-summary { font-size: var(--text-xs); color: var(--text-muted); line-height: 1.35; }
  .tpl-roles { font-size: var(--text-xs); color: var(--text-dim); }
  textarea.input { resize: vertical; height: auto; font-family: inherit; }
  .folder { display: flex; align-items: center; gap: var(--space-2); min-width: 0; }
  .path { font-family: var(--font-mono); font-size: var(--text-xs); color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .hint { margin: 0; font-size: var(--text-sm); color: var(--text-dim); }
</style>
