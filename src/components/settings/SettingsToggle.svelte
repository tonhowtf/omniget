<script lang="ts">
  import Toggle from "$components/misc/Toggle.svelte";

  let {
    label = "",
    description = "",
    enabled = $bindable(false),
    onToggle,
  } = $props();

  const toggleId = `toggle-${Math.random().toString(36).slice(2, 9)}`;
</script>

<div class="settings-toggle">
  <div class="toggle-row">
    <div class="toggle-labels">
      <label class="toggle-label" for={toggleId}>{label}</label>
      {#if description}
        <p class="toggle-description">{description}</p>
      {/if}
    </div>
    <Toggle bind:enabled onToggle={onToggle} />
  </div>
</div>

<style>
  .settings-toggle {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding: var(--padding);
    background: var(--button);
    border-radius: var(--border-radius);
  }

  .toggle-row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--padding);
  }

  .toggle-labels {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 4);
    flex: 1;
  }

  .toggle-label {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--secondary);
    cursor: pointer;
  }

  .toggle-description {
    margin: 0;
    font-size: 12.5px;
    font-weight: 400;
    color: var(--gray);
    line-height: 1.4;
  }
</style>
