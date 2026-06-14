<script lang="ts">
  let { title, hint, onclick }: {
    title: string;
    hint: string;
    onclick: () => void;
  } = $props();
</script>

<button type="button" class="settings-drill-item" {onclick}>
  <span class="settings-drill-item-text">
    <span class="settings-drill-item-title">{title}</span>
    <span class="settings-drill-item-hint">{hint}</span>
  </span>
  <span class="settings-drill-item-chevron" aria-hidden="true">›</span>
</button>
