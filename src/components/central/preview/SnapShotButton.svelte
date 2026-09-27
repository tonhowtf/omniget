<script lang="ts">
  // Composer button: capture another app's window (or the whole screen, or
  // this thread's Browser page) to a PNG under <app_data>/snapshots and hand
  // the path back as an attachment.
  import { t } from "$lib/i18n";
  import Icon from "$components/central/threads/Icon.svelte";
  import { errText, previewApi, type WindowInfo } from "./preview-api";

  let { threadId, onattach }: { threadId: string | null; onattach: (path: string) => void } = $props();

  let open = $state(false);
  let loading = $state(false);
  let busy = $state<string | null>(null);
  let windows = $state<WindowInfo[]>([]);
  let permission = $state(true);
  let note = $state<string | null>(null);
  let error = $state<string | null>(null);
  let hasPreview = $state(false);
  let btn = $state<HTMLButtonElement | null>(null);
  let pos = $state({ left: 0, bottom: 0 });

  async function load() {
    loading = true;
    error = null;
    try {
      const [list, states] = await Promise.all([previewApi.listWindows(), threadId ? previewApi.state(threadId).catch(() => []) : Promise.resolve([])]);
      windows = list.windows;
      permission = list.permission;
      note = list.note;
      hasPreview = states.some((s) => s.url && s.url !== "about:blank");
    } catch (e) {
      error = errText(e);
    } finally {
      loading = false;
    }
  }

  function toggle() {
    open = !open;
    if (open) {
      const r = btn?.getBoundingClientRect();
      if (r) pos = { left: Math.max(8, Math.min(r.left, window.innerWidth - 348)), bottom: window.innerHeight - r.top + 6 };
      void load();
    }
  }

  async function take(id: string) {
    busy = id;
    error = null;
    try {
      const shot = id === "__preview" && threadId ? await previewApi.screenshot(threadId) : await previewApi.capture(id);
      onattach(shot.path);
      open = false;
    } catch (e) {
      error = errText(e);
    } finally {
      busy = null;
    }
  }

  async function grant() {
    permission = await previewApi.requestPermission().catch(() => false);
    void load();
  }

  function label(w: WindowInfo): string {
    if (w.id === "screen") return $t("llm.central.preview.snap.screen") as string;
    return w.title || w.app || `#${w.id}`;
  }
</script>

<svelte:window
  onkeydown={(e) => {
    if (open && e.key === "Escape") open = false;
  }}
/>

<button
  bind:this={btn}
  type="button"
  class="tool"
  aria-haspopup="menu"
  aria-expanded={open}
  aria-label={$t("llm.central.preview.snap.button")}
  title={$t("llm.central.preview.snap.button")}
  onmousedown={(e) => e.preventDefault()}
  onclick={toggle}
>
  <Icon name="image" size={15} />
</button>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={() => (open = false)}></div>
  <div class="pop" role="menu" style:left="{pos.left}px" style:bottom="{pos.bottom}px">
    <div class="ph">
      <strong>{$t("llm.central.preview.snap.title")}</strong>
      <span class="grow"></span>
      <button type="button" class="nb" aria-label={$t("llm.central.preview.rescan")} disabled={loading} onclick={load}><Icon name="arrow-counter-clockwise" size={12} /></button>
    </div>
    {#if !permission}
      <div class="perm">
        <Icon name="shield-warning" size={14} />
        <span>{$t("llm.central.preview.snap.permission")}</span>
        <button type="button" class="button small" onclick={grant}>{$t("llm.central.preview.snap.grant")}</button>
      </div>
    {:else if note}
      <p class="note">{note}</p>
    {/if}
    {#if error}<p class="err">{error}</p>{/if}
    <div class="list">
      {#if hasPreview}
        <button type="button" role="menuitem" class="win" disabled={!!busy} onclick={() => take("__preview")}>
          <span class="ic br"><Icon name="globe" size={13} /></span>
          <span class="wt"><strong>{$t("llm.central.preview.snap.browser")}</strong></span>
          {#if busy === "__preview"}<span class="spin"><Icon name="circle-notch" size={12} /></span>{/if}
        </button>
      {/if}
      {#if loading && !windows.length}
        <p class="note">{$t("llm.central.preview.scanning")}</p>
      {/if}
      {#each windows as w (w.id)}
        <button type="button" role="menuitem" class="win" disabled={!!busy} onclick={() => take(w.id)} title={w.width ? `${Math.round(w.width)}×${Math.round(w.height)}` : undefined}>
          <span class="ic"><Icon name={w.id === "screen" ? "image" : "sidebar-simple"} size={13} /></span>
          <span class="wt">
            <strong>{label(w)}</strong>
            {#if w.id !== "screen" && w.app && w.title}<small>{w.app}</small>{/if}
          </span>
          {#if busy === w.id}<span class="spin"><Icon name="circle-notch" size={12} /></span>{/if}
        </button>
      {/each}
    </div>
  </div>
{/if}

<style>
  .tool { border: 0; background: transparent; color: var(--text-muted); padding: 6px; border-radius: 8px; cursor: pointer; display: inline-flex; align-items: center; }
  .tool:hover, .tool[aria-expanded="true"] { background: var(--fill-1); color: var(--text); }
  .scrim { position: fixed; inset: 0; z-index: 60; }
  .pop { position: fixed; z-index: 61; width: 340px; max-height: 420px; display: flex; flex-direction: column; background: var(--popup-bg, var(--surface)); border: 1px solid var(--separator); border-radius: 12px; box-shadow: 0 10px 30px rgba(0, 0, 0, 0.25); overflow: hidden; }
  .ph { display: flex; align-items: center; gap: 6px; padding: 9px 10px 7px 12px; font-size: 12.5px; }
  .grow { flex: 1; }
  .nb { border: 0; background: transparent; color: var(--text-muted); padding: 4px; border-radius: 6px; cursor: pointer; display: inline-flex; }
  .nb:hover:not(:disabled) { background: var(--fill-1); }
  .perm { display: flex; align-items: center; gap: 8px; margin: 0 10px 6px; padding: 8px; border-radius: 8px; background: var(--fill-1); font-size: 12px; color: var(--text-muted); }
  .perm span { flex: 1; }
  .note, .err { margin: 0 12px 6px; font-size: 12px; color: var(--text-muted); }
  .err { color: var(--error, var(--danger)); }
  .list { overflow: auto; padding: 2px 6px 8px; display: grid; gap: 1px; }
  .win { display: flex; align-items: center; gap: 9px; border: 0; background: transparent; color: var(--text); padding: 6px 7px; border-radius: 8px; cursor: pointer; text-align: left; }
  .win:hover:not(:disabled), .win:focus-visible { background: var(--fill-1); outline: none; }
  .win:disabled { opacity: 0.6; cursor: default; }
  .ic { width: 24px; height: 24px; border-radius: 7px; display: inline-flex; align-items: center; justify-content: center; background: var(--fill-2); color: var(--text-muted); flex-shrink: 0; }
  .ic.br { background: linear-gradient(180deg, #5aa9ff, #1e6fe8); color: #fff; }
  .wt { flex: 1; min-width: 0; display: grid; gap: 1px; }
  .wt strong { font-size: 12.5px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .wt small { font-size: 11px; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .spin { display: inline-flex; animation: rot 0.9s linear infinite; color: var(--text-muted); }
  @keyframes rot { to { transform: rotate(360deg); } }
</style>
