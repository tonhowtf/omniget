<script lang="ts">
  let {
    id,
    describedBy,
    on,
    disabled = false,
    onclick,
  }: {
    id: string;
    describedBy: string | undefined;
    on: boolean;
    disabled?: boolean;
    onclick: () => void;
  } = $props();
</script>

<button {id} type="button" class="toggle" class:on role="switch" aria-checked={on} aria-labelledby={`${id}-label`} aria-describedby={describedBy} {disabled} {onclick}><span class="toggle-knob"></span></button>
