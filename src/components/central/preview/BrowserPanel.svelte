<script lang="ts">
  // Browser surface of a thread: address bar + a native webview anchored over
  // the empty area below it (the host moves/resizes/hides it with the tab).
  // With no page yet it lists the dev servers listening from the thread
  // folder (lsof/netstat + HTML probe). Agents drive the same page through
  // the preview_* tools.
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import Icon from "$components/central/threads/Icon.svelte";
  import { errText, lastUrl, onPreviewState, previewApi, rectOf, rememberUrl, type DevServer, type PreviewState } from "./preview-api";

  let { threadId, cwd }: { threadId: string; cwd: string | null } = $props();

  let pstate = $state<PreviewState | null>(null);
  let address = $state("");
  let editing = $state(false);
  let servers = $state<DevServer[] | null>(null);
  let scanning = $state(false);
  let showAll = $state(false);
  let listOpen = $state(false);
  let error = $state<string | null>(null);
  let host = $state<HTMLElement | null>(null);
  let destroyed = false;

  let hasPage = $derived(!!pstate && !!pstate.url && pstate.url !== "about:blank");
  let showNative = $derived(hasPage && !listOpen);
  let inWorkspace = $derived((servers ?? []).filter((s) => s.inWorkspace));
  let others = $derived((servers ?? []).filter((s) => !s.inWorkspace));

  function sync() {
    if (destroyed || !host) return;
    if (showNative) previewApi.setBounds(threadId, rectOf(host)).catch(() => {});
    else previewApi.hide(threadId).catch(() => {});
  }

  $effect(() => {
    void showNative;
    void host;
    sync();
  });

  async function scan() {
    scanning = true;
    error = null;
    try {
      servers = await previewApi.discover(cwd, showAll);
    } catch (e) {
      error = errText(e);
      servers = [];
    } finally {
      scanning = false;
    }
  }

  async function go(url: string) {
    const u = url.trim();
    if (!u || !host) return;
    error = null;
    listOpen = false;
    editing = false;
    try {
      pstate = await previewApi.open(threadId, u, rectOf(host));
      address = pstate.url;
      rememberUrl(threadId, u);
    } catch (e) {
      error = errText(e);
    }
  }

  async function nav(action: "back" | "forward" | "reload") {
    try {
      pstate = await previewApi.history(threadId, action);
    } catch (e) {
      error = errText(e);
    }
  }

  async function openOutside() {
    if (!pstate?.url) return;
    try {
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(pstate.url);
    } catch (e) {
      error = errText(e);
    }
  }

  function toggleList() {
    listOpen = !listOpen;
    if (listOpen && !servers && !scanning) void scan();
  }

  let unlisten: (() => void) | null = null;
  let ro: ResizeObserver | null = null;

  onMount(() => {
    void onPreviewState((s) => {
      if (s.threadId !== threadId) return;
      pstate = s;
      if (!editing) address = s.url === "about:blank" ? "" : s.url;
    }).then((u) => {
      if (destroyed) u();
      else unlisten = u;
    });
    ro = new ResizeObserver(() => sync());
    if (host) ro.observe(host);
    void (async () => {
      try {
        const [existing] = await previewApi.state(threadId);
        if (existing) {
          pstate = existing;
          address = existing.url === "about:blank" ? "" : existing.url;
          sync();
          return;
        }
      } catch {
        /* host without the preview commands yet */
      }
      const last = lastUrl(threadId);
      if (last) void go(last);
      else {
        listOpen = true;
        void scan();
      }
    })();
  });

  onDestroy(() => {
    destroyed = true;
    ro?.disconnect();
    unlisten?.();
    previewApi.hide(threadId).catch(() => {});
  });
</script>

<svelte:window onresize={sync} />

<div class="browser">
  <form
    class="bar"
    onsubmit={(e) => {
      e.preventDefault();
      void go(address);
    }}
  >
    <button type="button" class="nb" aria-label={$t("llm.central.preview.back")} title={$t("llm.central.preview.back")} disabled={!hasPage} onclick={() => nav("back")}>
      <span class="flip"><Icon name="caret-right" size={12} /></span>
    </button>
    <button type="button" class="nb" aria-label={$t("llm.central.preview.forward")} title={$t("llm.central.preview.forward")} disabled={!hasPage} onclick={() => nav("forward")}>
      <Icon name="caret-right" size={12} />
    </button>
    <button type="button" class="nb" aria-label={$t("llm.central.preview.reload")} title={$t("llm.central.preview.reload")} disabled={!hasPage} onclick={() => nav("reload")}>
      <Icon name="arrow-counter-clockwise" size={13} />
    </button>
    <input
      class="addr"
      type="text"
      spellcheck="false"
      autocomplete="off"
      placeholder={$t("llm.central.preview.address_placeholder")}
      aria-label={$t("llm.central.preview.address")}
      bind:value={address}
      onfocus={(e) => {
        editing = true;
        e.currentTarget.select();
      }}
      onblur={() => (editing = false)}
      onkeydown={(e) => {
        if (e.key === "Escape") {
          address = pstate?.url ?? "";
          e.currentTarget.blur();
        }
      }}
    />
    <button type="button" class="nb" class:on={listOpen} aria-label={$t("llm.central.preview.servers")} title={$t("llm.central.preview.servers")} onclick={toggleList}>
      <Icon name="plug" size={13} />
    </button>
    <button type="button" class="nb" aria-label={$t("llm.central.preview.open_external")} title={$t("llm.central.preview.open_external")} disabled={!hasPage} onclick={openOutside}>
      <Icon name="arrow-square-out" size={13} />
    </button>
  </form>
  {#if pstate?.loading && showNative}<div class="loading" aria-hidden="true"></div>{/if}
  {#if error}<p class="err" title={error}>{error}</p>{/if}

  <div class="stage" bind:this={host}>
    {#if !showNative}
      <div class="servers">
        <div class="sh">
          <h4>{$t("llm.central.preview.servers_title")}</h4>
          <span class="grow"></span>
          <label class="all"><input type="checkbox" bind:checked={showAll} onchange={scan} />{$t("llm.central.preview.show_all")}</label>
          <button type="button" class="nb" aria-label={$t("llm.central.preview.rescan")} title={$t("llm.central.preview.rescan")} disabled={scanning} onclick={scan}>
            <span class:spin={scanning}><Icon name={scanning ? "circle-notch" : "arrow-counter-clockwise"} size={13} /></span>
          </button>
          {#if hasPage}
            <button type="button" class="nb" aria-label={$t("llm.central.preview.back_to_page")} title={$t("llm.central.preview.back_to_page")} onclick={() => (listOpen = false)}><Icon name="x" size={12} /></button>
          {/if}
        </div>
        {#if !cwd}<p class="note">{$t("llm.central.preview.no_folder")}</p>{/if}
        {#if scanning && !servers}
          <p class="note">{$t("llm.central.preview.scanning")}</p>
        {:else if servers && servers.length === 0}
          <p class="note">{$t("llm.central.preview.none")}</p>
        {/if}
        {#each [{ list: inWorkspace, key: "llm.central.preview.in_folder" }, { list: others, key: "llm.central.preview.elsewhere" }] as group (group.key)}
          {#if group.list.length}
            <p class="gl">{$t(group.key)}</p>
            <ul>
              {#each group.list as s (s.port)}
                <li>
                  <button type="button" class="srv" onclick={() => go(s.url)} title={s.cwd ?? s.url}>
                    <span class="port" class:html={s.html}>:{s.port}</span>
                    <span class="meta">
                      <strong>{s.title || s.command || s.url}</strong>
                      <small>{s.command}{s.pid ? ` · pid ${s.pid}` : ""}{s.status ? ` · HTTP ${s.status}` : ""}{s.html ? "" : ` · ${$t("llm.central.preview.not_html")}`}</small>
                    </span>
                    <Icon name="caret-right" size={11} />
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        {/each}
        <p class="hint">{$t("llm.central.preview.hint")}</p>
      </div>
    {/if}
  </div>
</div>

<style>
  .browser { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .bar { display: flex; align-items: center; gap: 3px; padding: 6px 8px; border-bottom: 1px solid var(--separator); flex-shrink: 0; }
  .nb { border: 0; background: transparent; color: var(--text-muted); padding: 5px; border-radius: 6px; cursor: pointer; display: inline-flex; align-items: center; justify-content: center; }
  .nb:hover:not(:disabled) { background: var(--fill-1); color: var(--text); }
  .nb:disabled { opacity: 0.35; cursor: default; }
  .nb.on { background: var(--accent-soft); color: var(--accent-hi); }
  .flip { display: inline-flex; transform: scaleX(-1); }
  .addr { flex: 1; min-width: 0; font: inherit; font-size: 12px; padding: 5px 9px; border-radius: 7px; border: 1px solid var(--separator); background: var(--input-bg, var(--fill-1)); color: var(--text); outline: none; }
  .addr:focus { border-color: var(--accent); }
  .loading { height: 2px; background: linear-gradient(90deg, transparent, var(--accent), transparent); background-size: 50% 100%; background-repeat: no-repeat; animation: slide 1s linear infinite; flex-shrink: 0; }
  @keyframes slide { from { background-position: -50% 0; } to { background-position: 150% 0; } }
  .err { margin: 0; padding: 6px 10px; font-size: 12px; color: var(--error, var(--danger)); border-bottom: 1px solid var(--separator); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .stage { flex: 1; min-height: 0; position: relative; }
  .servers { position: absolute; inset: 0; overflow: auto; padding: 12px; box-sizing: border-box; }
  .sh { display: flex; align-items: center; gap: 6px; margin-bottom: 6px; }
  h4 { margin: 0; font-size: 11.5px; text-transform: uppercase; letter-spacing: 0.03em; color: var(--text-muted); }
  .grow { flex: 1; }
  .all { display: inline-flex; align-items: center; gap: 5px; font-size: 12px; color: var(--text-muted); cursor: pointer; }
  .gl { margin: 10px 2px 4px; font-size: 11.5px; color: var(--text-muted); font-weight: 600; }
  ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 2px; }
  .srv { width: 100%; display: flex; align-items: center; gap: 10px; border: 0; background: transparent; color: var(--text); padding: 7px 8px; border-radius: 9px; cursor: pointer; text-align: left; }
  .srv:hover, .srv:focus-visible { background: var(--fill-1); outline: none; }
  .port { font-family: var(--font-mono); font-size: 12px; font-weight: 700; padding: 3px 7px; border-radius: 6px; background: var(--fill-2); color: var(--text-muted); min-width: 44px; text-align: center; }
  .port.html { background: var(--accent-soft); color: var(--accent-hi); }
  .meta { flex: 1; min-width: 0; display: grid; gap: 1px; }
  .meta strong { font-size: 13px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta small { font-size: 11.5px; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .note, .hint { margin: 6px 2px; font-size: 12.5px; color: var(--text-muted); }
  .hint { margin-top: 14px; font-size: 11.5px; }
  .spin { display: inline-flex; animation: rot 0.9s linear infinite; }
  @keyframes rot { to { transform: rotate(360deg); } }
</style>
