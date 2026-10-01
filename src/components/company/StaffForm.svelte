<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { errorText, rpc, type Adapter, type RuntimeTemplate, type Staff } from "$lib/company/rpc";
  import CronInput from "./CronInput.svelte";

  let {
    company,
    staff = null,
    all,
    templates,
    ondone,
    oncancel,
  }: {
    company: string;
    staff?: Staff | null;
    all: Staff[];
    templates: RuntimeTemplate[];
    ondone: (s: Staff) => void;
    oncancel: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  const src = staff;
  let name = $state(src?.name ?? "");
  let role = $state(src?.role ?? "");
  let title = $state(src?.title ?? "");
  let reportsTo = $state<string>(src?.reports_to ?? "");
  let capabilities = $state(src?.capabilities ?? "");
  let instructions = $state(src?.instructions ?? "");
  let cron = $state<string | null>(src?.heartbeat_cron ?? null);
  let maxConcurrent = $state<number>(src?.max_concurrent ?? 1);
  let canHire = $state(src?.can_hire ?? false);
  let runtime = $state<Adapter["kind"]>(src?.adapter.kind ?? "roster");
  let templateId = $state("");
  let command = $state(src?.adapter.kind === "process" ? src.adapter.command : "");
  let args = $state(src?.adapter.kind === "process" ? src.adapter.args.join("\n") : "");
  let url = $state(src?.adapter.kind === "http" ? src.adapter.url : "");
  let busy = $state(false);
  let error = $state("");

  $effect(() => {
    if (!templateId && templates.length) templateId = templates[0].id;
  });

  let managers = $derived.by(() => {
    if (!src) return all.filter((s) => s.status !== "terminated");
    const below = new Set<string>([src.id]);
    let grew = true;
    while (grew) {
      grew = false;
      for (const s of all) {
        if (s.reports_to && below.has(s.reports_to) && !below.has(s.id)) {
          below.add(s.id);
          grew = true;
        }
      }
    }
    return all.filter((s) => s.status !== "terminated" && !below.has(s.id));
  });

  let valid = $derived(
    !!name.trim() &&
      !!role.trim() &&
      (runtime === "roster" ? !!src || !!templateId : runtime === "process" ? !!command.trim() : /^https?:\/\//.test(url.trim())),
  );

  function adapter(): Adapter {
    if (runtime === "process") {
      return { kind: "process", command: command.trim(), args: args.split("\n").map((a) => a.trim()).filter(Boolean) };
    }
    if (runtime === "http") return { kind: "http", url: url.trim() };
    return { kind: "roster" };
  }

  function slug(s: string) {
    return s.toLowerCase().replace(/[^a-z0-9]+/g, "").slice(0, 20) || "agent";
  }

  async function submit(e: Event) {
    e.preventDefault();
    if (!valid || busy) return;
    busy = true;
    error = "";
    const fields = {
      name: name.trim(),
      role: role.trim(),
      title: title.trim(),
      reports_to: reportsTo || null,
      capabilities: capabilities.trim(),
      instructions: instructions.trim(),
      can_hire: canHire,
      heartbeat_cron: cron,
      max_concurrent: Math.min(20, Math.max(1, Math.floor(maxConcurrent || 1))),
    };
    try {
      let out: Staff;
      if (src) {
        const patch: Record<string, unknown> = {};
        for (const [k, v] of Object.entries(fields)) {
          if ((src as Record<string, unknown>)[k] !== v) patch[k] = v;
        }
        if (runtime !== "roster" || src.adapter.kind !== "roster") {
          const next = adapter();
          if (JSON.stringify(next) !== JSON.stringify({ ...src.adapter, timeout_sec: undefined })) patch.adapter = next;
        }
        out = await rpc<Staff>("staff.update", { company, staff: src.id, patch });
      } else if (runtime === "roster") {
        out = await invoke<Staff>("company_hire", { company, staff: { ...fields, agent_id: "" }, templateAgentId: templateId });
      } else {
        const agentId = `ext-${slug(name)}-${Math.random().toString(36).slice(2, 6)}`;
        out = await rpc<Staff>("staff.hire", { company, staff: { ...fields, agent_id: agentId, adapter: adapter() } });
      }
      ondone(out);
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<form class="staff-form" onsubmit={submit}>
  <div class="row">
    <label class="field grow">
      <span class="field-label">{$t("company.team.form.name")}</span>
      <input class="input" type="text" bind:value={name} required />
    </label>
    <label class="field grow">
      <span class="field-label">{$t("company.team.form.role")}</span>
      <input class="input" type="text" bind:value={role} placeholder={$t("company.team.form.role_placeholder")} required />
    </label>
  </div>
  <div class="row">
    <label class="field grow">
      <span class="field-label">{$t("company.team.form.title")}</span>
      <input class="input" type="text" bind:value={title} placeholder={$t("company.team.form.title_placeholder")} />
    </label>
    <label class="field grow">
      <span class="field-label">{$t("company.team.form.manager")}</span>
      <select class="input" bind:value={reportsTo}>
        <option value="">{$t("company.team.form.no_manager")}</option>
        {#each managers as m (m.id)}<option value={m.id}>{m.name} · {m.title || m.role}</option>{/each}
      </select>
    </label>
  </div>
  <label class="field">
    <span class="field-label">{$t("company.team.form.responsibilities")}</span>
    <textarea class="input" rows="2" bind:value={capabilities} placeholder={$t("company.team.form.responsibilities_placeholder")}></textarea>
  </label>
  <label class="field">
    <span class="field-label">{$t("company.team.form.instructions")}</span>
    <textarea class="input" rows="4" bind:value={instructions} placeholder={$t("company.team.form.instructions_placeholder")}></textarea>
  </label>

  <div class="field">
    <span class="field-label">{$t("company.team.form.heartbeat")}</span>
    <CronInput bind:value={cron} />
    <span class="field-hint">{$t("company.team.form.heartbeat_hint")}</span>
  </div>

  <div class="row">
    <label class="field small">
      <span class="field-label">{$t("company.team.form.max_concurrent")}</span>
      <input class="input" type="number" min="1" max="20" bind:value={maxConcurrent} />
    </label>
    <div class="switch-row grow">
      <button type="button" class="toggle" role="switch" aria-checked={canHire} aria-labelledby="can-hire-label" onclick={() => (canHire = !canHire)}><span class="toggle-knob"></span></button>
      <span class="switch-text">
        <span id="can-hire-label">{$t("company.team.form.can_hire")}</span>
        <span class="field-hint">{$t("company.team.form.can_hire_hint")}</span>
      </span>
    </div>
  </div>

  <fieldset class="runtime">
    <legend class="field-label">{$t("company.team.form.runtime")}</legend>
    <div class="segmented" role="radiogroup" aria-label={$t("company.team.form.runtime")}>
      {#each ["roster", "process", "http"] as const as k (k)}
        <button type="button" class="segmented-btn" role="radio" aria-checked={runtime === k} disabled={!!src && src.adapter.kind === "roster" && k !== "roster"} onclick={() => (runtime = k)}>
          {$t(`company.team.form.runtime_${k}`)}
        </button>
      {/each}
    </div>
    {#if runtime === "roster"}
      {#if src}
        <p class="field-hint">{$t("company.team.form.runtime_roster_fixed", { agent: src.agent_id })}</p>
      {:else if templates.length === 0}
        <p class="field-hint">{$t("company.team.form.no_templates")} <a href="/llm/roster">{$t("company.team.form.open_roster")}</a></p>
      {:else}
        <label class="field">
          <span class="field-label">{$t("company.team.form.template")}</span>
          <select class="input" bind:value={templateId}>
            {#each templates as tpl (tpl.id)}<option value={tpl.id}>{tpl.name}{tpl.model ? ` · ${tpl.model}` : ""}</option>{/each}
          </select>
          <span class="field-hint">{$t("company.team.form.template_hint")}</span>
        </label>
      {/if}
    {:else if runtime === "process"}
      <label class="field">
        <span class="field-label">{$t("company.team.form.command")}</span>
        <input class="input mono" type="text" bind:value={command} placeholder="/usr/local/bin/my-agent" spellcheck="false" />
      </label>
      <label class="field">
        <span class="field-label">{$t("company.team.form.args")}</span>
        <textarea class="input mono" rows="2" bind:value={args} spellcheck="false"></textarea>
        <span class="field-hint">{$t("company.team.form.args_hint")}</span>
      </label>
    {:else}
      <label class="field">
        <span class="field-label">{$t("company.team.form.url")}</span>
        <input class="input mono" type="url" bind:value={url} placeholder="http://127.0.0.1:8080/wake" spellcheck="false" />
      </label>
    {/if}
  </fieldset>

  {#if error}<p class="field-error">{error}</p>{/if}

  <div class="actions">
    <button type="button" class="button" disabled={busy} onclick={oncancel}>{$t("company.common.cancel")}</button>
    <button type="submit" class="button active" disabled={busy || !valid}>
      {src ? $t("company.common.save") : $t("company.team.hire")}
    </button>
  </div>
</form>

<style>
  .staff-form { display: flex; flex-direction: column; gap: var(--space-3); }
  .row { display: flex; flex-wrap: wrap; gap: var(--space-3); align-items: flex-end; }
  .grow { flex: 1; min-width: 200px; }
  .small { width: 140px; }
  textarea.input { resize: vertical; height: auto; font-family: inherit; }
  .mono, textarea.mono { font-family: var(--font-mono); }
  .switch-row { display: flex; align-items: center; gap: var(--space-2); min-height: 36px; }
  .switch-text { display: flex; flex-direction: column; font-size: var(--text-sm); }
  .runtime { border: 0; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-2); }
  .runtime .segmented { align-self: flex-start; }
  .actions { display: flex; justify-content: flex-end; gap: var(--space-2); }
</style>
