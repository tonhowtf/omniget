<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t, locale } from "$lib/i18n";
  import { formatBytes, formatSpeed } from "$lib/download-format";

  type Awake = { mode: string; active: boolean; until_ms: number | null };
  type Snippet = { id: number; name: string; text: string };
  type Status = {
    capabilities: { clipboard_history: boolean; volume: "apps" | "master" | "none" };
    awake: Awake;
    clipboard_enabled: boolean;
    snippets: Snippet[];
  };
  type Stats = {
    cpu_percent: number | null;
    memory_used: number;
    memory_total: number;
    disk_free: number | null;
    disk_total: number | null;
    net_down_per_s: number;
    net_up_per_s: number;
    top: { pid: number; name: string; cpu_percent: number }[];
  };
  type Target = { id: string; name: string; volume: number; muted: boolean };
  type Clip = { id: number; preview: string; pinned: boolean };
  type ClipView = { enabled: boolean; entries: Clip[] };
  type Tab = "system" | "clipboard" | "snippets";
  type Draft = { id: number | null; name: string; text: string };

  const ICON = {
    download: "M12 3v12m0 0-4-4m4 4 4-4M5 21h14",
    capture: "M4 8h3l2-3h6l2 3h3v11H4zM12 17a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z",
    dictation: "M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM5 11a7 7 0 0 0 14 0M12 18v3",
    palette: "M12.5 7.5l4 4L8 20H4v-4zM16 4a2.8 2.8 0 0 1 4 4l-2 2-4-4z",
    ports: "M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v4",
    cleaner: "M11 3l1.8 4.7 4.7 1.8-4.7 1.8L11 16l-1.8-4.7-4.7-1.8 4.7-1.8zM18 15l.9 2.1L21 18l-2.1.9L18 21l-.9-2.1L15 18l2.1-.9z",
    pin: "M9 4h6l-1 6 3 3v2H7v-2l3-3zM12 15v6",
    trash: "M5 7h14M9 7V4h6v3M7 7l1 13h8l1-13",
    edit: "M4 20h4L19 9l-4-4L4 16z",
    plus: "M12 5v14M5 12h14",
    sound: "M4 9h4l5-4v14l-5-4H4zM16.5 9a4 4 0 0 1 0 6",
    muted: "M4 9h4l5-4v14l-5-4H4zM16 9.5l5 5M21 9.5l-5 5",
  };
  const SHORTCUTS = [
    { target: "download", icon: ICON.download, label: "quick_panel.paste_link", hint: "quick_panel.paste_link_hint" },
    { target: "capture", icon: ICON.capture, label: "capture.title" },
    { target: "dictation", icon: ICON.dictation, label: "dictation.title" },
    { target: "palette", icon: ICON.palette, label: "palette.title" },
    { target: "ports", icon: ICON.ports, label: "devtools.tab_ports" },
    { target: "cleaner", icon: ICON.cleaner, label: "quick_panel.cleaner", hint: "washmybelly.nav" },
  ];
  const TABS: Tab[] = ["system", "clipboard", "snippets"];
  const AWAKE_MODES = [
    { mode: "off", label: "quick_panel.awake.off" },
    { mode: "30m", label: "quick_panel.awake.m30" },
    { mode: "1h", label: "quick_panel.awake.h1" },
    { mode: "2h", label: "quick_panel.awake.h2" },
    { mode: "on", label: "quick_panel.awake.until_off", glyph: "∞" },
  ];

  let status = $state<Status | null>(null);
  let stats = $state<Stats | null>(null);
  let targets = $state<Target[] | null>(null);
  let clips = $state<Clip[]>([]);
  let clipsOn = $state(false);
  let tab = $state<Tab>("system");
  let shown = $state(true);
  let now = $state(Date.now());
  let awakeFailed = $state(false);
  let copied = $state<string | null>(null);
  let draft = $state<Draft | null>(null);
  let draftInvalid = $state(false);

  let awake = $derived(status?.awake ?? null);
  let volumeKind = $derived(status?.capabilities.volume ?? "none");
  let memoryPct = $derived(stats && stats.memory_total > 0 ? (stats.memory_used / stats.memory_total) * 100 : 0);
  let diskPct = $derived(stats?.disk_free != null && stats.disk_total ? (1 - stats.disk_free / stats.disk_total) * 100 : 0);

  let volumeSeq = 0;
  let dragging = false;
  const queued = new Map<string, number>();
  let sending = false;
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;

  function timeLeft(until: number): string {
    const minutes = Math.max(1, Math.ceil((until - now) / 60_000));
    const unit = (value: number, name: "hour" | "minute") =>
      new Intl.NumberFormat($locale || "en", { style: "unit", unit: name, unitDisplay: "short" }).format(value);
    if (minutes < 60) return unit(minutes, "minute");
    const rest = minutes % 60;
    return rest ? `${unit(Math.floor(minutes / 60), "hour")} ${unit(rest, "minute")}` : unit(minutes / 60, "hour");
  }

  function awakeLine(a: Awake | null): string {
    if (awakeFailed) return $t("quick_panel.awake.failed");
    if (!a?.active) return $t("quick_panel.awake.off_hint");
    if (a.until_ms == null) return $t("quick_panel.awake.on_hint");
    return $t("quick_panel.awake.left", { time: timeLeft(a.until_ms) });
  }

  async function loadStatus() {
    const next = await invoke<Status>("quicktools_status").catch(() => null);
    if (!next) return;
    status = next;
    clipsOn = next.clipboard_enabled;
  }

  function applyClips(view: ClipView | null) {
    if (!view) return;
    clipsOn = view.enabled;
    clips = view.entries;
  }

  async function loadClips() {
    applyClips(await invoke<ClipView>("quicktools_clipboard").catch(() => null));
  }

  async function loadVolume() {
    if (volumeKind === "none") return;
    const seq = ++volumeSeq;
    const list = await invoke<Target[]>("quicktools_volume_list").catch(() => null);
    if (list && seq === volumeSeq && !dragging && queued.size === 0) targets = list;
  }

  async function setAwake(mode: string) {
    try {
      const next = await invoke<Awake>("quicktools_awake_set", { mode });
      awakeFailed = false;
      if (status) status.awake = next;
    } catch {
      awakeFailed = true;
    }
  }

  async function flushVolume() {
    if (sending) return;
    sending = true;
    while (queued.size > 0) {
      const [id, volume] = queued.entries().next().value as [string, number];
      queued.delete(id);
      await invoke("quicktools_volume_set", { id, volume }).catch(() => {});
    }
    sending = false;
  }

  function slide(target: Target, value: number) {
    target.volume = value;
    queued.set(target.id, value);
    void flushVolume();
  }

  function toggleMute(target: Target) {
    target.muted = !target.muted;
    volumeSeq++;
    void invoke("quicktools_volume_set", { id: target.id, muted: target.muted }).catch(() => {});
  }

  function flash(key: string) {
    copied = key;
    clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => (copied = null), 900);
  }

  async function clipAct(op: string, id?: number) {
    const view = await invoke<ClipView>("quicktools_clipboard_act", { op, id }).catch(() => null);
    applyClips(view);
    if (view && op === "copy") flash(`c${id}`);
  }

  async function toggleClips() {
    applyClips(await invoke<ClipView>("quicktools_clipboard", { enabled: !clipsOn }).catch(() => null));
  }

  async function copySnippet(id: number) {
    const ok = await invoke("quicktools_snippet_copy", { id }).then(() => true, () => false);
    if (ok) flash(`s${id}`);
  }

  async function saveDraft() {
    if (!draft) return;
    try {
      const list = await invoke<Snippet[]>("quicktools_snippet_save", { id: draft.id, name: draft.name, text: draft.text });
      if (status) status.snippets = list;
      draft = null;
      draftInvalid = false;
    } catch {
      draftInvalid = true;
    }
  }

  async function deleteSnippet(id: number) {
    const list = await invoke<Snippet[]>("quicktools_snippet_delete", { id }).catch(() => null);
    if (list && status) status.snippets = list;
  }

  function open(target: string) {
    void invoke("quicktools_open", { target }).catch(() => {});
  }

  function onTabKey(e: KeyboardEvent) {
    const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    const dir = document.documentElement.dir === "rtl" ? -step : step;
    tab = TABS[(TABS.indexOf(tab) + dir + TABS.length) % TABS.length];
    requestAnimationFrame(() => document.getElementById(`qp-tab-${tab}`)?.focus());
  }

  $effect(() => {
    if (!shown || tab !== "system") return;
    let alive = true;
    let ticks = 0;
    const tick = async () => {
      if (ticks++ % 3 === 0) void loadVolume();
      const next = await invoke<Stats>("quicktools_system_stats").catch(() => null);
      if (!alive) return;
      if (next) stats = next;
      now = Date.now();
    };
    void tick();
    const timer = setInterval(tick, 1000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  });

  onMount(() => {
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";

    const unlisteners: UnlistenFn[] = [];
    let disposed = false;
    const keep = (un: UnlistenFn) => (disposed ? un() : unlisteners.push(un));
    void listen<boolean>("quick-panel://visible", (e) => {
      shown = e.payload;
      if (!shown) return;
      now = Date.now();
      void loadStatus();
      void loadClips();
    }).then(keep);
    void listen<Awake>("quick-panel://awake", (e) => {
      if (status) status.awake = e.payload;
    }).then(keep);
    void listen("quick-panel://clipboard", () => void loadClips()).then(keep);
    void loadStatus();
    void loadClips();

    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (draft) draft = null;
      else void invoke("quicktools_close").catch(() => {});
    };
    window.addEventListener("keydown", onKey);

    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
      window.removeEventListener("keydown", onKey);
      clearTimeout(copiedTimer);
    };
  });
</script>

{#snippet icon(d: string)}
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path {d} /></svg>
{/snippet}

<main class="qp">
  <header>
    <img src="/loop.png" alt="" class="loop" />
    <h1>{$t("quick_panel.title")}</h1>
    <button type="button" class="open" onclick={() => open("app")}>{$t("usage_tray.panel.open_app")}</button>
  </header>

  <nav class="shortcuts">
    {#each SHORTCUTS as s (s.target)}
      <button type="button" title={$t(s.hint ?? s.label) as string} onclick={() => open(s.target)}>
        {@render icon(s.icon)}
        <span>{$t(s.label)}</span>
      </button>
    {/each}
  </nav>

  <div class="segmented tabs" role="tablist" tabindex="-1" onkeydown={onTabKey}>
    {#each TABS as id (id)}
      <button type="button" role="tab" id="qp-tab-{id}" class="segmented-btn" class:active={tab === id} aria-selected={tab === id} aria-controls="qp-body" tabindex={tab === id ? 0 : -1} onclick={() => (tab = id)}>
        {$t(`quick_panel.tab_${id}`)}
      </button>
    {/each}
  </div>

  <div class="body" id="qp-body" role="tabpanel" aria-labelledby="qp-tab-{tab}">
    {#if tab === "system"}
      <section>
        <div class="head">
          <h2>{$t("quick_panel.awake.title")}</h2>
          <span class="note" class:bad={awakeFailed} class:good={awake?.active && !awakeFailed}>{awakeLine(awake)}</span>
        </div>
        <div class="segmented awake" role="group" aria-label={$t("quick_panel.awake.title") as string}>
          {#each AWAKE_MODES as m (m.mode)}
            <button type="button" class="segmented-btn" class:active={(awake?.mode ?? "off") === m.mode} aria-pressed={(awake?.mode ?? "off") === m.mode} title={m.glyph ? ($t(m.label) as string) : undefined} aria-label={m.glyph ? ($t(m.label) as string) : undefined} onclick={() => setAwake(m.mode)}>
              {m.glyph ?? $t(m.label)}
            </button>
          {/each}
        </div>
      </section>

      <section>
        <div class="tiles">
          <div class="tile">
            <span class="k">{$t("quick_panel.monitor.cpu")}</span>
            <span class="v">{stats?.cpu_percent != null ? `${Math.round(stats.cpu_percent)}%` : "–"}</span>
            <span class="meter"><i style:width="{stats?.cpu_percent ?? 0}%"></i></span>
          </div>
          <div class="tile">
            <span class="k">{$t("quick_panel.monitor.memory")}</span>
            <span class="v">{stats ? formatBytes(stats.memory_used) : "–"}</span>
            <span class="meter"><i style:width="{memoryPct}%"></i></span>
          </div>
          <div class="tile">
            <span class="k">{$t("quick_panel.monitor.disk")}</span>
            <span class="v">{stats?.disk_free != null ? formatBytes(stats.disk_free) : "–"}</span>
            <span class="meter"><i style:width="{diskPct}%"></i></span>
          </div>
          <div class="tile">
            <span class="k">{$t("quick_panel.monitor.network")}</span>
            <span class="v net"><span>↓ {stats ? formatSpeed(stats.net_down_per_s) : "–"}</span><span>↑ {stats ? formatSpeed(stats.net_up_per_s) : "–"}</span></span>
          </div>
        </div>
        <h2 class="sub">{$t("quick_panel.monitor.processes")}</h2>
        <ol class="procs">
          {#each { length: 5 } as _, i (i)}
            {@const p = stats?.top[i]}
            <li>
              <span class="pname">{p?.name ?? ""}</span>
              <span class="num">{p ? `${p.cpu_percent.toFixed(1)}%` : ""}</span>
            </li>
          {/each}
        </ol>
      </section>

      {#if volumeKind !== "none"}
        <section>
          <h2>{$t(volumeKind === "apps" ? "quick_panel.volume.apps" : "quick_panel.volume.master")}</h2>
          {#if targets && targets.length === 0}
            <p class="hint">{$t(volumeKind === "apps" ? "quick_panel.volume.none" : "quick_panel.volume.no_control")}</p>
          {/if}
          <ul class="mixer">
            {#each targets ?? [] as target (target.id)}
              {@const label = target.name || ($t("quick_panel.volume.master") as string)}
              <li>
                {#if volumeKind === "apps"}<span class="app" title={label}>{label}</span>{/if}
                <input type="range" min="0" max="100" value={target.volume} aria-label={label} disabled={target.muted} onpointerdown={() => (dragging = true)} onpointerup={() => (dragging = false)} onpointercancel={() => (dragging = false)} oninput={(e) => slide(target, Number(e.currentTarget.value))} />
                <span class="num">{target.volume}</span>
                <button type="button" class="icon-btn" class:on={target.muted} aria-pressed={target.muted} title={$t(target.muted ? "quick_panel.volume.unmute" : "quick_panel.volume.mute") as string} aria-label={$t(target.muted ? "quick_panel.volume.unmute" : "quick_panel.volume.mute") as string} onclick={() => toggleMute(target)}>
                  {@render icon(target.muted ? ICON.muted : ICON.sound)}
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    {:else if tab === "clipboard"}
      <section class="fill">
        <div class="head">
          <h2 id="qp-clip-title">{$t("quick_panel.clipboard.title")}</h2>
          {#if clipsOn && clips.some((c) => !c.pinned)}
            <button type="button" class="text-btn" onclick={() => clipAct("clear")}>{$t("common.clear")}</button>
          {/if}
          <button type="button" class="switch" class:on={clipsOn} role="switch" aria-checked={clipsOn} aria-labelledby="qp-clip-title" onclick={toggleClips}><i></i></button>
        </div>
        {#if !clipsOn}
          <p class="hint">{$t("quick_panel.clipboard.off_hint")}</p>
        {:else if clips.length === 0}
          <p class="hint">{$t("quick_panel.clipboard.empty")}</p>
        {:else}
          <ul class="rows">
            {#each clips as c (c.id)}
              <li class:pinned={c.pinned}>
                <button type="button" class="row-main" onclick={() => clipAct("copy", c.id)}>
                  <span class="clip">{copied === `c${c.id}` ? $t("quick_panel.copied") : c.preview}</span>
                </button>
                <button type="button" class="icon-btn" class:on={c.pinned} aria-pressed={c.pinned} title={$t(c.pinned ? "quick_panel.clipboard.unpin" : "quick_panel.clipboard.pin") as string} aria-label={$t(c.pinned ? "quick_panel.clipboard.unpin" : "quick_panel.clipboard.pin") as string} onclick={() => clipAct(c.pinned ? "unpin" : "pin", c.id)}>
                  {@render icon(ICON.pin)}
                </button>
                <button type="button" class="icon-btn" title={$t("quick_panel.delete") as string} aria-label={$t("quick_panel.delete") as string} onclick={() => clipAct("delete", c.id)}>
                  {@render icon(ICON.trash)}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {:else}
      <section class="fill">
        <div class="head">
          <h2>{$t("quick_panel.tab_snippets")}</h2>
          {#if !draft}
            <button type="button" class="text-btn" onclick={() => (draft = { id: null, name: "", text: "" })}>
              {@render icon(ICON.plus)}
              {$t("quick_panel.snippets.add")}
            </button>
          {/if}
        </div>
        {#if draft}
          <form class="draft" onsubmit={(e) => { e.preventDefault(); void saveDraft(); }}>
            <!-- svelte-ignore a11y_autofocus -->
            <input type="text" maxlength="60" autofocus placeholder={$t("quick_panel.snippets.name") as string} aria-label={$t("quick_panel.snippets.name") as string} bind:value={draft.name} />
            <textarea rows="4" maxlength="16000" placeholder={$t("quick_panel.snippets.text") as string} aria-label={$t("quick_panel.snippets.text") as string} bind:value={draft.text}></textarea>
            <div class="draft-foot">
              <span class="note bad">{draftInvalid ? $t("quick_panel.snippets.invalid") : ""}</span>
              <button type="button" onclick={() => { draft = null; draftInvalid = false; }}>{$t("common.cancel")}</button>
              <button type="submit" class="primary">{$t("quick_panel.snippets.save")}</button>
            </div>
          </form>
        {/if}
        {#if status && status.snippets.length === 0 && !draft}
          <p class="hint">{$t("quick_panel.snippets.empty")}</p>
        {/if}
        <ul class="rows">
          {#each status?.snippets ?? [] as s (s.id)}
            <li>
              <button type="button" class="row-main" onclick={() => copySnippet(s.id)}>
                <span class="sname">{copied === `s${s.id}` ? $t("quick_panel.copied") : s.name}</span>
                <span class="clip dim">{s.text}</span>
              </button>
              <button type="button" class="icon-btn" title={$t("quick_panel.snippets.edit") as string} aria-label={$t("quick_panel.snippets.edit") as string} onclick={() => { draft = { ...s }; draftInvalid = false; }}>
                {@render icon(ICON.edit)}
              </button>
              <button type="button" class="icon-btn" title={$t("quick_panel.delete") as string} aria-label={$t("quick_panel.delete") as string} onclick={() => deleteSnippet(s.id)}>
                {@render icon(ICON.trash)}
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}
  </div>
</main>

<style>
  .qp {
    box-sizing: border-box;
    width: 100%;
    height: 100vh;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 1px var(--border-hi);
    color: var(--text);
    font: var(--text-sm) / 1.35 var(--font-body);
    user-select: none;
    -webkit-user-select: none;
    cursor: default;
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .loop {
    width: 24px;
    height: 24px;
  }
  h1 {
    flex: 1;
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
  }
  h2 {
    flex: 1;
    min-width: 0;
    margin: 0;
    font-size: var(--text-sm);
    font-weight: 600;
  }
  h2.sub {
    margin-top: 4px;
    color: var(--text-muted);
    font-size: var(--text-xs);
  }
  p {
    margin: 0;
  }
  button {
    font: inherit;
    color: inherit;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    padding: 5px 9px;
    cursor: default;
  }
  button:hover {
    background: var(--fill-2);
  }
  button:focus-visible,
  input:focus-visible,
  textarea:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 1px;
  }
  .open,
  .primary {
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 600;
  }
  .open:hover,
  .primary:hover {
    background: var(--accent-lo);
  }

  .shortcuts {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 4px;
  }
  .shortcuts button {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    height: 28px;
    padding: 0 8px;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .shortcuts button:hover {
    color: var(--text);
  }
  .shortcuts svg {
    flex: none;
  }
  .shortcuts span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tabs,
  .awake {
    display: flex;
    width: 100%;
  }
  .tabs .segmented-btn,
  .awake .segmented-btn {
    flex: 1 1 auto;
    min-width: 0;
    padding-inline: 6px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    overflow-y: auto;
    scrollbar-width: thin;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  section.fill {
    flex: 1;
    min-height: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 22px;
  }
  .note {
    flex: 0 1 auto;
    min-width: 0;
    color: var(--text-muted);
    font-size: var(--text-xs);
    text-align: end;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .note.good {
    color: var(--success);
  }
  .note.bad {
    color: var(--danger);
  }
  .hint {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .tiles {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 6px;
  }
  .tile {
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: baseline;
    gap: 4px 6px;
    min-height: 34px;
    padding: 7px 8px;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
  }
  .k {
    color: var(--text-muted);
    font-size: var(--text-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .v,
  .num {
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    white-space: nowrap;
  }
  .v.net {
    grid-column: 1 / -1;
    display: flex;
    justify-content: space-between;
    font-size: var(--text-xs);
  }
  .meter {
    grid-column: 1 / -1;
    height: 3px;
    border-radius: 2px;
    background: var(--fill-2);
    overflow: hidden;
  }
  .meter i {
    display: block;
    height: 100%;
    max-width: 100%;
    background: var(--accent);
    transition: width 0.3s linear;
  }
  .procs,
  .rows,
  .mixer {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .procs li {
    display: flex;
    gap: 8px;
    height: 18px;
    align-items: center;
    font-size: var(--text-xs);
  }
  .pname,
  .app,
  .clip,
  .sname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: start;
  }
  .procs .num {
    font-weight: 500;
    color: var(--text-muted);
  }

  .mixer {
    gap: 4px;
  }
  .mixer li {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 24px;
  }
  .app {
    flex: 0 0 96px;
    font-size: var(--text-xs);
  }
  .mixer input {
    flex: 1;
    min-width: 0;
    accent-color: var(--accent);
  }
  .mixer .num {
    width: 26px;
    text-align: end;
    font-size: var(--text-xs);
    font-weight: 500;
  }
  .icon-btn {
    flex: none;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    background: transparent;
    color: var(--text-muted);
  }
  .icon-btn.on {
    color: var(--accent-text);
  }
  .text-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 7px;
    background: transparent;
    color: var(--accent-text);
    font-size: var(--text-xs);
  }
  .switch {
    flex: none;
    width: 34px;
    height: 20px;
    padding: 2px;
    border-radius: var(--radius-full);
    background: var(--fill-3);
  }
  .switch i {
    display: block;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #fff;
    transition: transform 0.12s ease;
  }
  .switch.on {
    background: var(--accent);
  }
  .switch.on i {
    transform: translateX(14px);
  }
  :global([dir="rtl"]) .switch.on i {
    transform: translateX(-14px);
  }

  .rows {
    flex: 1;
    min-height: 0;
    gap: 2px;
    overflow-y: auto;
    scrollbar-width: thin;
  }
  .rows li {
    display: flex;
    align-items: center;
    gap: 2px;
    border-radius: var(--radius-sm);
  }
  .rows li:hover,
  .rows li:focus-within {
    background: var(--fill-1);
  }
  .row-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 6px 8px;
    background: transparent;
    text-align: start;
  }
  .row-main:hover {
    background: transparent;
  }
  .sname {
    flex: none;
    font-weight: 600;
  }
  .clip {
    flex: none;
    font-size: var(--text-xs);
  }
  .clip.dim {
    color: var(--text-muted);
  }

  .draft {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .draft input,
  .draft textarea {
    box-sizing: border-box;
    width: 100%;
    padding: 6px 8px;
    border: 1px solid var(--border-hi);
    border-radius: var(--radius-sm);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    resize: none;
    user-select: text;
    -webkit-user-select: text;
  }
  .draft-foot {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .draft-foot .note {
    flex: 1;
    text-align: start;
  }
</style>
