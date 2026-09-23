<script lang="ts">
  // PR surface: repo state of the thread's folder (branch, upstream,
  // ahead/behind, changes), the smart git button (commit / push / PR) and
  // the PR of this branch with a link to open it.
  import { t } from "$lib/i18n";
  import GitActionsButton from "$components/central/vcs/GitActionsButton.svelte";
  import type { PrInfo, RepoStatus } from "$lib/central/vcs";
  import { prLookup, repoStatus } from "$lib/central/threads-ui/api";
  import { indicators } from "$lib/central/threads-ui/indicators.svelte";
  import Icon from "./Icon.svelte";

  let { threadId, cwd }: { threadId: string; cwd: string | null } = $props();

  let status = $state<RepoStatus | null>(null);
  let pr = $state<PrInfo | null>(null);
  let loading = $state(false);

  async function load() {
    if (!cwd) return;
    loading = true;
    status = await repoStatus(cwd);
    pr = status?.is_repo ? await prLookup(cwd) : null;
    indicators.setPr(threadId, pr);
    loading = false;
  }

  $effect(() => {
    cwd;
    threadId;
    void load();
  });

  async function openPr() {
    if (!pr) return;
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(pr.url);
    } catch {
      window.open(pr.url, "_blank");
    }
  }
</script>

<div class="pr-panel">
  {#if !cwd}
    <p class="note">{$t("llm.central.threads.diff.no_folder")}</p>
  {:else if status && !status.is_repo}
    <p class="note">{$t("llm.central.vcs.git.not_repo")}</p>
  {:else}
    <div class="repo" aria-busy={loading}>
      <Icon name="git-branch" size={15} />
      <div class="repo-text">
        <strong>{status?.branch ?? (status?.detached ? "HEAD" : "…")}</strong>
        <span class="muted">
          {#if status?.upstream}{status.upstream}{#if status.ahead || status.behind} · ↑{status.ahead} ↓{status.behind}{/if}{:else if status}{$t("llm.central.threads.pr.no_upstream")}{/if}
          {#if status?.dirty} · {$t("llm.central.vcs.git.dirty", { count: status.files.length })}{/if}
        </span>
      </div>
    </div>
    <GitActionsButton path={cwd} onchanged={load} onpr={(p) => { pr = p; indicators.setPr(threadId, p); }} />
    {#if pr}
      <article class="pr" data-state={pr.draft ? "draft" : pr.state}>
        <header>
          <Icon name="git-pull-request" size={15} />
          <span class="num">#{pr.number}</span>
          <span class="state">{$t(`llm.central.vcs.git.pr_state.${pr.draft ? "draft" : pr.state}`)}</span>
        </header>
        <p class="title">{pr.title}</p>
        <p class="muted mono">{pr.head} → {pr.base}{pr.review_decision ? ` · ${pr.review_decision}` : ""}</p>
        <button type="button" class="button" onclick={openPr}><Icon name="arrow-square-out" size={13} />{$t("llm.central.threads.pr.open")}</button>
      </article>
    {:else if status}
      <p class="note small">{$t("llm.central.threads.pr.none")}</p>
    {/if}
  {/if}
</div>

<style>
  .pr-panel { padding: 12px; display: grid; gap: 12px; align-content: start; overflow: auto; height: 100%; box-sizing: border-box; }
  .repo { display: flex; gap: 10px; align-items: flex-start; }
  .repo-text { display: grid; gap: 2px; min-width: 0; }
  .repo-text strong { font-size: 13.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .muted { color: var(--text-muted); font-size: 12px; margin: 0; }
  .mono { font-family: var(--font-mono); }
  .pr { border: 1px solid var(--separator); border-radius: 12px; padding: 12px; display: grid; gap: 6px; }
  .pr header { display: flex; align-items: center; gap: 6px; font-size: 12.5px; }
  .pr[data-state="open"] header { color: var(--green, #22c55e); }
  .pr[data-state="merged"] header { color: var(--purple, #a855f7); }
  .pr[data-state="closed"] header { color: var(--red, #ef4444); }
  .num { font-weight: 700; }
  .state { text-transform: capitalize; }
  .title { margin: 0; font-size: 14px; font-weight: 600; color: var(--text); }
  .pr .button { justify-self: start; display: inline-flex; gap: 6px; align-items: center; }
  .note { color: var(--text-muted); font-size: 13px; text-align: center; margin-top: 24px; }
  .note.small { margin-top: 0; text-align: left; font-size: 12.5px; }
</style>
