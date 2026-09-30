<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import TgIcon from "./TgIcon.svelte";

  let { value = $bindable("") }: { value: string } = $props();

  async function pick() {
    try {
      const dir = await open({ directory: true, multiple: false, defaultPath: value || undefined });
      if (typeof dir === "string" && dir) value = dir;
    } catch {
      /* dialog closed or unavailable */
    }
  }
</script>

<button class="out btn btn-ghost btn-sm" onclick={pick} title={value}>
  <TgIcon name="folder" />
  <span class="path">{value || $t("telegram.common.choose_folder")}</span>
</button>

<style>
  .out {
    max-width: 280px;
    min-width: 0;
  }

  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
</style>
