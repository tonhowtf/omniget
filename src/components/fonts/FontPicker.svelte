<script lang="ts" module>
  import type { PreviewFont } from "$lib/fonts/preview";

  export type PickerGroup = { label: string; fonts: PreviewFont[] };
</script>

<script lang="ts">
  let {
    value,
    groups,
    label,
    placeholder,
    onpick,
  }: {
    value: string | null;
    groups: PickerGroup[];
    label: string;
    placeholder?: string;
    onpick: (key: string) => void;
  } = $props();
</script>

<label class="ty-pick">
  <span class="field-label">{label}</span>
  <select class="ty-select" value={value ?? ""} onchange={(e) => {
    const key = (e.currentTarget as HTMLSelectElement).value;
    if (key) onpick(key);
  }}>
    {#if placeholder}<option value="" disabled>{placeholder}</option>{/if}
    {#each groups as group (group.label)}
      {#if group.fonts.length > 0}
        <optgroup label={group.label}>
          {#each group.fonts as f (f.key)}
            <option value={f.key}>{f.family}</option>
          {/each}
        </optgroup>
      {/if}
    {/each}
  </select>
</label>

<style>
  .ty-pick {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .ty-pick select {
    width: 100%;
    min-width: 0;
  }
</style>
