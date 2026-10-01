<script lang="ts">
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
    suggestCommitMessage,
    type ChangeSummary,
    type HostInfo,
    type BranchInfo,
    type PrInfo,
    type RepoStatus,
  } from "$lib/central/vcs";

  type GitOps = {
    run: (
      action: string,
      args?: Record<string, unknown>,
    ) => Promise<{ ok: boolean; output: string; conflicts?: string[]; stashes?: string[]; branches?: BranchInfo[] }>;
  };
  type Extra = "pull" | "fetch" | "sync" | "branches" | "stash" | "mergeAbort" | "removeLock";

  type Action = "commit" | "commit_push" | "commit_push_pr" | "push" | "push_pr" | "create_pr" | "view_pr" | "none";

  let {
    path,
    compact = false,
    onchanged,
    onpr,
    ops,
  }: {
    path: string;
    compact?: boolean;
    onchanged?: () => void;
    onpr?: (pr: PrInfo) => void;
    ops?: GitOps;
  } = $props();

  const tx = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(`llm.central.vcs.git.${key}`, { default: fallback, ...(vars ?? {}) }) as string;

  let status = $state<RepoStatus | null>(null);
  let host = $state<HostInfo | null>(null);
  let pr = $state<PrInfo | null>(null);
  let busy = $state<string | null>(null);
  let menuOpen = $state(false);
  let dialog = $state<{
    action: Action;
    message: string;
    draft: boolean;
    files: { path: string; kind: string; checked: boolean }[];
  } | null>(null);
  let suggested: { changes: ChangeSummary[]; text: string } | null = null;
  let root: HTMLDivElement | undefined = $state();
  let chevEl: HTMLButtonElement | undefined = $state();
  let menuPos = $state({ top: 0, right: 0 });
  let branchDlg = $state<{ list: BranchInfo[]; name: string } | null>(null);
  let stashDlg = $state<{ list: string[]; message: string } | null>(null);
  let failure = $state<{ title: string; text: string; conflicts: string[] } | null>(null);

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
        suggested = { changes: s.files ?? [], text: message };
      } else {
        const [last] = await vcsLog(path, 1);
        message = last?.subject ?? status?.branch ?? "";
      }
      const files = action.startsWith("commit")
        ? Array.from(new Map((status?.files ?? []).map((f) => [f.path, { path: f.path, kind: String(f.kind || "?"), checked: true }])).values())
        : [];
      dialog = { action, message, draft: true, files };
    } catch (e) {
      showToast("error", vcsError(e).message);
    } finally {
      busy = null;
    }
  }

  function setChecked(pick: (path: string) => boolean | null) {
    if (!dialog) return;
    dialog.files = dialog.files.map((f) => {
      const on = pick(f.path);
      return on == null ? f : { ...f, checked: on };
    });
    if (!suggested || dialog.message !== suggested.text) return;
    const by = new Map(suggested.changes.map((c) => [c.path, c]));
    const chosen = dialog.files
      .filter((f) => f.checked)
      .map((f) => by.get(f.path) ?? { path: f.path, status: "modified" as ChangeSummary["status"], additions: 0, deletions: 0 });
    if (!chosen.length) return;
    const text = suggestCommitMessage(chosen);
    suggested = { ...suggested, text };
    dialog.message = text;
  }

  async function run(action: Action, message: string, draft: boolean) {
    const picked = dialog?.files ?? [];
    const only = picked.length && picked.some((f) => !f.checked) ? picked.filter((f) => f.checked).map((f) => f.path) : null;
    dialog = null;
    const doCommit = action.startsWith("commit");
    const doPush = action.includes("push");
    const doPr = action.endsWith("_pr") || action === "create_pr";
    try {
      let subject = message.split("\n")[0].trim();
      let body = message.split("\n").slice(1).join("\n").trim();
      if (doCommit) {
        busy = $t("llm.central.vcs.git.phase.commit") as string;
        const c = await vcsCommit(path, message, only);
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
      fail(label(action), e);
    } finally {
      busy = null;
      onchanged?.();
      await refresh();
    }
  }

  function fail(title: string, e: unknown, conflicts: string[] = []) {
    const text = typeof e === "string" ? e : vcsError(e).message.replace(/^CANVAS_GIT:\s*/, "");
    failure = { title, text, conflicts };
  }

  async function extra(x: Extra | "checkout" | "merge" | "deleteBranch" | "newBranch" | "stashPush" | "stashApply" | "stashPop" | "stashDrop", args: Record<string, unknown> = {}) {
    menuOpen = false;
    if (!ops) return;
    if (x === "branches") {
      await openBranches();
      return;
    }
    if (x === "stash") {
      await openStash();
      return;
    }
    const act = x;
    busy = tx(`phase.${act}`, `${act}…`);
    try {
      const r = await ops.run(act, args);
      if (!r.ok && r.conflicts?.length) fail(tx(`action.${act}`, act), r.output, r.conflicts);
      else if (!r.ok && act !== "removeLock") fail(tx(`action.${act}`, act), r.output);
      else showToast(r.ok ? "success" : "info", r.output.split("\n").filter(Boolean).pop() || tx(`action.${act}`, act));
      if (r.stashes && stashDlg) stashDlg = { ...stashDlg, list: r.stashes };
    } catch (e) {
      fail(tx(`action.${act}`, act), e);
    } finally {
      busy = null;
      onchanged?.();
      await refresh();
    }
  }

  async function openBranches() {
    if (!ops) return;
    try {
      const r = await ops.run("branches");
      branchDlg = { list: r.branches ?? [], name: "" };
    } catch (e) {
      fail(tx("action.branches", "Branches…"), e);
    }
  }

  async function branchAct(act: "checkout" | "merge" | "deleteBranch" | "newBranch", branch: string, force = false) {
    if (!branch.trim()) return;
    await extra(act, { branch: branch.trim(), force });
    if (branchDlg) await openBranches();
  }

  async function openStash() {
    if (!ops) return;
    try {
      const r = await ops.run("stashList");
      stashDlg = { list: r.stashes ?? [], message: "" };
    } catch (e) {
      fail(tx("action.stash", "Stash…"), e);
    }
  }

  async function copyFailure() {
    if (failure) await navigator.clipboard?.writeText(failure.text).catch(() => {});
  }

  function onDialogKey(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && dialog) {
      e.preventDefault();
      e.stopPropagation();
      if (dialog.message.trim()) void run(dialog.action, dialog.message, dialog.draft);
    } else if (e.key === "Escape") {
      e.stopPropagation();
      dialog = null;
      branchDlg = null;
      stashDlg = null;
      failure = null;
    }
  }

  $effect(() => {
    if (!dialog && !branchDlg && !stashDlg && !failure) return;
    const h = (e: KeyboardEvent) => {
      if ((e.key === "Enter" && (e.metaKey || e.ctrlKey)) || e.key === "Escape") onDialogKey(e);
    };
    window.addEventListener("keydown", h, true);
    return () => window.removeEventListener("keydown", h, true);
  });

  function onDocClick(e: MouseEvent) {
    const t = e.target as Node;
    if (menuOpen && root && !root.contains(t) && !(t instanceof Element && t.closest(".gab-menu"))) menuOpen = false;
  }

  function placeMenu() {
    const r = chevEl?.getBoundingClientRect();
    if (r) menuPos = { top: r.bottom + 4, right: Math.max(8, window.innerWidth - r.right) };
  }

  function portal(el: HTMLElement) {
    document.body.appendChild(el);
    return { destroy: () => el.remove() };
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
      bind:this={chevEl}
      aria-label={$t("llm.central.vcs.git.options")}
      aria-expanded={menuOpen}
      disabled={!!busy}
      onclick={(e) => {
        e.stopPropagation();
        menuOpen = !menuOpen;
        if (menuOpen) {
          placeMenu();
          void refresh();
        }
      }}>▾</button
    >
  </div>

  {#if pr}
    <button class="pr-badge pr-{pr.state}" class:draft={pr.draft} title={pr.title} onclick={() => openExternal(pr!.url)}>
      {term} #{pr.number} · {$t(`llm.central.vcs.git.pr_state.${pr.draft && pr.state === "open" ? "draft" : pr.state}`)}
    </button>
  {/if}

  {#if menuOpen}
    <div class="menu gab-menu" role="menu" use:portal style:top="{menuPos.top}px" style:right="{menuPos.right}px">
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
      {#if ops && status?.is_repo}
        <div class="menu-sep"></div>
        <button role="menuitem" disabled={!status.remote} onclick={() => extra("pull")}><span class="ic">↓</span>{tx("action.pull", "Pull")}</button>
        <button role="menuitem" disabled={!status.remote} onclick={() => extra("fetch")}><span class="ic">⟳</span>{tx("action.fetch", "Fetch all")}</button>
        <button role="menuitem" disabled={!status.remote} onclick={() => extra("sync")}><span class="ic">⇅</span>{tx("action.sync", "Sync (pull + push)")}</button>
        <button role="menuitem" onclick={() => extra("branches")}><span class="ic">⎇</span>{tx("action.branches", "Branches…")}</button>
        <button role="menuitem" onclick={() => extra("stash")}><span class="ic">▤</span>{tx("action.stash", "Stash…")}</button>
        {#if status.conflicts}
          <button role="menuitem" onclick={() => extra("mergeAbort")}><span class="ic">✕</span>{tx("action.mergeAbort", "Abort merge")}</button>
        {/if}
        {#if status.index_locked}
          <button role="menuitem" onclick={() => extra("removeLock", { force: true })}><span class="ic">🔓</span>{tx("action.removeLock", "Remove stale git lock")}</button>
        {/if}
      {/if}
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
    <div class="backdrop" use:portal onclick={() => (dialog = null)}>
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="dlg" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={onDialogKey}>
        <h3>{label(dialog.action)}</h3>
        <label class="fld">
          <span>{dialog.action.startsWith("commit") ? $t("llm.central.vcs.git.message") : $t("llm.central.vcs.git.pr_title_body", { term })}</span>
          <!-- svelte-ignore a11y_autofocus -->
          <textarea bind:value={dialog.message} rows="8" autofocus></textarea>
        </label>
        {#if dialog.files.length > 1}
          <div class="files" role="group" aria-label={tx("files", "Files in this commit")}>
            <label class="chk all">
              <input
                type="checkbox"
                checked={dialog.files.every((f) => f.checked)}
                onchange={(e) => {
                  const on = (e.currentTarget as HTMLInputElement).checked;
                  setChecked(() => on);
                }}
              />{tx("files_count", "{{checked}} of {{total}} files", { checked: dialog.files.filter((f) => f.checked).length, total: dialog.files.length })}
            </label>
            {#each dialog.files as f (f.path)}
              <label class="chk file"
                ><input
                  type="checkbox"
                  checked={f.checked}
                  onchange={(e) => {
                    const on = (e.currentTarget as HTMLInputElement).checked;
                    setChecked((p) => (p === f.path ? on : null));
                  }}
                /><span class="k k-{f.kind}">{f.kind.charAt(0).toUpperCase()}</span>{f.path}</label
              >
            {/each}
          </div>
        {/if}
        {#if dialog.action.endsWith("_pr") || dialog.action === "create_pr"}
          <label class="chk"><input type="checkbox" bind:checked={dialog.draft} />{$t("llm.central.vcs.git.draft", { term })}</label>
        {/if}
        <div class="dlg-actions">
          <span class="hint">{$t("llm.central.vcs.git.run_hint")}</span>
          <button class="ghost" onclick={() => (dialog = null)}>{$t("llm.central.vcs.cancel")}</button>
          <button
            class="primary"
            disabled={!dialog.message.trim() || (dialog.files.length > 0 && !dialog.files.some((f) => f.checked))}
            onclick={() => dialog && run(dialog.action, dialog.message, dialog.draft)}>{label(dialog.action)}</button
          >
        </div>
      </div>
    </div>
  {/if}

  {#if branchDlg}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="backdrop" use:portal onclick={() => (branchDlg = null)}>
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="dlg" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={onDialogKey}>
        <h3>{tx("action.branches", "Branches…")}</h3>
        <div class="newb">
          <input
            placeholder={tx("new_branch", "New branch name")}
            bind:value={branchDlg.name}
            spellcheck="false"
            onkeydown={(e) => {
              if (e.key === "Enter" && branchDlg) {
                e.preventDefault();
                void branchAct("newBranch", branchDlg.name);
              }
            }}
          />
          <button class="primary" disabled={!branchDlg.name.trim()} onclick={() => branchDlg && branchAct("newBranch", branchDlg.name)}>{tx("action.newBranch", "Create")}</button>
        </div>
        <div class="blist">
          {#each branchDlg.list as b (b.name + (b.remote ? ":r" : ""))}
            <div class="brow" class:cur={b.current}>
              <span class="bname">{b.current ? "● " : ""}{b.name}</span>
              {#if b.remote}<span class="pill">{tx("remote", "remote")}</span>{/if}
              {#if !b.current}
                <button class="ghost" onclick={() => branchAct("checkout", b.name)}>{tx("action.checkout", "Checkout")}</button>
                <button class="ghost" onclick={() => branchAct("merge", b.name)}>{tx("action.merge", "Merge")}</button>
                {#if !b.remote}<button class="ghost danger" onclick={() => branchAct("deleteBranch", b.name, true)}>{tx("action.deleteBranch", "Delete")}</button>{/if}
              {/if}
            </div>
          {:else}
            <p class="hint">{tx("no_branches", "No branches")}</p>
          {/each}
        </div>
        <div class="dlg-actions"><span class="hint"></span><button class="ghost" onclick={() => (branchDlg = null)}>{$t("llm.central.vcs.cancel")}</button></div>
      </div>
    </div>
  {/if}

  {#if stashDlg}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="backdrop" use:portal onclick={() => (stashDlg = null)}>
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="dlg" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={onDialogKey}>
        <h3>{tx("action.stash", "Stash…")}</h3>
        <div class="newb">
          <input placeholder={tx("stash_message", "Stash message (optional)")} bind:value={stashDlg.message} />
          <button class="primary" disabled={!status?.dirty} onclick={() => stashDlg && extra("stashPush", { message: stashDlg.message })}>{tx("action.stashPush", "Stash changes")}</button>
        </div>
        <div class="blist">
          {#each stashDlg.list as st, i (st)}
            <div class="brow">
              <span class="bname">{st}</span>
              <button class="ghost" onclick={() => extra("stashApply", { index: i })}>{tx("action.stashApply", "Apply")}</button>
              <button class="ghost" onclick={() => extra("stashPop", { index: i })}>{tx("action.stashPop", "Pop")}</button>
              <button class="ghost danger" onclick={() => extra("stashDrop", { index: i })}>{tx("action.stashDrop", "Drop")}</button>
            </div>
          {:else}
            <p class="hint">{tx("no_stashes", "No stashes")}</p>
          {/each}
        </div>
        <div class="dlg-actions"><span class="hint"></span><button class="ghost" onclick={() => (stashDlg = null)}>{$t("llm.central.vcs.cancel")}</button></div>
      </div>
    </div>
  {/if}

  {#if failure}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="backdrop" use:portal onclick={() => (failure = null)}>
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="dlg" role="alertdialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={onDialogKey}>
        <h3>{failure.title}</h3>
        {#if failure.conflicts.length}
          <p class="hint">{tx("conflicts_in", "Conflicts in {{count}} files. Open Changes to see Current and Incoming, or abort the merge.", { count: failure.conflicts.length })}</p>
          <ul class="clist">{#each failure.conflicts as c (c)}<li>{c}</li>{/each}</ul>
        {/if}
        <pre class="err">{failure.text}</pre>
        <div class="dlg-actions">
          <span class="hint"></span>
          {#if failure.conflicts.length && ops}
            <button class="ghost danger" onclick={() => { failure = null; void extra("mergeAbort"); }}>{tx("action.mergeAbort", "Abort merge")}</button>
          {/if}
          <button class="ghost" onclick={copyFailure}>{tx("copy", "Copy")}</button>
          <button class="primary" onclick={() => (failure = null)}>OK</button>
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
    position: fixed;
    z-index: 1000;
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
  .menu-sep {
    height: 1px;
    background: var(--separator);
    margin: 4px 2px;
  }
  .files {
    margin-top: 10px;
    max-height: 180px;
    overflow: auto;
    border: 1px solid var(--separator);
    border-radius: 8px;
    padding: 4px 8px;
  }
  .chk.file {
    margin-top: 2px;
    font-family: var(--font-mono, monospace);
    font-size: 11px;
  }
  .chk.all {
    margin-top: 2px;
    font-weight: 600;
  }
  .k {
    font-size: 10px;
    width: 12px;
    color: var(--text-muted);
  }
  .k-added,
  .k-untracked {
    color: var(--green, #34c759);
  }
  .k-deleted,
  .k-conflict {
    color: var(--red, #ff3b30);
  }
  .newb {
    display: flex;
    gap: 6px;
    margin-bottom: 8px;
  }
  .newb input {
    flex: 1;
    background: var(--input-bg, var(--surface));
    color: var(--text);
    border: 1px solid var(--input-border, var(--separator));
    border-radius: 8px;
    padding: 6px 8px;
    font-size: 12px;
  }
  .blist {
    max-height: 300px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .brow {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 4px;
    border-radius: 6px;
  }
  .brow:hover {
    background: var(--fill-2, var(--surface-mut));
  }
  .brow.cur .bname {
    font-weight: 600;
  }
  .bname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono, monospace);
    font-size: 12px;
  }
  .danger {
    color: var(--red, #ff3b30) !important;
  }
  .err {
    white-space: pre-wrap;
    user-select: text;
    max-height: 240px;
    overflow: auto;
    font-size: 11px;
    background: var(--input-bg, var(--surface));
    border: 1px solid var(--separator);
    border-radius: 8px;
    padding: 8px;
    margin: 8px 0 0;
  }
  .clist {
    margin: 6px 0;
    padding-left: 18px;
    font-family: var(--font-mono, monospace);
    font-size: 11px;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 1000;
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
