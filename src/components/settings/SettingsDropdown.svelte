<script lang="ts">
  type Option = {
    value: string;
    label: string;
  };

  let {
    label = "",
    description = "",
    value = $bindable(""),
    options = [] as Option[],
    onChange,
  } = $props();

  const selectId = `dropdown-${Math.random().toString(36).slice(2, 9)}`;
  const labelId = `${selectId}-label`;

  function handleChange(e: Event) {
    const target = e.target as HTMLSelectElement;
    value = target.value;
    onChange?.(value);
  }
</script>

<div class="settings-dropdown">
  <div class="dropdown-labels">
    <label class="dropdown-label" id={labelId} for={selectId}>
      {label}
    </label>
    {#if description}
      <p class="dropdown-description">{description}</p>
    {/if}
  </div>

  <div class="dropdown-wrapper">
    <select
      id={selectId}
      class="dropdown-select"
      bind:value
      onchange={handleChange}
      aria-labelledby={labelId}
    >
      {#each options as option (option.value)}
        <option value={option.value}>{option.label}</option>
      {/each}
    </select>
  </div>
</div>

<style>
  .settings-dropdown {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding: var(--padding);
    background: var(--button);
    border-radius: var(--border-radius);
  }

  .dropdown-labels {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 4);
  }

  .dropdown-label {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--secondary);
    cursor: pointer;
  }

  .dropdown-description {
    margin: 0;
    font-size: 12.5px;
    font-weight: 400;
    color: var(--gray);
    line-height: 1.4;
  }

  .dropdown-wrapper {
    position: relative;
  }

  .dropdown-select {
    width: 100%;
    padding: 8px var(--padding);
    font-size: 14.5px;
    background: var(--input-bg);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    color: var(--secondary);
    cursor: pointer;
    appearance: none;
    padding-right: 32px;
    background-image: url('data:image/svg+xml;utf8,<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"></polyline></svg>');
    background-repeat: no-repeat;
    background-position: right 8px center;
    background-size: 16px;
  }

  .dropdown-select::placeholder {
    color: var(--gray);
  }

  .dropdown-select:focus-visible {
    border-color: var(--secondary);
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  @media (prefers-reduced-motion: reduce) {
    .dropdown-select {
      transition: none;
    }
  }
</style>
