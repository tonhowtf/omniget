<script lang="ts">
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import type { Group } from "$lib/canvas/model/types";
  import { atWorld } from "$lib/canvas/screen";

  let { group }: { group: Group } = $props();
  const engine = useEngine();
  let editing = $state(false);
  let draft = $state("");

  const bounds = $derived.by(() => {
    void engine.store.version;
    return engine.groupBounds(group.id);
  });

  function down(e: PointerEvent) {
    if (e.button !== 0 || editing) return;
    e.stopPropagation();
    e.preventDefault();
    engine.tools.startGroupDrag(e, group.id);
  }

  function rename() {
    editing = false;
    const name = draft.trim();
    if (name && name !== group.name) engine.renameGroup(group.id, name);
  }
</script>

{#if bounds}
  <div class="pos" style:transform={atWorld(bounds.x, bounds.y, 0, -26)}>
    <div
      class="header"
      data-canvas-chrome
      role="button"
      tabindex="-1"
      style:--gc={group.color ?? "#7c8cff"}
      onpointerdown={down}
      ondblclick={(e) => {
        e.stopPropagation();
        draft = group.name ?? "";
        editing = true;
      }}
    >
      <span class="swatch"></span>
      {#if editing}
        <!-- svelte-ignore a11y_autofocus -->
        <input
          bind:value={draft}
          autofocus
          onblur={rename}
          onkeydown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter") rename();
            if (e.key === "Escape") editing = false;
          }}
        />
      {:else}
        <span class="name">{group.name || $t("canvas.engine.group", { default: "Group" })}</span>
      {/if}
    </div>
  </div>
{/if}

<style>
  .pos {
    position: absolute;
    left: 0;
    top: 0;
    pointer-events: none;
  }
  .header {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 8px;
    border-radius: 6px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text);
    background: color-mix(in srgb, var(--gc) 18%, var(--popup-bg, #222));
    border: 1px solid color-mix(in srgb, var(--gc) 55%, transparent);
    pointer-events: auto;
    cursor: grab;
    user-select: none;
    white-space: nowrap;
  }
  .swatch {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--gc);
  }
  input {
    width: 120px;
    font: inherit;
    border: none;
    outline: none;
    background: transparent;
    color: inherit;
  }
</style>
