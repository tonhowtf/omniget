<script lang="ts">
  import { t } from "$lib/i18n";

  let { preset, mode }: { preset: string; mode: "light" | "dark" } = $props();
</script>

<div class="preview ds-scope" data-ds-preset={preset} data-ds-mode={mode} aria-hidden="true" data-setting-key="design.title">
  <strong>{$t("design.preview")}</strong>
  <p>{$t("design.preview_body")}</p>
  <span class="preview-action">{$t("llm.roster.new")}</span>
</div>

<style>
  .preview { padding: var(--space-4); border: 1px solid var(--border); border-radius: 10px; background: var(--bg); color: var(--text); }
  .preview p { margin: var(--space-1) 0 var(--space-3); color: var(--text-muted); font-size: 12px; line-height: 18px; }
  .preview-action { display: inline-block; background: var(--accent); color: var(--on-accent); padding: 6px 12px; border-radius: 6px; font-size: 12px; }
</style>
