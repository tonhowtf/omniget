<script lang="ts">
  /**
   * Create or edit a group room: name, members (existing bots) with a role
   * each, optional coordinator, who answers without a mention, and — in
   * "Advanced" — the limits the backend enforces. On an existing room, the
   * user can share memory into it explicitly; nothing private reaches a room
   * otherwise.
   */
  import { onMount, untrack } from "svelte";
  import { t } from "$lib/i18n";
  import type { AgentDef, MentionPolicy, Room, RoomLimits } from "$lib/llm/types";
  import {
    createRoom,
    deleteRoom,
    loadShares,
    shareIntoRoom,
    unshareFromRoom,
    updateRoom,
    type RoomShare,
  } from "$lib/stores/llm-store.svelte";

  let {
    room = null,
    agents,
    onclose,
  }: {
    room?: Room | null;
    agents: AgentDef[];
    onclose: (saved: Room | null) => void;
  } = $props();

  const DEFAULT_LIMITS: RoomLimits = {
    max_depth: 2,
    max_delegations_per_round: 4,
    max_turns_per_round: 8,
    max_concurrency: 3,
    max_task_ms: 180000,
    max_tokens_per_round: null,
    unknown_run_tokens: 4000,
  };

  let dialogEl = $state<HTMLDialogElement | null>(null);
  let titleInput = $state<HTMLInputElement | null>(null);
  // The form edits a copy taken once, when the dialog opens.
  const initial = untrack(() => room);
  let title = $state(initial?.title ?? "");
  let selected = $state<Record<string, boolean>>(
    Object.fromEntries((initial?.members ?? []).map((m) => [m.bot, true])),
  );
  let roles = $state<Record<string, string>>(
    Object.fromEntries((initial?.members ?? []).map((m) => [m.bot, m.role])),
  );
  let coordinator = $state<string>(initial?.coordinator ?? "");
  let defaultBot = $state<string>(initial?.default_bot ?? "");
  let policy = $state<MentionPolicy>(initial?.mention_policy ?? "mentions_or_coordinator");
  let limits = $state<RoomLimits>({ ...(initial?.limits ?? DEFAULT_LIMITS) });
  let taskSeconds = $state(Math.round((initial?.limits ?? DEFAULT_LIMITS).max_task_ms / 1000));
  let tokenCap = $state<string>(initial?.limits.max_tokens_per_round ? String(initial.limits.max_tokens_per_round) : "");
  let saving = $state(false);
  let errorKey = $state<string | null>(null);
  let shares = $state<RoomShare[]>([]);

  let chosen = $derived(agents.filter((a) => selected[a.id]));
  let removedMembers = $derived(
    (room?.members ?? []).filter((m) => !selected[m.bot]).map((m) => agents.find((a) => a.id === m.bot)?.name ?? m.bot),
  );

  onMount(() => {
    dialogEl?.showModal();
    titleInput?.focus();
    if (room) void loadShares(room.id).then((list) => (shares = list));
  });

  function close(saved: Room | null) {
    dialogEl?.close();
    onclose(saved);
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    errorKey = null;
    if (!title.trim()) { errorKey = "assist.groups.err_title"; return; }
    if (chosen.length === 0) { errorKey = "assist.groups.err_members"; return; }
    const members = chosen.map((a) => ({ bot: a.id, role: (roles[a.id] ?? "").trim() }));
    const ids = new Set(members.map((m) => m.bot));
    const draft = {
      title: title.trim(),
      members,
      coordinator: coordinator && ids.has(coordinator) ? coordinator : null,
      default_bot: defaultBot && ids.has(defaultBot) ? defaultBot : null,
      mention_policy: policy,
      limits: {
        ...limits,
        max_task_ms: Math.max(5, Math.min(3600, Number(taskSeconds) || 180)) * 1000,
        max_tokens_per_round: tokenCap.trim() ? Math.max(1, Number(tokenCap) || 0) : null,
      },
    };
    saving = true;
    try {
      const saved = room ? await updateRoom(room.id, draft) : await createRoom(draft);
      if (saved) close(saved);
      else errorKey = "assist.groups.err_save";
    } finally {
      saving = false;
    }
  }

  async function remove() {
    if (!room) return;
    if (await deleteRoom(room.id)) close(null);
    else errorKey = "assist.groups.err_save";
  }

  function shareOf(scope: string): RoomShare | undefined {
    return shares.find((s) => s.scope === scope && !s.record_id);
  }

  async function toggleShare(scope: string) {
    if (!room) return;
    const existing = shareOf(scope);
    const ok = existing ? await unshareFromRoom(room.id, existing.id) : await shareIntoRoom(room.id, scope);
    if (!ok) errorKey = "assist.groups.err_save";
    shares = await loadShares(room.id);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialogEl}
  class="room-dialog"
  aria-labelledby="room-editor-title"
  aria-modal="true"
  onkeydown={(e) => { if (e.key === "Escape") { e.preventDefault(); close(null); } }}
>
  <form class="room-form" onsubmit={save}>
    <header class="room-head">
      <h3 id="room-editor-title">{room ? $t("assist.groups.edit_title") : $t("assist.groups.new_title")}</h3>
      <button type="button" class="button" onclick={() => close(null)}>{$t("assist.groups.cancel")}</button>
    </header>

    <label class="field">
      <span>{$t("assist.groups.name")}</span>
      <input class="input" bind:this={titleInput} bind:value={title} placeholder={$t("assist.groups.name_placeholder")} maxlength="80" />
    </label>

    <fieldset class="field">
      <legend>{$t("assist.groups.members")}</legend>
      {#if agents.length === 0}
        <p class="dim">{$t("assist.groups.no_bots")}</p>
      {/if}
      <ul class="member-list">
        {#each agents as agent (agent.id)}
          <li>
            <label class="member-check">
              <input type="checkbox" bind:checked={selected[agent.id]} />
              <span>{agent.name}</span>
            </label>
            {#if selected[agent.id]}
              <input
                class="input role-input"
                bind:value={roles[agent.id]}
                placeholder={$t("assist.groups.role_placeholder")}
                aria-label={$t("assist.groups.role_label", { name: agent.name }) as string}
                maxlength="60"
              />
            {/if}
          </li>
        {/each}
      </ul>
      {#if removedMembers.length > 0}
        <p class="note" role="note">{$t("assist.groups.removed_note", { names: removedMembers.join(", ") })}</p>
      {/if}
    </fieldset>

    <div class="field-row">
      <label class="field">
        <span>{$t("assist.groups.coordinator")}</span>
        <select class="input" bind:value={coordinator}>
          <option value="">{$t("assist.groups.no_coordinator")}</option>
          {#each chosen as agent (agent.id)}<option value={agent.id}>{agent.name}</option>{/each}
        </select>
      </label>
      <label class="field">
        <span>{$t("assist.groups.default_bot")}</span>
        <select class="input" bind:value={defaultBot}>
          <option value="">{$t("assist.groups.first_member")}</option>
          {#each chosen as agent (agent.id)}<option value={agent.id}>{agent.name}</option>{/each}
        </select>
      </label>
    </div>

    <fieldset class="field">
      <legend>{$t("assist.groups.who_answers")}</legend>
      <label class="radio"><input type="radio" bind:group={policy} value="mentions_or_coordinator" /> {$t("assist.groups.policy_default")}</label>
      <label class="radio"><input type="radio" bind:group={policy} value="mentions_only" /> {$t("assist.groups.policy_mentions")}</label>
    </fieldset>

    {#if room}
      <fieldset class="field">
        <legend>{$t("assist.groups.memory_title")}</legend>
        <p class="note">{$t("assist.groups.memory_hint")}</p>
        <label class="member-check">
          <input type="checkbox" checked={!!shareOf("user")} onchange={() => void toggleShare("user")} />
          <span>{$t("assist.groups.share_profile")}</span>
        </label>
        {#each room.members as member (member.bot)}
          <label class="member-check">
            <input type="checkbox" checked={!!shareOf(`bot:${member.bot}`)} onchange={() => void toggleShare(`bot:${member.bot}`)} />
            <span>{$t("assist.groups.share_bot", { name: agents.find((a) => a.id === member.bot)?.name ?? member.bot })}</span>
          </label>
        {/each}
      </fieldset>
    {/if}

    <details class="advanced">
      <summary>{$t("assist.groups.advanced")}</summary>
      <p class="note">{$t("assist.groups.limits_hint")}</p>
      <div class="limits">
        <label>{$t("assist.groups.limit_depth")}<input class="input" type="number" min="0" max="5" bind:value={limits.max_depth} /></label>
        <label>{$t("assist.groups.limit_delegations")}<input class="input" type="number" min="0" max="32" bind:value={limits.max_delegations_per_round} /></label>
        <label>{$t("assist.groups.limit_turns")}<input class="input" type="number" min="1" max="64" bind:value={limits.max_turns_per_round} /></label>
        <label>{$t("assist.groups.limit_concurrency")}<input class="input" type="number" min="1" max="8" bind:value={limits.max_concurrency} /></label>
        <label>{$t("assist.groups.limit_time")}<input class="input" type="number" min="5" max="3600" bind:value={taskSeconds} /></label>
        <label>{$t("assist.groups.limit_tokens")}<input class="input" type="number" min="1" bind:value={tokenCap} placeholder={$t("assist.groups.no_cap")} /></label>
      </div>
    </details>

    {#if errorKey}<p class="error" role="alert">{$t(errorKey)}</p>{/if}

    <footer class="room-foot">
      {#if room}
        <button type="button" class="button danger" onclick={remove}>{$t("assist.groups.delete")}</button>
      {/if}
      <button type="submit" class="button primary" disabled={saving}>{room ? $t("assist.groups.save") : $t("assist.groups.create")}</button>
    </footer>
  </form>
</dialog>

<style>
  .room-dialog {
    border: none;
    border-radius: var(--radius-lg);
    padding: 0;
    width: min(560px, calc(100vw - 32px));
    max-height: calc(100vh - 32px);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-lg);
  }
  .room-dialog::backdrop { background: rgba(0, 0, 0, 0.35); }
  .room-form { display: flex; flex-direction: column; gap: var(--space-3); padding: var(--space-4); }
  .room-head { display: flex; justify-content: space-between; align-items: center; gap: var(--space-2); }
  .room-head h3 { margin: 0; font-size: var(--text-md); }
  .field { display: flex; flex-direction: column; gap: var(--space-1); border: 0; padding: 0; margin: 0; min-width: 0; }
  .field > span, legend { font-size: var(--text-sm); font-weight: 600; color: var(--text); padding: 0; }
  .field-row { display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-3); }
  .member-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-1); }
  .member-list li { display: flex; align-items: center; gap: var(--space-2); flex-wrap: wrap; }
  .member-check { display: inline-flex; align-items: center; gap: var(--space-2); font-size: var(--text-sm); min-width: 160px; }
  .role-input { flex: 1; min-width: 140px; }
  .radio { display: flex; align-items: center; gap: var(--space-2); font-size: var(--text-sm); }
  .note { margin: 0; font-size: var(--text-caption); color: var(--text-dim); overflow-wrap: anywhere; }
  .dim { color: var(--text-dim); font-size: var(--text-sm); margin: 0; }
  .advanced summary { cursor: pointer; font-size: var(--text-sm); color: var(--text-muted, var(--text-dim)); }
  .advanced summary:focus-visible { outline: var(--focus-ring); }
  .limits { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: var(--space-2); margin-top: var(--space-2); }
  .limits label { display: flex; flex-direction: column; gap: 2px; font-size: var(--text-caption); color: var(--text-dim); }
  .error { margin: 0; color: var(--danger); font-size: var(--text-sm); overflow-wrap: anywhere; }
  .room-foot { display: flex; justify-content: flex-end; gap: var(--space-2); flex-wrap: wrap; }
  .danger { color: var(--danger); margin-right: auto; }
  @media (max-width: 520px) { .field-row { grid-template-columns: 1fr; } }
</style>
