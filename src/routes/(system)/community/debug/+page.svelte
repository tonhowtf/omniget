<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { BUILD_INFO } from "$lib/build-info";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { isDebugEnabled } from "$lib/stores/debug-store.svelte";

  type DependencyStatus = {
    name: string;
    installed: boolean;
    version: string | null;
  };

  type DebugInfo = {
    os: string;
    arch: string;
    os_family: string;
    proxy_enabled: boolean;
  };

  let debug = $state<DebugInfo | null>(null);
  let deps = $state<DependencyStatus[]>([]);
  let copied = $state(false);

  onMount(async () => {
    if (!isDebugEnabled()) {
      showToast("info", $t("about.debug_disabled_redirect"));
      await goto("/community/about");
      return;
    }
    const [d, dp] = await Promise.all([
      invoke<DebugInfo>("get_debug_info").catch(() => null),
      invoke<DependencyStatus[]>("check_dependencies").catch(() => []),
    ]);
    debug = d;
    deps = dp;
  });

  const report = $derived.by(() => {
    const lines: string[] = [];
    lines.push("```");
    lines.push(`OmniGet v${BUILD_INFO.version}`);
    if (BUILD_INFO.commit && BUILD_INFO.commit !== "unknown") {
      lines.push(`Build: ${BUILD_INFO.commitShort} (${BUILD_INFO.branch}) · ${BUILD_INFO.date}`);
    }
    if (debug) {
      lines.push(`OS: ${debug.os} (${debug.os_family}, ${debug.arch})`);
      lines.push(`Proxy: ${debug.proxy_enabled ? "enabled" : "disabled"}`);
    }
    lines.push("");
    lines.push("Dependencies:");
    if (deps.length === 0) {
      lines.push("  (none detected)");
    } else {
      for (const d of deps) {
        const v = d.installed ? d.version ?? "unknown" : "not installed";
        lines.push(`  - ${d.name}: ${v}`);
      }
    }
    lines.push("```");
    return lines.join("\n");
  });

  async function copyReport() {
    try {
      await navigator.clipboard.writeText(report);
      copied = true;
      showToast("success", $t("about.debug.copied"));
      setTimeout(() => {
        copied = false;
      }, 2000);
    } catch {
      showToast("error", $t("about.debug.copy_failed"));
    }
  }
</script>

<section class="debug-card surface-card">
  <div class="debug-header">
    <h2 class="debug-title">{$t("about.debug.title")}</h2>
    <p class="debug-desc">{$t("about.debug.description")}</p>
  </div>

  <pre class="debug-report">{report}</pre>

  <button type="button" class="btn btn-secondary btn-sm copy-button" onclick={copyReport}>
    {#if copied}
      {$t("about.debug.copied_short")}
    {:else}
      {$t("about.debug.copy_button")}
    {/if}
  </button>
</section>

<style>
  .debug-card {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 20px 22px;
  }

  .debug-header {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .debug-title {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.015em;
    color: var(--text);
  }

  .debug-desc {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
    color: var(--text-muted);
  }

  .debug-report {
    width: 100%;
    margin: 0;
    padding: 14px 16px;
    background: var(--bg);
    border: 1px solid var(--content-border);
    border-radius: var(--radius-md);
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.55;
    color: var(--text);
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
    overflow-x: auto;
  }

  .copy-button {
    align-self: flex-start;
  }
</style>
