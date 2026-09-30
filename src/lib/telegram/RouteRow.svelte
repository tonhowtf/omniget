<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgIcon from "./TgIcon.svelte";
  import { net, errText, kindLabel, sourceInfo, type RouteView } from "./api";
  import { refreshNet } from "./net.svelte";

  let {
    view,
    pending = false,
    actions = true,
    fixed = false,
  }: { view: RouteView; pending?: boolean; actions?: boolean; fixed?: boolean } = $props();

  let busy = $state<string | null>(null);
  let src = $derived(sourceInfo(view.route.source));
  let rowState = $derived(
    busy === "test" || (pending && view.health.ok == null)
      ? "testing"
      : view.health.ok === true
        ? "ok"
        : view.health.ok === false
          ? "fail"
          : "untested",
  );
  let addr = $derived(view.route.kind === "direct" ? "" : `${view.route.host}:${view.route.port}`);

  async function run(kind: string, fn: () => Promise<unknown>, okMsg?: string) {
    busy = kind;
    try {
      await fn();
      if (okMsg) showToast("success", okMsg);
      await refreshNet();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }

  async function copy() {
    if (!view.share_link) return;
    try {
      await navigator.clipboard.writeText(view.share_link);
      showToast("success", $t("telegram.common.copied") as string);
    } catch (e) {
      showToast("error", errText(e));
    }
  }
</script>

<div class="route" data-state={rowState} class:active={view.active}>
  <span class="mark" aria-hidden="true">
    {#if rowState === "testing"}
      <span class="spinner"></span>
    {:else if rowState === "ok"}
      <TgIcon name="check" size={13} />
    {:else if rowState === "fail"}
      <TgIcon name="x" size={13} />
    {:else}
      <span class="hollow"></span>
    {/if}
  </span>
  <div class="body">
    <div class="line1">
      <span class="name" title={view.route.name}>{view.route.name}</span>
      {#if view.active}<span class="tag tag-success">{fixed ? $t("telegram.net.in_use_fixed") : $t("telegram.net.in_use")}</span>{/if}
    </div>
    <div class="line2">
      <span class="tag">{kindLabel(view.route.kind)}</span>
      <span class="src">{$t(`telegram.net.source.${src.key}`, { value: src.value })}</span>
      {#if addr && !view.route.name.includes(addr)}<span class="addr">{addr}</span>{/if}
      {#if view.saved}<span class="saved">· {$t("telegram.net.saved")}</span>{/if}
    </div>
    {#if rowState === "fail" && view.health.error}
      <div class="err" title={view.health.error}>{view.health.error}</div>
    {/if}
  </div>
  <span class="ping">
    {#if view.health.ok === true && view.health.rtt_ms != null}
      {$t("telegram.status.ms", { ms: view.health.rtt_ms })}
    {:else if rowState === "fail"}
      {$t("telegram.net.failed")}
    {:else if rowState === "testing"}
      {$t("telegram.net.testing")}
    {/if}
  </span>
  {#if actions}
    <span class="acts">
      <button class="btn btn-ghost btn-sm btn-icon" title={$t("telegram.net.test")} aria-label={$t("telegram.net.test")} disabled={!!busy} onclick={() => run("test", () => net.test(view.route.id))}>
        <TgIcon name="bolt" />
      </button>
      <button
        class="btn btn-ghost btn-sm"
        disabled={!!busy || (view.active && fixed)}
        onclick={() => run("use", () => net.setMode("fixed", view.route.id), $t("telegram.net.use_done", { route: view.route.name }) as string)}
      >{$t("telegram.net.use")}</button>
      {#if !view.saved && view.route.kind !== "direct"}
        <button class="btn btn-ghost btn-sm btn-icon" title={$t("telegram.net.save")} aria-label={$t("telegram.net.save")} disabled={!!busy} onclick={() => run("keep", () => net.keep(view.route.id), $t("telegram.net.saved_done") as string)}>
          <TgIcon name="save" />
        </button>
      {/if}
      {#if view.share_link}
        <button class="btn btn-ghost btn-sm btn-icon" title={$t("telegram.net.copy_link")} aria-label={$t("telegram.net.copy_link")} onclick={copy}>
          <TgIcon name="share" />
        </button>
      {/if}
      {#if view.saved}
        <button class="btn btn-ghost btn-sm btn-icon" title={$t("telegram.net.remove")} aria-label={$t("telegram.net.remove")} disabled={!!busy} onclick={() => run("remove", () => net.removeRoute(view.route.id))}>
          <TgIcon name="trash" />
        </button>
      {/if}
    </span>
  {/if}
</div>

<style>
  .route {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    min-height: 48px;
  }

  .route.active {
    background: color-mix(in srgb, var(--success) 7%, transparent);
  }

  .mark {
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-full);
    flex-shrink: 0;
    color: var(--text-dim);
  }

  [data-state="ok"] .mark {
    background: color-mix(in srgb, var(--success) 18%, transparent);
    color: var(--success);
  }

  [data-state="fail"] .mark {
    background: color-mix(in srgb, var(--danger) 14%, transparent);
    color: var(--danger);
  }

  .mark .spinner {
    width: 12px;
    height: 12px;
  }

  .hollow {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
    box-shadow: inset 0 0 0 1.5px var(--text-dim);
  }

  .body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .line1,
  .line2 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .name {
    font-size: var(--text-base);
    font-weight: 550;
    color: var(--text);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .line2 {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .addr {
    font-family: var(--font-mono);
  }

  .err {
    font-size: var(--text-xs);
    color: var(--danger);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .ping {
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    flex-shrink: 0;
  }

  [data-state="ok"] .ping {
    color: var(--success);
  }

  .acts {
    display: flex;
    gap: 2px;
    flex-shrink: 0;
  }
</style>
