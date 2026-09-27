<script lang="ts">
  // "Aplicar vencedor": mostra o que vai acontecer (branch alvo, conflitos,
  // se a branch está em checkout no clone do usuário e se ele está sujo),
  // pede confirmação explícita e, depois, oferece limpar as perdedoras.
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { vcsBranches, type BranchInfo } from "$lib/central/vcs";
  import {
    arenaApply,
    arenaApplyPreview,
    arenaCleanup,
    arenaError,
    type ApplyPreview,
    type ApplyResult,
    type ApplyStrategy,
    type ArenaEntry,
    type ArenaView,
  } from "$lib/central/arena";
  import Modal from "$components/central/threads/Modal.svelte";
  import Icon from "$components/central/threads/Icon.svelte";

  let {
    open,
    arena,
    entry,
    onclose,
    ondone,
  }: {
    open: boolean;
    arena: ArenaView;
    entry: ArenaEntry | null;
    onclose: () => void;
    ondone: (a: ArenaView) => void;
  } = $props();

  let preview = $state<ApplyPreview | null>(null);
  let branches = $state<BranchInfo[]>([]);
  let target = $state("");
  let strategy = $state<ApplyStrategy>("merge");
  let message = $state("");
  let touchCheckout = $state(false);
  let understood = $state(false);
  let cleanLosers = $state(true);
  let deleteBranches = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let result = $state<ApplyResult | null>(null);
  let cleanupNote = $state<string | null>(null);

  $effect(() => {
    const id = entry?.entryId;
    if (!open || !id) return;
    untrack(reset);
  });

  function reset() {
    preview = null;
    result = null;
    cleanupNote = null;
    error = null;
    understood = false;
    touchCheckout = false;
    target = arena.baseBranch ?? "";
    message = "";
    void vcsBranches(arena.repoRoot)
      .then((b) => (branches = b.filter((x) => !x.remote)))
      .catch(() => (branches = []));
    void load();
  }

  async function load() {
    if (!entry) return;
    busy = true;
    error = null;
    try {
      preview = await arenaApplyPreview(arena.arenaId, entry.entryId, target || null);
      target = preview.targetBranch;
    } catch (e) {
      preview = null;
      error = arenaError(e).message;
    } finally {
      busy = false;
    }
  }

  let blocked = $derived(
    !preview ||
      preview.alreadyApplied ||
      preview.conflicts.length > 0 ||
      (!!preview.checkedOutAt && (preview.checkoutDirty || !touchCheckout)) ||
      !understood,
  );

  async function apply() {
    if (!entry || blocked) return;
    busy = true;
    error = null;
    try {
      result = await arenaApply({
        arenaId: arena.arenaId,
        entryId: entry.entryId,
        strategy,
        targetBranch: target || null,
        touchCheckout,
        message: message.trim() || null,
      });
      let latest = result.arena;
      if (cleanLosers) {
        const c = await arenaCleanup(arena.arenaId, "losers", deleteBranches);
        latest = c.arena;
        cleanupNote = $t("llm.central.arena.apply.cleaned", { count: c.archived.length }) as string;
        if (c.errors.length) cleanupNote += ` · ${c.errors.join("; ")}`;
      }
      ondone(latest);
    } catch (e) {
      error = arenaError(e).message;
    } finally {
      busy = false;
    }
  }
</script>

<Modal {open} title={$t("llm.central.arena.apply.title") as string} {onclose} width={560}>
  {#if entry}
    <div class="body">
      <p class="lede">
        {$t("llm.central.arena.apply.lede", { name: entry.label })}
      </p>

      {#if result}
        <div class="done">
          <Icon name="check-circle" size={18} />
          <div>
            <strong>{$t("llm.central.arena.apply.ok", { branch: result.targetBranch })}</strong>
            <p class="mono">{result.commit.slice(0, 10)} · {result.updated === "checkout" ? $t("llm.central.arena.apply.updated_checkout") : $t("llm.central.arena.apply.updated_ref")}</p>
            {#if cleanupNote}<p class="muted">{cleanupNote}</p>{/if}
          </div>
        </div>
      {:else}
        <div class="grid">
          <label class="field">
            <span>{$t("llm.central.arena.apply.target")}</span>
            <select bind:value={target} onchange={load} disabled={busy}>
              {#if target && !branches.some((b) => b.name === target)}<option value={target}>{target}</option>{/if}
              {#each branches as b (b.name)}<option value={b.name}>{b.name}</option>{/each}
            </select>
          </label>
          <fieldset class="field">
            <legend>{$t("llm.central.arena.apply.strategy")}</legend>
            <label class="radio"><input type="radio" bind:group={strategy} value="merge" /> {$t("llm.central.arena.apply.merge")}</label>
            <label class="radio"><input type="radio" bind:group={strategy} value="squash" /> {$t("llm.central.arena.apply.squash")}</label>
          </fieldset>
        </div>

        <label class="field">
          <span>{$t("llm.central.arena.apply.message")}</span>
          <input type="text" bind:value={message} placeholder={`Arena: ${arena.title}`} disabled={busy} />
        </label>

        {#if preview}
          <ul class="facts">
            <li><Icon name="git-diff" size={13} /> <span class="add">+{preview.additions}</span> <span class="del">−{preview.deletions}</span> · {$t("llm.central.arena.files", { count: preview.filesChanged })}</li>
            <li><Icon name="git-branch" size={13} /> <span class="mono">{preview.targetBranch}@{preview.targetHead.slice(0, 7)}</span> ← <span class="mono">{preview.winnerCommit.slice(0, 7)}</span></li>
            {#if preview.alreadyApplied}
              <li class="warn"><Icon name="warning" size={13} /> {$t("llm.central.arena.apply.already")}</li>
            {/if}
            {#if preview.conflicts.length}
              <li class="bad"><Icon name="warning-circle" size={13} /> {$t("llm.central.arena.apply.conflicts", { count: preview.conflicts.length })}
                <span class="mono small">{preview.conflicts.slice(0, 8).join(", ")}</span></li>
            {/if}
            {#if preview.checkedOutAt}
              <li class="warn"><Icon name="warning" size={13} /> {$t("llm.central.arena.apply.checked_out", { path: preview.checkedOutAt })}</li>
              {#if preview.checkoutDirty}
                <li class="bad"><Icon name="x-circle" size={13} /> {$t("llm.central.arena.apply.dirty")}</li>
              {:else}
                <li><label class="check"><input type="checkbox" bind:checked={touchCheckout} /> {$t("llm.central.arena.apply.touch_checkout")}</label></li>
              {/if}
            {:else}
              <li><Icon name="shield-check" size={13} /> {$t("llm.central.arena.apply.ref_only")}</li>
            {/if}
          </ul>
        {/if}

        <div class="cleanup">
          <label class="check"><input type="checkbox" bind:checked={cleanLosers} /> {$t("llm.central.arena.apply.clean_losers")}</label>
          <label class="check sub" class:off={!cleanLosers}><input type="checkbox" bind:checked={deleteBranches} disabled={!cleanLosers} /> {$t("llm.central.arena.apply.delete_branches")}</label>
        </div>

        <label class="check confirm"><input type="checkbox" bind:checked={understood} /> {$t("llm.central.arena.apply.confirm", { branch: target || "?" })}</label>
      {/if}

      {#if error}<p class="err" role="alert">{error}</p>{/if}
    </div>
  {/if}
  {#snippet footer()}
    <button type="button" class="button" onclick={onclose}>{result ? $t("llm.central.arena.close") : $t("llm.central.arena.cancel")}</button>
    {#if !result}
      <button type="button" class="button active" onclick={apply} disabled={blocked || busy}>
        {#if busy}<Icon name="circle-notch" size={14} />{/if}
        {$t("llm.central.arena.apply.do")}
      </button>
    {/if}
  {/snippet}
</Modal>

<style>
  .body { display: flex; flex-direction: column; gap: 12px; }
  .lede { margin: 0; color: var(--text-muted); font-size: var(--text-base); }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .field { display: flex; flex-direction: column; gap: 4px; border: 0; padding: 0; margin: 0; min-width: 0; }
  .field > span, legend { font-size: var(--text-sm); font-weight: 600; padding: 0; }
  select, input[type="text"] { font: inherit; font-size: var(--text-base); color: var(--text); background: var(--input-bg, var(--surface)); border: 1px solid var(--input-border, var(--separator)); border-radius: var(--radius-md); padding: 6px 9px; }
  .radio, .check { display: flex; align-items: center; gap: 6px; font-size: var(--text-base); }
  .check.sub { padding-left: 22px; }
  .check.off { opacity: 0.5; }
  .confirm { font-weight: 600; }
  .facts { list-style: none; margin: 0; padding: 8px 10px; display: flex; flex-direction: column; gap: 5px; background: var(--fill-1); border-radius: var(--radius-md); font-size: var(--text-sm); }
  .facts li { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .warn { color: var(--warning); }
  .bad { color: var(--error); }
  .add { color: var(--success); }
  .del { color: var(--error); }
  .mono { font-family: var(--font-mono); }
  .small { font-size: var(--text-xs); }
  .muted { color: var(--text-muted); }
  .cleanup { display: flex; flex-direction: column; gap: 4px; }
  .done { display: flex; gap: 10px; color: var(--success); }
  .done p { margin: 2px 0 0; color: var(--text-muted); font-size: var(--text-sm); }
  .err { color: var(--error); margin: 0; font-size: var(--text-sm); overflow-wrap: anywhere; }
</style>
