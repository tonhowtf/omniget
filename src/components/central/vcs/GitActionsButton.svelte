<script lang="ts">
  // Botão dividido de git: a parte principal faz "o próximo passo sensato"
  // (commit, commit & push, commit/push & PR, push, abrir PR, ver PR) e a
  // seta abre o menu com cada ação. A mensagem vem sugerida do diff; o
  // diálogo deixa editar antes de rodar.
  import { onMount } from "svelte";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    vcsCommit,
    vcsCommitSuggest,
    vcsError,
    vcsHost,
    vcsLog,
    vcsPrCreate,
    vcsPrView,
    vcsPush,
    vcsStatus,
    type HostInfo,
    type PrInfo,
    type RepoStatus,
  } from "$lib/central/vcs";

  type Action = "commit" | "commit_push" | "commit_push_pr" | "push" | "push_pr" | "create_pr" | "view_pr" | "none";

  let {
    path,
    compact = false,
    onchanged,
    onpr,
  }: {
    /** Pasta do repositório/worktree da thread. */
    path: string;
    /** Só o ícone na parte principal (cabeçalho estreito). */
    compact?: boolean;
    onchanged?: () => void;
    onpr?: (pr: PrInfo) => void;
  } = $props();

  let status = $state<RepoStatus | null>(null);
  let host = $state<HostInfo | null>(null);
  let pr = $state<PrInfo | null>(null);
  let busy = $state<string | null>(null);
  let menuOpen = $state(false);
  let dialog = $state<{ action: Action; message: string; draft: boolean } | null>(null);
  let root: HTMLDivElement | undefined = $state();

  let refreshing = false;
  async function refresh() {
    if (!path || refreshing) return;
    refreshing = true;
    try {
      status = await vcsStatus(path);
      if (status.is_repo && status.remote) {
        host = await vcsHost(path).catch(() => null);
        pr = host?.available && status.branch ? await vcsPrView(path).catch(() => null) : null;
      } else {
        host = null;
        pr = null;
      }
    } catch (e) {
      status = null;
      showToast("error", vcsError(e).message);
    } finally {
      refreshing = false;
    }
  }

  onMount(() => {
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  });

  $effect(() => {
    void path;
    void refresh();
  });

  const term = $derived(host?.term ?? "PR");
  const hostOk = $derived(!!host?.available);
  const prOpen = $derived(pr?.state === "open");

  const quick = $derived.by((): { action: Action; hint?: string } => {
    const s = status;
    if (!s) return { action: "none", hint: $t("llm.central.vcs.git.loading") as string };
    if (!s.is_repo) return { action: "none", hint: $t("llm.central.vcs.git.not_repo") as string };
    if (s.detached) return { action: "none", hint: $t("llm.central.vcs.git.detached") as string };
    if (s.conflicts) return { action: "none", hint: $t("llm.central.vcs.git.conflicts") as string };
    if (s.dirty) {
      if (!s.remote) return { action: "commit" };
      if (prOpen || !hostOk) return { action: "commit_push" };
      return { action: "commit_push_pr" };
    }
    if (s.remote && (!s.upstream || s.ahead > 0)) {
      if (prOpen || !hostOk) return { action: "push" };
      return { action: "push_pr" };
    }
    if (pr) return { action: "view_pr" };
    if (hostOk && s.upstream && s.branch && s.base_branch && s.branch !== s.base_branch) return { action: "create_pr" };
    return { action: "none", hint: $t("llm.central.vcs.git.up_to_date") as string };
  });

  function label(a: Action): string {
    return $t(`llm.central.vcs.git.action.${a}`, { term }) as string;
  }

  const icon: Record<Action, string> = {
    commit: "✓",
    commit_push: "↑",
    commit_push_pr: "⇪",
    push: "↑",
    push_pr: "⇪",
    create_pr: "⇪",
    view_pr: "↗",
    none: "✓",
  };

  async function prepare(action: Action) {
    menuOpen = false;
    if (action === "none") return;
    if (action === "view_pr") {
      if (pr?.url) await openExternal(pr.url);
      return;
    }
    if (action === "push") {
      await run(action, "", false);
      return;
    }
    busy = $t("llm.central.vcs.git.preparing") as string;
    try {
      let message = "";
      if (action.startsWith("commit")) {
        const s = await vcsCommitSuggest(path);
        message = s.body ? `${s.subject}\n\n${s.body}` : s.subject;
      } else {
        const [last] = await vcsLog(path, 1);
        message = last?.subject ?? status?.branch ?? "";
      }
      dialog = { action, message, draft: true };
    } catch (e) {
      showToast("error", vcsError(e).message);
    } finally {
      busy = null;
    }
  }

  async function run(action: Action, message: string, draft: boolean) {
    dialog = null;
    const doCommit = action.startsWith("commit");
    const doPush = action.includes("push");
    const doPr = action.endsWith("_pr") || action === "create_pr";
    try {
      let subject = message.split("\n")[0].trim();
      let body = message.split("\n").slice(1).join("\n").trim();
      if (doCommit) {
        busy = $t("llm.central.vcs.git.phase.commit") as string;
        const c = await vcsCommit(path, message);
        if (c.status === "skipped_no_changes") showToast("info", $t("llm.central.vcs.git.nothing_to_commit") as string);
        else {
          subject = c.subject;
          showToast("success", $t("llm.central.vcs.git.committed", { sha: (c.sha ?? "").slice(0, 7) }) as string);
        }
      }
      if (doPush) {
        busy = $t("llm.central.vcs.git.phase.push") as string;
        const p = await vcsPush(path);
        showToast(
          p.status === "pushed" ? "success" : "info",
          (p.status === "pushed"
            ? $t("llm.central.vcs.git.pushed", { upstream: p.upstream })
            : $t("llm.central.vcs.git.up_to_date")) as string,
        );
      }
      if (doPr) {
        busy = $t("llm.central.vcs.git.phase.pr", { term }) as string;
        const r = await vcsPrCreate(path, { title: subject || (status?.branch ?? "update"), body, draft });
        pr = r.pr;
        onpr?.(r.pr);
        showToast(
          "success",
          (r.status === "created"
            ? $t("llm.central.vcs.git.pr_created", { term, number: r.pr.number })
            : $t("llm.central.vcs.git.pr_exists", { term, number: r.pr.number })) as string,
        );
      }
    } catch (e) {
      showToast("error", vcsError(e).message);
    } finally {
      busy = null;
      onchanged?.();
      await refresh();
    }
  }

  function onDialogKey(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && dialog) {
      e.preventDefault();
      void run(dialog.action, dialog.message, dialog.draft);
    } else if (e.key === "Escape") {
      dialog = null;
    }
  }

  function onDocClick(e: MouseEvent) {
    if (menuOpen && root && !root.contains(e.target as Node)) menuOpen = false;
  }

  const menuItems = $derived.by((): { action: Action; enabled: boolean }[] => {
    const s = status;
    const repo = !!s?.is_repo && !s.detached;
    return [
      { action: "commit", enabled: repo && !!s?.dirty && !s?.conflicts },
      { action: "commit_push", enabled: repo && !!s?.dirty && !!s?.remote },
      { action: "push", enabled: repo && !!s?.remote && (!s?.upstream || (s?.ahead ?? 0) > 0) },
      { action: "create_pr", enabled: repo && hostOk && !!s?.upstream && !prOpen && !s?.dirty },
      { action: "view_pr", enabled: !!pr?.url },
    ];
  });
</script>

<svelte:window onclick={onDocClick} />

<div class="gab" bind:this={root}>
  <div class="split" class:busy={!!busy}>
    <button
      class="main"
      disabled={!!busy || quick.action === "none"}
      title={busy ?? quick.hint ?? label(quick.action)}
      onclick={() => prepare(quick.action)}
    >
      {#if busy}<span class="spin" aria-hidden="true"></span>{:else}<span class="ic">{icon[quick.action]}</span>{/if}
      {#if !compact}<span class="lbl">{busy ?? (quick.action === "none" ? quick.hint : label(quick.action))}</span>{/if}
    </button>
    <button
      class="chev"
      aria-label={$t("llm.central.vcs.git.options")}
      aria-expanded={menuOpen}
      disabled={!!busy}
      onclick={(e) => {
        e.stopPropagation();
        menuOpen = !menuOpen;
        if (menuOpen) void refresh();
      }}>▾</button
    >
  </div>

  {#if pr}
    <button class="pr-badge pr-{pr.state}" class:draft={pr.draft} title={pr.title} onclick={() => openExternal(pr!.url)}>
      {term} #{pr.number} · {$t(`llm.central.vcs.git.pr_state.${pr.draft && pr.state === "open" ? "draft" : pr.state}`)}
    </button>
  {/if}

  {#if menuOpen}
    <div class="menu" role="menu">
      {#if status?.is_repo}
        <div class="menu-head">
          <span class="branch">{status.branch ?? "HEAD"}</span>
          {#if status.ahead}<span class="pill">↑{status.ahead}</span>{/if}
          {#if status.behind}<span class="pill">↓{status.behind}</span>{/if}
          {#if status.dirty}<span class="pill">{$t("llm.central.vcs.git.dirty", { count: status.files.length })}</span>{/if}
        </div>
      {/if}
      {#each menuItems as it (it.action)}
        <button role="menuitem" disabled={!it.enabled} onclick={() => prepare(it.action)}>
          <span class="ic">{icon[it.action]}</span>{label(it.action)}
        </button>
      {/each}
      {#if host && !host.available && host.reason}
        <div class="menu-note">{$t(`llm.central.vcs.git.unavailable.${host.reason}`, { cli: host.cli ?? "", term })}</div>
      {/if}
      <button role="menuitem" onclick={() => { menuOpen = false; void refresh(); }}>
        <span class="ic">↻</span>{$t("llm.central.vcs.git.refresh")}
      </button>
    </div>
  {/if}

  {#if dialog}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="backdrop" onclick={() => (dialog = null)}>
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="dlg" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={onDialogKey}>
        <h3>{label(dialog.action)}</h3>
        <label class="fld">
          <span>{dialog.action.startsWith("commit") ? $t("llm.central.vcs.git.message") : $t("llm.central.vcs.git.pr_title_body", { term })}</span>
          <!-- svelte-ignore a11y_autofocus -->
          <textarea bind:value={dialog.message} rows="8" autofocus></textarea>
        </label>
        {#if dialog.action.endsWith("_pr") || dialog.action === "create_pr"}
          <label class="chk"><input type="checkbox" bind:checked={dialog.draft} />{$t("llm.central.vcs.git.draft", { term })}</label>
        {/if}
        <div class="dlg-actions">
          <span class="hint">{$t("llm.central.vcs.git.run_hint")}</span>
          <button class="ghost" onclick={() => (dialog = null)}>{$t("llm.central.vcs.cancel")}</button>
          <button
            class="primary"
            disabled={!dialog.message.trim()}
            onclick={() => dialog && run(dialog.action, dialog.message, dialog.draft)}>{label(dialog.action)}</button
          >
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .gab {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }
  .split {
    display: inline-flex;
    border-radius: 8px;
    overflow: hidden;
    border: 1px solid var(--button-stroke, var(--separator));
    background: var(--button, var(--surface-hi));
  }
  .split button {
    border: 0;
    background: transparent;
    color: var(--on-button, var(--text));
    cursor: pointer;
    height: 28px;
    font-size: 12px;
  }
  .split button:hover:not(:disabled) {
    background: var(--button-hover, var(--fill-2));
  }
  .split button:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .main {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    max-width: 260px;
  }
  .lbl {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chev {
    padding: 0 8px;
    border-left: 1px solid var(--button-stroke, var(--separator)) !important;
  }
  .ic {
    width: 14px;
    text-align: center;
  }
  .spin {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid var(--text-muted);
    border-top-color: transparent;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .pr-badge {
    border: 1px solid var(--separator);
    background: transparent;
    border-radius: 999px;
    padding: 2px 8px;
    font-size: 11px;
    cursor: pointer;
    color: var(--text-muted);
  }
  .pr-open {
    color: var(--green, #34c759);
    border-color: color-mix(in srgb, var(--green, #34c759) 40%, transparent);
  }
  .pr-open.draft {
    color: var(--text-muted);
    border-color: var(--separator);
  }
  .pr-merged {
    color: var(--purple, #8e44ad);
  }
  .pr-closed {
    color: var(--red, #ff3b30);
  }
  .menu {
    position: absolute;
    top: 34px;
    right: 0;
    z-index: 30;
    min-width: 240px;
    padding: 4px;
    background: var(--popup-bg, var(--surface-hi));
    border: 1px solid var(--separator);
    border-radius: 10px;
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.2);
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    border: 0;
    background: transparent;
    color: var(--text);
    padding: 6px 8px;
    border-radius: 6px;
    cursor: pointer;
    text-align: left;
    font-size: 12px;
  }
  .menu button:hover:not(:disabled) {
    background: var(--accent-soft);
  }
  .menu button:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .menu-head {
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 6px 8px 8px;
    border-bottom: 1px solid var(--separator);
    margin-bottom: 4px;
  }
  .branch {
    font-family: var(--font-mono, monospace);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .pill {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--fill-2, var(--surface-mut));
    color: var(--text-muted);
  }
  .menu-note {
    padding: 6px 8px;
    color: var(--text-muted);
    font-size: 11px;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    background: var(--dialog-backdrop, rgba(0, 0, 0, 0.35));
    display: grid;
    place-items: center;
  }
  .dlg {
    width: min(560px, calc(100vw - 32px));
    background: var(--popup-bg, var(--surface-hi));
    border: 1px solid var(--separator);
    border-radius: 14px;
    padding: 16px;
    box-shadow: 0 20px 50px rgba(0, 0, 0, 0.3);
    outline: none;
  }
  .dlg h3 {
    margin: 0 0 10px;
    font-size: 14px;
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 6px;
    color: var(--text-muted);
    font-size: 11px;
  }
  .fld textarea {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    background: var(--input-bg, var(--surface));
    color: var(--text);
    border: 1px solid var(--input-border, var(--separator));
    border-radius: 8px;
    padding: 8px 10px;
    font-family: var(--font-mono, monospace);
    font-size: 12px;
  }
  .chk {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-top: 10px;
    font-size: 12px;
  }
  .dlg-actions {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 14px;
  }
  .hint {
    flex: 1;
    color: var(--text-muted);
    font-size: 11px;
  }
  .ghost {
    background: transparent;
    border: 0;
    color: var(--text-muted);
    padding: 6px 10px;
    border-radius: 8px;
    cursor: pointer;
  }
  .primary {
    background: var(--accent);
    color: var(--on-accent, white);
    border: 0;
    border-radius: 8px;
    padding: 6px 12px;
    cursor: pointer;
  }
  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
