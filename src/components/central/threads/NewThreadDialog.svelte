<script lang="ts">
  // New thread: project, driver instance, model, access mode, worktree yes/no
  // and an optional first message. A worktree is created before the thread
  // (setup stages stream in via `central://vcs/worktree-setup`); cancelling
  // the setup removes it.
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { threadsStore } from "$lib/central/threads/threads-store.svelte";
  import type { AccessMode } from "$lib/central/threads/types";
  import { onWorktreeSetup, vcsStatus, vcsWorktreeCancel, vcsError, type SetupSnapshot } from "$lib/central/vcs";
  import { createWorktree, ensureCheckpoint } from "$lib/central/threads-ui/api";
  import { ACCESS_MODES, MODEL_HINTS, accessIcon, driverName, driverTint } from "$lib/central/threads-ui/drivers";
  import Modal from "./Modal.svelte";
  import Icon from "./Icon.svelte";

  let {
    open,
    projectId = null,
    onclose,
    oncreated,
    onnewproject,
  }: {
    open: boolean;
    projectId?: string | null;
    onclose: () => void;
    oncreated: (threadId: string, firstMessage: string) => void;
    onnewproject: () => void;
  } = $props();

  const LAST_KEY = "omniget.central.threads.new.last";
  type Last = { projectId?: string; instanceId?: string; model?: string; access?: AccessMode; worktree?: boolean };
  function readLast(): Last {
    try {
      return JSON.parse(localStorage.getItem(LAST_KEY) ?? "{}") as Last;
    } catch {
      return {};
    }
  }

  let project = $state("");
  let instanceId = $state("");
  let model = $state("");
  let access = $state<AccessMode>("approval-required");
  let worktree = $state(false);
  let title = $state("");
  let message = $state("");
  let isRepo = $state<boolean | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let setup = $state<SetupSnapshot | null>(null);
  let pendingThread: string | null = null;
  const modelList = $props.id();

  $effect(() => {
    if (!open) return;
    untrack(() => {
      const last = readLast();
      project = projectId ?? last.projectId ?? threadsStore.projects[0]?.projectId ?? "";
      if (!threadsStore.projects.some((p) => p.projectId === project)) project = threadsStore.projects[0]?.projectId ?? "";
      const avail = threadsStore.instances.filter((i) => i.available);
      instanceId = avail.some((i) => i.instance.id === last.instanceId) ? (last.instanceId ?? "") : (avail[0]?.instance.id ?? threadsStore.instances[0]?.instance.id ?? "");
      model = last.model ?? "";
      access = last.access ?? "approval-required";
      worktree = last.worktree ?? false;
      title = "";
      message = "";
      error = null;
      setup = null;
      busy = false;
      if (!threadsStore.instances.length) void threadsStore.refreshDrivers();
    });
  });

  let projectRow = $derived(threadsStore.projects.find((p) => p.projectId === project) ?? null);
  let inst = $derived(threadsStore.instances.find((i) => i.instance.id === instanceId) ?? null);
  let hints = $derived(MODEL_HINTS[inst?.instance.driver ?? ""] ?? []);

  $effect(() => {
    const root = projectRow?.workspaceRoot;
    isRepo = null;
    if (!root || !open) return;
    let alive = true;
    vcsStatus(root)
      .then((s) => {
        if (alive) isRepo = s.is_repo;
      })
      .catch(() => {
        if (alive) isRepo = false;
      });
    return () => {
      alive = false;
    };
  });

  async function create() {
    if (!projectRow || !inst) return;
    busy = true;
    error = null;
    const threadId = `thr_${crypto.randomUUID().replaceAll("-", "").slice(0, 16)}`;
    pendingThread = threadId;
    let worktreePath: string | undefined;
    let branch: string | undefined;
    let unlisten: (() => void) | null = null;
    try {
      if (worktree && isRepo) {
        unlisten = await onWorktreeSetup((s) => (setup = s), threadId);
        const info = await createWorktree(projectRow.workspaceRoot, threadId);
        worktreePath = info.path;
        branch = info.branch;
      }
      await threadsStore.dispatch({
        type: "thread.create",
        threadId,
        projectId: projectRow.projectId,
        title: title.trim() || undefined,
        instanceId: inst.instance.id,
        driver: inst.instance.driver,
        model: model.trim() || undefined,
        runtimeMode: access,
        worktreePath,
        branch,
      });
      void ensureCheckpoint(worktreePath ?? projectRow.workspaceRoot, threadId, 0);
      try {
        localStorage.setItem(LAST_KEY, JSON.stringify({ projectId: project, instanceId, model, access, worktree }));
      } catch {
        /* optional */
      }
      pendingThread = null;
      oncreated(threadId, message.trim());
    } catch (e) {
      const { code, message: msg } = vcsError(e);
      error = code === "ERR_VCS_CANCELLED" ? ($t("llm.central.threads.new.cancelled") as string) : msg;
    } finally {
      unlisten?.();
      busy = false;
    }
  }

  async function cancel() {
    if (busy && pendingThread && worktree) {
      await vcsWorktreeCancel(pendingThread).catch(() => false);
      return;
    }
    onclose();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void create();
    }
  }
</script>

<Modal {open} title={$t("llm.central.threads.new.title")} onclose={cancel} width={560}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="form" onkeydown={onKey}>
    <label class="field">
      <span class="lbl">{$t("llm.central.threads.new.project")}</span>
      <div class="row">
        <select class="input" bind:value={project} disabled={busy}>
          {#each threadsStore.projects as p (p.projectId)}<option value={p.projectId}>{p.title}</option>{/each}
        </select>
        <button type="button" class="button" onclick={onnewproject} disabled={busy}><Icon name="folder-simple" size={14} />{$t("llm.central.threads.new_project")}</button>
      </div>
      {#if projectRow}<span class="hint mono">{projectRow.workspaceRoot}</span>{/if}
    </label>

    <fieldset class="field">
      <legend class="lbl">{$t("llm.central.threads.new.driver")}</legend>
      <div class="instances" role="radiogroup">
        {#each threadsStore.instances as i (i.instance.id)}
          <label class="inst" class:off={!i.available} title={i.unavailableReason ?? ""}>
            <input type="radio" name="instance" value={i.instance.id} bind:group={instanceId} disabled={!i.available || busy} />
            <span class="dot" style:--tint={driverTint(i.instance.driver, i)}></span>
            <span class="inst-name">{driverName(i.instance.driver, i)}</span>
            {#if i.instance.accountId}<span class="acct">{i.instance.accountId}</span>{/if}
            {#if !i.available}<span class="why">{i.unavailableReason ?? $t("llm.central.threads.new.unavailable")}</span>{/if}
          </label>
        {:else}
          <p class="hint">{$t("llm.central.threads.new.no_drivers")}</p>
        {/each}
      </div>
    </fieldset>

    <div class="two">
      <label class="field">
        <span class="lbl">{$t("llm.central.threads.new.model")}</span>
        <input class="input" list={modelList} bind:value={model} placeholder={$t("llm.central.threads.new.model_default")} disabled={busy} />
        <datalist id={modelList}>{#each hints as h (h)}<option value={h}></option>{/each}</datalist>
      </label>
      <label class="field">
        <span class="lbl">{$t("llm.central.threads.new.title_field")}</span>
        <input class="input" bind:value={title} placeholder={$t("llm.central.threads.new.title_auto")} disabled={busy} />
      </label>
    </div>

    <fieldset class="field">
      <legend class="lbl">{$t("llm.central.threads.access.label")}</legend>
      <div class="seg" role="radiogroup">
        {#each ACCESS_MODES as m (m)}
          <label class="seg-item" class:on={access === m} title={$t(`llm.central.threads.access_hint.${m}`)}>
            <input type="radio" name="access" value={m} bind:group={access} disabled={busy} />
            <Icon name={accessIcon(m)} size={13} />{$t(`llm.central.threads.access.${m}`)}
          </label>
        {/each}
      </div>
    </fieldset>

    <label class="check" class:off={isRepo === false}>
      <input type="checkbox" bind:checked={worktree} disabled={busy || isRepo === false} />
      <Icon name="git-branch" size={14} />
      <span>
        {$t("llm.central.threads.new.worktree")}
        <span class="hint">{isRepo === false ? $t("llm.central.threads.new.worktree_not_repo") : $t("llm.central.threads.new.worktree_hint")}</span>
      </span>
    </label>

    <label class="field">
      <span class="lbl">{$t("llm.central.threads.new.first_message")}</span>
      <textarea class="input" rows="3" bind:value={message} placeholder={$t("llm.central.threads.new.first_message_ph")} disabled={busy}></textarea>
    </label>

    {#if setup}
      <ol class="stages" aria-live="polite">
        {#each setup.stages as s (s.id)}
          <li data-state={s.state}>
            <span class="st">{s.state === "done" ? "✓" : s.state === "failed" ? "✕" : s.state === "running" ? "…" : s.state === "skipped" ? "–" : "○"}</span>
            {$t(`llm.central.threads.setup.${s.id}`)}{#if s.percent != null}&nbsp;{s.percent}%{/if}
            {#if s.tail.length}<pre class="tail">{s.tail.join("\n")}</pre>{/if}
          </li>
        {/each}
      </ol>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </div>

  {#snippet footer()}
    <button type="button" class="button" onclick={cancel}>{busy && worktree ? $t("llm.central.threads.new.cancel_setup") : $t("llm.central.threads.cancel")}</button>
    <button type="button" class="button primary" disabled={busy || !projectRow || !inst?.available} onclick={create}>
      {busy ? $t("llm.central.threads.new.creating") : $t("llm.central.threads.new.create")}
    </button>
  {/snippet}
</Modal>

<style>
  .form { display: grid; gap: 14px; }
  .field { display: grid; gap: 6px; border: 0; padding: 0; margin: 0; min-width: 0; }
  .lbl { font-size: 12px; font-weight: 600; color: var(--text-muted); padding: 0; }
  .row { display: flex; gap: 8px; }
  .row select { flex: 1; min-width: 0; }
  .button { display: inline-flex; align-items: center; gap: 6px; }
  .hint { font-size: 11.5px; color: var(--text-muted); }
  .mono { font-family: var(--font-mono); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  @media (max-width: 520px) { .two { grid-template-columns: 1fr; } }
  .instances { display: grid; gap: 4px; max-height: 180px; overflow: auto; }
  .inst { display: flex; align-items: center; gap: 8px; padding: 7px 10px; border-radius: 9px; border: 1px solid var(--separator); cursor: pointer; font-size: 13px; }
  .inst:has(input:checked) { border-color: var(--accent); background: var(--accent-soft); }
  .inst:has(input:focus-visible) { outline: 2px solid var(--accent); outline-offset: 1px; }
  .inst input { position: absolute; opacity: 0; pointer-events: none; }
  .inst.off { opacity: 0.55; cursor: not-allowed; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--tint); flex-shrink: 0; }
  .inst-name { font-weight: 600; }
  .acct { font-size: 11px; color: var(--text-muted); font-family: var(--font-mono); }
  .why { margin-left: auto; font-size: 11px; color: var(--text-muted); }
  .seg { display: flex; flex-wrap: wrap; gap: 4px; padding: 3px; border-radius: 10px; background: var(--fill-1); }
  .seg-item { flex: 1 1 auto; display: inline-flex; align-items: center; justify-content: center; gap: 5px; padding: 6px 8px; border-radius: 8px; font-size: 12px; cursor: pointer; color: var(--text-muted); white-space: nowrap; }
  .seg-item input { position: absolute; opacity: 0; pointer-events: none; }
  .seg-item.on { background: var(--surface, #fff); color: var(--text); box-shadow: 0 1px 3px color-mix(in srgb, #000 12%, transparent); font-weight: 600; }
  .seg-item:has(input:focus-visible) { outline: 2px solid var(--accent); }
  .check { display: flex; gap: 8px; align-items: flex-start; font-size: 13px; cursor: pointer; }
  .check span { display: grid; gap: 2px; }
  .check.off { opacity: 0.6; }
  textarea { resize: vertical; min-height: 64px; font: inherit; }
  .stages { list-style: none; margin: 0; padding: 10px 12px; border-radius: 10px; background: var(--fill-1); display: grid; gap: 4px; font-size: 12.5px; }
  .stages li[data-state="failed"] { color: var(--red, #ef4444); }
  .stages li[data-state="running"] { color: var(--blue, var(--accent)); }
  .st { display: inline-block; width: 16px; }
  .tail { margin: 4px 0 0 16px; height: 4.6em; overflow: hidden; font-size: 11px; color: var(--text-muted); white-space: pre; }
  .error { color: var(--red, #ef4444); font-size: 12.5px; margin: 0; }
</style>
