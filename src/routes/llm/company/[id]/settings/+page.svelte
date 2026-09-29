<script lang="ts">
  /**
   * Company settings: identity (name, mission, description, folder), the
   * owner's governance switches, parallelism, pause/archive, export as a
   * package and delete (typing the name to confirm).
   */
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { errorText, rpc, type Company } from "$lib/company/rpc";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { pickFolder } from "$lib/stores/llm-jobs-store.svelte";
  import { downloadText } from "$components/company/company-ui";

  let id = $derived(page.params.id ?? "");
  let company = $state<Company | null>(null);
  let error = $state("");

  let name = $state("");
  let mission = $state("");
  let description = $state("");
  let workspace = $state<string | null>(null);
  let requireHire = $state(false);
  let requireStrategy = $state(false);
  let maxParallel = $state(4);
  let saving = $state(false);

  let includeIssues = $state(false);
  let exporting = $state(false);
  let deleting = $state(false);
  let confirmName = $state("");

  function fill(c: Company) {
    company = c;
    name = c.name;
    mission = c.mission;
    description = c.description;
    workspace = c.workspace;
    requireHire = c.require_hire_approval;
    requireStrategy = c.require_strategy_approval;
    maxParallel = c.max_parallel;
  }

  $effect(() => {
    const cid = id;
    if (!cid) return;
    rpc<Company>("company.get", { company: cid })
      .then(fill)
      .catch((e) => (error = errorText(e)));
  });

  let dirty = $derived(
    !!company &&
      (name.trim() !== company.name ||
        mission !== company.mission ||
        description !== company.description ||
        workspace !== company.workspace ||
        requireHire !== company.require_hire_approval ||
        requireStrategy !== company.require_strategy_approval ||
        maxParallel !== company.max_parallel),
  );

  async function save(e: Event) {
    e.preventDefault();
    if (!company || !name.trim()) return;
    saving = true;
    try {
      const patch: Record<string, unknown> = {
        name: name.trim(),
        mission,
        description,
        require_hire_approval: requireHire,
        require_strategy_approval: requireStrategy,
        max_parallel: Math.min(64, Math.max(1, Math.floor(maxParallel || 1))),
      };
      if (workspace !== company.workspace) patch.workspace = workspace;
      fill(await rpc<Company>("company.update", { company: company.id, patch }));
      showToast("success", $t("company.common.saved"));
    } catch (err) {
      showToast("error", errorText(err));
    } finally {
      saving = false;
    }
  }

  async function setStatus(status: "active" | "paused" | "archived") {
    if (!company) return;
    if (status === "archived" && !confirm($t("company.settings.archive_confirm", { name: company.name }) as string)) return;
    try {
      fill(await rpc<Company>("company.status", { company: company.id, status }));
    } catch (e) {
      showToast("error", errorText(e));
    }
  }

  async function exportPackage() {
    if (!company) return;
    exporting = true;
    try {
      const pkg = await rpc<unknown>("company.export", { company: company.id, include_issues: includeIssues });
      const slug = company.name.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, "-").replace(/^-|-$/g, "") || "company";
      downloadText(`${slug}.omniget-company.json`, JSON.stringify(pkg, null, 2));
      showToast("success", $t("company.settings.exported"));
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      exporting = false;
    }
  }

  async function remove() {
    if (!company || confirmName.trim() !== company.name) return;
    deleting = true;
    try {
      await rpc("company.delete", { company: company.id });
      await goto("/llm/company");
    } catch (e) {
      showToast("error", errorText(e));
      deleting = false;
    }
  }
</script>

<div class="page settings-page">
  {#if error}<p class="field-error">{error}</p>{/if}
  {#if company}
    <form onsubmit={save}>
      <h2 class="group-label">{$t("company.settings.identity")}</h2>
      <div class="group pad">
        <label class="field">
          <span class="field-label">{$t("company.settings.name")}</span>
          <input class="input" type="text" bind:value={name} required />
        </label>
        <label class="field">
          <span class="field-label">{$t("company.settings.mission")}</span>
          <textarea class="input" rows="2" bind:value={mission}></textarea>
        </label>
        <label class="field">
          <span class="field-label">{$t("company.settings.description")}</span>
          <textarea class="input" rows="3" bind:value={description}></textarea>
        </label>
        <div class="field">
          <span class="field-label">{$t("company.settings.workspace")}</span>
          <div class="folder">
            <button type="button" class="button" onclick={async () => (workspace = (await pickFolder()) ?? workspace)}>{$t("company.common.pick_folder")}</button>
            {#if workspace}<code class="path" title={workspace}>{workspace}</code>{:else}<span class="hint">{$t("company.common.no_folder")}</span>{/if}
          </div>
          <span class="field-hint">{$t("company.settings.workspace_hint")}</span>
        </div>
        <div class="field-hint">{$t("company.settings.prefix", { prefix: company.issue_prefix })}</div>
      </div>

      <h2 class="group-label">{$t("company.settings.governance")}</h2>
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label" id="req-hire">{$t("company.settings.require_hire")}</span>
            <span class="setting-path wrap">{$t("company.settings.require_hire_hint")}</span>
          </div>
          <button type="button" class="toggle" role="switch" aria-checked={requireHire} aria-labelledby="req-hire" onclick={() => (requireHire = !requireHire)}><span class="toggle-knob"></span></button>
        </div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label" id="req-strategy">{$t("company.settings.require_strategy")}</span>
            <span class="setting-path wrap">
              {$t("company.settings.require_strategy_hint")}
              {#if company.strategy_state !== "none"} · {$t(`company.settings.strategy_state.${company.strategy_state}`)}{/if}
            </span>
          </div>
          <button type="button" class="toggle" role="switch" aria-checked={requireStrategy} aria-labelledby="req-strategy" onclick={() => (requireStrategy = !requireStrategy)}><span class="toggle-knob"></span></button>
        </div>
        <div class="setting-row">
          <div class="setting-col">
            <label class="setting-label" for="max-parallel">{$t("company.settings.max_parallel")}</label>
            <span class="setting-path wrap">{$t("company.settings.max_parallel_hint")}</span>
          </div>
          <input id="max-parallel" class="input number" type="number" min="1" max="64" bind:value={maxParallel} />
        </div>
      </div>

      <div class="save-bar">
        <button type="submit" class="button active" disabled={saving || !dirty || !name.trim()}>{$t("company.common.save")}</button>
      </div>
    </form>

    <h2 class="group-label">{$t("company.settings.state")}</h2>
    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t(`company.common.company_status.${company.status}`)}</span>
          <span class="setting-path wrap">{$t(`company.settings.state_hint.${company.status}`)}</span>
        </div>
        <div class="row-actions">
          {#if company.status === "active"}
            <button type="button" class="button" onclick={() => setStatus("paused")}>{$t("company.shell.pause")}</button>
          {:else}
            <button type="button" class="button" onclick={() => setStatus("active")}>{company.status === "archived" ? $t("company.settings.unarchive") : $t("company.shell.resume")}</button>
          {/if}
          {#if company.status !== "archived"}
            <button type="button" class="button" onclick={() => setStatus("archived")}>{$t("company.settings.archive")}</button>
          {/if}
        </div>
      </div>
    </div>

    <h2 class="group-label">{$t("company.settings.export")}</h2>
    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t("company.settings.export_label")}</span>
          <span class="setting-path wrap">{$t("company.settings.export_hint")}</span>
        </div>
        <button type="button" class="button" disabled={exporting} onclick={exportPackage}>{$t("company.settings.export_button")}</button>
      </div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label" id="inc-issues">{$t("company.settings.include_issues")}</span>
        </div>
        <button type="button" class="toggle" role="switch" aria-checked={includeIssues} aria-labelledby="inc-issues" onclick={() => (includeIssues = !includeIssues)}><span class="toggle-knob"></span></button>
      </div>
    </div>

    <h2 class="group-label danger-label">{$t("company.settings.danger")}</h2>
    <div class="group pad danger-zone">
      <p class="hint">{$t("company.settings.delete_hint")}</p>
      <label class="field">
        <span class="field-label">{$t("company.settings.delete_type", { name: company.name })}</span>
        <input class="input" type="text" bind:value={confirmName} autocomplete="off" spellcheck="false" />
      </label>
      <div>
        <button type="button" class="button danger" disabled={deleting || confirmName.trim() !== company.name} onclick={remove}>{$t("company.settings.delete_button")}</button>
      </div>
    </div>
  {:else if !error}
    <div class="skeleton" style="height: 240px"></div>
  {/if}
</div>

<style>
  .settings-page { display: flex; flex-direction: column; }
  .settings-page form { display: flex; flex-direction: column; }
  .group-label { margin-top: var(--space-5); }
  .settings-page > .group-label:first-child, form > .group-label:first-child { margin-top: 0; }
  .pad { padding: var(--space-3) var(--space-4); display: flex; flex-direction: column; gap: var(--space-3); }
  textarea.input { resize: vertical; height: auto; font-family: inherit; }
  .folder { display: flex; align-items: center; gap: var(--space-2); min-width: 0; }
  .path { font-family: var(--font-mono); font-size: var(--text-xs); color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .hint { margin: 0; font-size: var(--text-sm); color: var(--text-dim); }
  .wrap { white-space: normal; }
  .number { width: 80px; font-variant-numeric: tabular-nums; }
  .save-bar { display: flex; justify-content: flex-end; margin-top: var(--space-3); }
  .row-actions { display: flex; gap: var(--space-2); flex-shrink: 0; }
  .danger-label { color: var(--red); }
  .danger-zone { box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--red) 40%, transparent); }
  .button.danger:not(:disabled) { color: var(--red); }
</style>
