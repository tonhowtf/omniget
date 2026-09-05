<script lang="ts">
  /** Linha de cookies (arquivo cookies.txt ou string), lembrada entre telas. */
  import { t } from "$lib/i18n";
  import { pickFile } from "$lib/tools/rt";
  import { saveCookies } from "$lib/tools/pinterest";

  let { value = $bindable("") }: { value: string } = $props();

  $effect(() => {
    saveCookies(value);
  });
</script>

<div class="group-row">
  <div class="group-row-content">
    <div class="group-row-title">{$t("tools.pinterest.cookies")}</div>
    <div class="group-row-sub">{$t("tools.pinterest.cookies_hint")}</div>
    <input class="input" type="text" bind:value placeholder={$t("tools.pinterest.cookies_placeholder")} spellcheck="false" autocomplete="off" />
  </div>
  <div class="group-row-trailing btn-row">
    {#if value}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (value = "")}>×</button>{/if}
    <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile([{ name: "cookies.txt", extensions: ["txt"] }]); if (f) value = f; }}>{$t("tools.common.choose")}</button>
  </div>
</div>
