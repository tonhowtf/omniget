<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { openCommandPalette } from "$lib/stores/command-palette-store.svelte";
  import { shortcut } from "$lib/platform";

  let pageTitle = $derived.by(() => {
    const path = page.url.pathname;
    if (path === "/") return $t("nav.home");
    if (path.startsWith("/downloads")) return $t("nav.downloads");
    if (path.startsWith("/marketplace")) return $t("nav.marketplace");
    if (path.startsWith("/settings")) return $t("nav.settings");
    if (path.startsWith("/about")) return $t("nav.about");
    return "OmniGet";
  });
</script>

<header class="mac-titlebar" data-tauri-drag-region>
  <span class="mac-titlebar-title" data-tauri-drag-region>{pageTitle}</span>
  <div class="mac-titlebar-actions">
    <button type="button" class="btn btn-ghost btn-sm" onclick={() => openCommandPalette()}>
      {$t("command_palette.open")}
      <span class="kbd">{shortcut("K")}</span>
    </button>
  </div>
</header>
