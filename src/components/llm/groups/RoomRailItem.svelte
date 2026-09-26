<script lang="ts">
  /** One group room in the rail: its name, who is in it, and whether it is working. */
  import { t } from "$lib/i18n";
  import type { Room } from "$lib/llm/types";
  import { getAgent, getRoomLive } from "$lib/stores/llm-store.svelte";

  let {
    room,
    active = false,
    onselect,
  }: {
    room: Room;
    active?: boolean;
    onselect: (id: string) => void;
  } = $props();

  let working = $derived(getRoomLive(room.id).length > 0);
  let names = $derived(room.members.map((m) => getAgent(m.bot)?.name ?? m.bot).join(", "));
</script>

<button
  type="button"
  class="room-item"
  class:active
  aria-current={active ? "true" : undefined}
  onclick={() => onselect(room.id)}
>
  <span class="room-badge" aria-hidden="true">{room.members.length}</span>
  <span class="room-body">
    <span class="room-name">{room.title}</span>
    <span class="room-preview">{working ? $t("llm.surface.working") : names}</span>
  </span>
</button>

<style>
  .room-item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    height: 56px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  .room-item:focus-visible { outline: var(--focus-ring); }
  @media (hover: hover) { .room-item:hover { background: var(--fill-2); } }
  .room-item.active { background: var(--accent-soft); }
  .room-badge {
    width: 36px;
    height: 36px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    border: var(--hairline) solid var(--separator);
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-muted, var(--text-dim));
  }
  .room-body { display: flex; flex-direction: column; gap: 1px; min-width: 0; flex: 1; }
  .room-name { font-size: var(--text-sm); font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .room-preview { font-size: var(--text-caption); color: var(--text-dim); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
