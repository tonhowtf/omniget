<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import {
    EVENT_FLOOR_PROGRESS,
    EVENT_FLOORS,
    createFloor,
    discardFloor,
    errorText,
    getEnv,
    getHooks,
    listFloors,
    newHook,
    runHook,
    setEnv,
    setHooks,
    testEnv,
    getEnvProviders,
    sshHosts,
    type EnvProviders,
    type SshHost,
    type EnvRecord,
    type FloorHooks,
    type FloorProgress,
    type FloorView,
    type HookKind,
    type HookRun,
  } from "$lib/canvas/floors/api";
  import LandDialog from "./LandDialog.svelte";

  interface Props {
    workspaceId: string;
    floorId: string;
    onSwitch: (floorId: string) => void;
  }
  let { workspaceId, floorId, onSwitch }: Props = $props();

  let floors = $state<FloorView[]>([]);
  let shownFor = "";
  let panel = $state<null | "create" | "hooks" | "settings" | "delete">(null);
  let landing = $state<FloorView | null>(null);
  let progress = $state<FloorProgress | null>(null);
  let error = $state("");
  let busy = $state(false);
  let runs = $state<HookRun[]>([]);

  let name = $state("");
  let branch = $state("");
  let existingBranch = $state(false);
  let noGit = $state(false);
  let copyGround = $state(false);

  let hooks = $state<FloorHooks>({ setup: [], run: [], teardown: [], autoRunSetup: true });
  let env = $state<EnvRecord>({ kind: "local" });
  let providers = $state<EnvProviders>({ tmux: true, ssh: true, docker: true, sandbox: true, custom: true, wsl: false, runtimes: {} });
  let envStatus = $state("");
  let sshList = $state<SshHost[]>([]);

  let keepBranch = $state(true);
  let force = $state(false);

  const current = $derived(floors.find((f) => f.id === floorId) ?? null);
  const KINDS: HookKind[] = ["setup", "run", "teardown"];
  const KIND_LABEL: Record<HookKind, [string, string]> = {
    setup: ["canvas.floors.hooks.setup", "Setup"],
    run: ["canvas.floors.hooks.run", "Run"],
    teardown: ["canvas.floors.hooks.teardown", "Teardown"],
  };

  async function refresh() {
    const ws = workspaceId;
    try {
      const list = await listFloors(ws);
      if (ws === workspaceId) floors = list;
    } catch (e) {
      if (ws === workspaceId) error = errorText(e);
    }
  }

  let unlisten: UnlistenFn[] = [];
  onMount(async () => {
    unlisten.push(
      await listen<FloorProgress>(EVENT_FLOOR_PROGRESS, (e) => {
        if (e.payload.workspaceId !== workspaceId) return;
        progress = e.payload.phase === "done" || e.payload.phase === "failed" ? null : e.payload;
      }),
    );
    unlisten.push(
      await listen<{ workspaceId: string }>(EVENT_FLOORS, (e) => {
        if (e.payload.workspaceId === workspaceId) refresh();
      }),
    );
  });
  onDestroy(() => unlisten.forEach((u) => u()));

  $effect(() => {
    void workspaceId;
    if (workspaceId !== shownFor) {
      shownFor = workspaceId;
      floors = [];
    }
    refresh();
  });

  function toggle(p: typeof panel) {
    error = "";
    panel = panel === p ? null : p;
    if (panel === "hooks" || panel === "settings") loadSettings();
  }

  async function loadSettings() {
    try {
      hooks = await getHooks(workspaceId);
      env = await getEnv(workspaceId);
      providers = await getEnvProviders().catch(() => providers);
      sshList = await sshHosts().catch(() => []);
    } catch (e) {
      error = errorText(e);
    }
  }

  async function create() {
    if (!name.trim() || busy) return;
    busy = true;
    error = "";
    try {
      const r = await createFloor(workspaceId, {
        name: name.trim(),
        branch: branch.trim() || undefined,
        existingBranch,
        noGit,
        copyGround,
      });
      runs = r.setup;
      if (r.warnings?.length) error = r.warnings.join("\n");
      name = branch = "";
      panel = r.warnings?.length ? "create" : null;
      await refresh();
      onSwitch(r.floor.id);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
      progress = null;
    }
  }

  async function run(kind: HookKind, hookId?: string) {
    if (!current || busy) return;
    busy = true;
    error = "";
    try {
      runs = await runHook(workspaceId, current.id, kind, hookId);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function saveSettings() {
    busy = true;
    error = "";
    try {
      hooks = await setHooks(workspaceId, $state.snapshot(hooks));
      env = await setEnv(workspaceId, $state.snapshot(env));
      panel = null;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function checkEnv() {
    envStatus = "…";
    try {
      const r = await testEnv($state.snapshot(env));
      const bad = (r.checks ?? []).filter((c) => !c.ok || c.detail?.startsWith("warning"));
      envStatus =
        (r.ok ? "✓ " : "✗ ") +
        r.output +
        (bad.length ? " · " + bad.map((c) => `${c.name}: ${c.detail ?? ""}`).join(" · ") : "");
    } catch (e) {
      envStatus = "✗ " + errorText(e);
    }
  }

  async function remove() {
    if (!current || current.isGround || busy) return;
    busy = true;
    error = "";
    try {
      const r = await discardFloor(workspaceId, current.id, { keepBranch, force });
      runs = r.teardown ?? [];
      panel = null;
      onSwitch("ground");
      await refresh();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  function modeLabel(f: FloorView): string {
    if (f.isGround) return $t("canvas.floors.ground", { default: "Ground" }) as string;
    if (f.mode === "clone") return $t("canvas.floors.mode.clone", { default: "Copy-on-write clone" }) as string;
    if (f.mode === "worktree") return $t("canvas.floors.mode.worktree", { default: "Git worktree" }) as string;
    return $t("canvas.floors.mode.plain", { default: "Shares the ground folder" }) as string;
  }
</script>

<div class="floorbar" data-canvas-chrome>
  {#if panel || runs.length || error}
    <div class="panel" role="dialog" aria-label={$t("canvas.floors.title", { default: "Floors" })}>
      {#if panel === "create"}
        <h4>{$t("canvas.floors.new", { default: "New floor" })}</h4>
        <label>
          <span>{$t("canvas.floors.name", { default: "Name" })}</span>
          <!-- svelte-ignore a11y_autofocus -->
          <input bind:value={name} placeholder={$t("canvas.floors.namePlaceholder", { default: "Fix login bug" })} autofocus />
        </label>
        {#if !noGit}
          <label>
            <span>{$t("canvas.floors.branch", { default: "Branch" })}</span>
            <input bind:value={branch} placeholder={$t("canvas.floors.branchPlaceholder", { default: "from the name" })} />
          </label>
          <label class="check"><input type="checkbox" bind:checked={existingBranch} />{$t("canvas.floors.existingBranch", { default: "Use an existing branch" })}</label>
        {/if}
        <label class="check"><input type="checkbox" bind:checked={noGit} />{$t("canvas.floors.noGit", { default: "No git isolation (share the ground folder)" })}</label>
        <label class="check"><input type="checkbox" bind:checked={copyGround} />{$t("canvas.floors.copyGround", { default: "Clone the ground layout" })}</label>
        <div class="row">
          <button type="button" class="primary" disabled={!name.trim() || busy} onclick={create}>
            {busy ? $t("canvas.floors.creating", { default: "Creating…" }) : $t("canvas.floors.create", { default: "Create" })}
          </button>
        </div>
      {:else if panel === "hooks" && current}
        <h4>{$t("canvas.floors.hooks.title", { default: "Hooks" })} · {current.name}</h4>
        {#each KINDS as kind (kind)}
          <div class="hookgroup">
            <div class="row">
              <strong>{$t(KIND_LABEL[kind][0], { default: KIND_LABEL[kind][1] })}</strong>
              <button type="button" disabled={busy || !hooks[kind].some((h) => h.enabled)} onclick={() => run(kind)}>
                {$t("canvas.floors.hooks.runAll", { default: "Run all" })}
              </button>
            </div>
            {#each hooks[kind] as h (h.id)}
              <div class="hook">
                <code title={h.command}>{h.command}</code>
                <button type="button" class="icon" disabled={busy} title={$t("canvas.floors.hooks.runOne", { default: "Run" })} onclick={() => run(kind, h.id)}>▶</button>
              </div>
            {:else}
              <p class="muted">{$t("canvas.floors.hooks.none", { default: "No commands" })}</p>
            {/each}
          </div>
        {/each}
        <button type="button" class="link" onclick={() => toggle("settings")}>{$t("canvas.floors.hooks.configure", { default: "Configure hooks…" })}</button>
      {:else if panel === "settings"}
        <h4>{$t("canvas.floors.settings", { default: "Floor hooks and environment" })}</h4>
        <p class="muted">
          {$t("canvas.floors.hooks.vars", { default: "Variables: $OMNIGET_FLOOR_NAME, $OMNIGET_BRANCH_NAME, $OMNIGET_FLOOR_PATH, $OMNIGET_ROOT_PATH, $OMNIGET_PROJECT_NAME, $OMNIGET_FLOOR_PORT, $PORT" })}
        </p>
        {#each KINDS as kind (kind)}
          <div class="hookgroup">
            <div class="row">
              <strong>{$t(KIND_LABEL[kind][0], { default: KIND_LABEL[kind][1] })}</strong>
              <button type="button" onclick={() => (hooks[kind] = [...hooks[kind], newHook()])}>+ {$t("canvas.floors.hooks.add", { default: "Add command" })}</button>
            </div>
            {#each hooks[kind] as h, i (h.id)}
              <div class="hook edit">
                <input type="checkbox" bind:checked={h.enabled} title={$t("canvas.floors.hooks.enabled", { default: "Enabled" })} />
                <input class="mono" bind:value={h.command} placeholder="pnpm install" />
                <button type="button" class="icon" disabled={i === 0} onclick={() => {
                  const l = [...hooks[kind]];
                  [l[i - 1], l[i]] = [l[i], l[i - 1]];
                  hooks[kind] = l;
                }}>↑</button>
                <button type="button" class="icon" onclick={() => (hooks[kind] = hooks[kind].filter((x) => x.id !== h.id))}>✕</button>
              </div>
            {/each}
          </div>
        {/each}
        <label class="check"><input type="checkbox" bind:checked={hooks.autoRunSetup} />{$t("canvas.floors.hooks.autoRun", { default: "Run setup when a floor is created" })}</label>
        <h4>{$t("canvas.floors.env.title", { default: "Environment" })}</h4>
        <label>
          <span>{$t("canvas.floors.env.kind", { default: "Runs on" })}</span>
          <select bind:value={env.kind}>
            <option value="local">{$t("canvas.floors.env.local", { default: "This computer" })}</option>
            {#if providers.tmux}<option value="tmux">{$t("canvas.floors.env.tmux", { default: "This computer (tmux, persistent)" })}</option>{/if}
            {#if providers.ssh}<option value="ssh">SSH</option>{/if}
            {#if providers.docker}<option value="docker">Docker</option>{/if}
            {#if providers.sandbox}<option value="sandbox">Docker Sandbox</option>{/if}
            {#if providers.custom && Object.keys(providers.runtimes).length}<option value="custom">{$t("canvas.floors.env.custom", { default: "Custom runtime" })}</option>{/if}
            {#if providers.wsl}<option value="wsl">WSL</option>{/if}
            {#if env.kind !== "local" && (providers as unknown as Record<string, unknown>)[env.kind] !== true}
              <option value={env.kind} disabled>{env.kind} ({$t("canvas.floors.env.off", { default: "turned off" })})</option>
            {/if}
          </select>
        </label>
        {#if env.kind === "custom"}
          <label>
            <span>{$t("canvas.floors.env.runtime", { default: "Runtime" })}</span>
            <select bind:value={env.runtime}>
              {#each Object.keys(providers.runtimes) as name (name)}
                <option value={name}>{name}</option>
              {/each}
            </select>
          </label>
        {/if}
        {#if env.kind !== "local" && env.kind !== "tmux" && env.kind !== "custom"}
          <label>
            <span>{env.kind === "ssh" ? $t("canvas.floors.env.host", { default: "Host (user@server)" }) : env.kind === "docker" ? $t("canvas.floors.env.container", { default: "Container" }) : $t("canvas.floors.env.distro", { default: "Distro" })}</span>
            <input bind:value={env.host} list={env.kind === "ssh" ? "omniget-ssh-hosts" : undefined} />
          </label>
          {#if env.kind === "ssh" && sshList.length}
            <datalist id="omniget-ssh-hosts">
              {#each sshList as h (h.host)}
                <option value={h.host}>{[h.user, h.hostName].filter(Boolean).join("@")}{h.port ? `:${h.port}` : ""}{h.proxyJump ? ` via ${h.proxyJump}` : ""}</option>
              {/each}
            </datalist>
            <label>
              <span>{$t("canvas.floors.env.sshConfig", { default: "From ~/.ssh/config" })}</span>
              <select value="" onchange={(e) => { const v = (e.currentTarget as HTMLSelectElement).value; if (v) env.host = v; }}>
                <option value="">{$t("canvas.floors.env.sshPick", { default: "Pick a host…" })}</option>
                {#each sshList as h (h.host)}
                  <option value={h.host}>{h.host}{h.hostName ? ` (${h.hostName})` : ""}</option>
                {/each}
              </select>
            </label>
          {/if}
        {/if}
        <label>
          <span>{$t("canvas.floors.env.dir", { default: "Folder there" })}</span>
          <input class="mono" bind:value={env.dir} />
        </label>
        {#if env.kind === "ssh"}
          <label>
            <span>{$t("canvas.floors.env.sshOptions", { default: "SSH options" })}</span>
            <input class="mono" bind:value={env.sshOptions} placeholder="-p 2222" />
          </label>
        {/if}
        {#if env.kind === "docker" || env.kind === "custom" || env.kind === "wsl"}
          <label>
            <span>{$t("canvas.floors.env.user", { default: "User" })}</span>
            <input bind:value={env.user} />
          </label>
        {/if}
        {#if env.kind === "ssh" || env.kind === "docker" || env.kind === "sandbox" || env.kind === "custom"}
          <label class="check"><input type="checkbox" bind:checked={env.persistent} />{$t("canvas.floors.env.persistent", { default: "Persistent session (tmux there, survives closing the app)" })}</label>
        {/if}
        <div class="row">
          <button type="button" onclick={checkEnv}>{$t("canvas.floors.env.test", { default: "Test" })}</button>
          <span class="muted small">{envStatus}</span>
        </div>
        <div class="row">
          <button type="button" class="primary" disabled={busy} onclick={saveSettings}>{$t("canvas.floors.save", { default: "Save" })}</button>
        </div>
      {:else if panel === "delete" && current}
        <h4>{$t("canvas.floors.delete.title", { default: "Delete floor" })} · {current.name}</h4>
        <p class="muted">{$t("canvas.floors.delete.body", { default: "The floor's folder and its terminals go away. Teardown hooks run first." })}</p>
        {#if current.branch}
          <label class="check"><input type="checkbox" bind:checked={keepBranch} />{$t("canvas.floors.delete.keepBranch", { default: "Keep the branch" })} <code>{current.branch}</code></label>
        {/if}
        <label class="check"><input type="checkbox" bind:checked={force} />{$t("canvas.floors.delete.force", { default: "Delete even with uncommitted changes" })}</label>
        <div class="row">
          <button type="button" class="danger" disabled={busy} onclick={remove}>{$t("canvas.floors.delete.confirm", { default: "Delete floor" })}</button>
        </div>
      {/if}
      {#if error}
        <p class="error">{error}</p>
      {/if}
      {#if runs.length}
        <div class="runs">
          {#each runs as r (r.id + r.durationMs)}
            <details open={!r.ok}>
              <summary class:bad={!r.ok}>{r.ok ? "✓" : "✗"} <code>{r.command}</code> <span class="muted">{(r.durationMs / 1000).toFixed(1)} s</span></summary>
              <pre>{r.output || "—"}</pre>
            </details>
          {/each}
          <button type="button" class="link" onclick={() => (runs = [])}>{$t("canvas.floors.hooks.clear", { default: "Clear" })}</button>
        </div>
      {/if}
    </div>
  {/if}

  <div class="bar" role="toolbar" tabindex="-1" aria-label={$t("canvas.floors.title", { default: "Floors" })}>
    {#each floors as f (f.id)}
      <button
        type="button"
        class="chip"
        class:on={f.id === floorId}
        class:missing={f.missing}
        style:--fc={f.color ?? "var(--accent)"}
        title={`${modeLabel(f)}${f.branch ? ` · ${f.branch}` : ""}${f.port ? ` · :${f.port}` : ""}${f.path ? `\n${f.path}` : ""}`}
        onclick={() => onSwitch(f.id)}
      >
        <span class="dot"></span>
        {f.isGround ? $t("canvas.floors.ground", { default: "Ground" }) : f.name}
        {#if f.ahead}<span class="badge" title={$t("canvas.floors.ahead", { default: "Commits to land" })}>↑{f.ahead}</span>{/if}
        {#if f.dirty}<span class="badge warn" title={$t("canvas.floors.dirty", { default: "Uncommitted files" })}>●{f.dirty}</span>{/if}
      </button>
    {/each}
    {#if progress}
      <span class="pill" title={progress.message ?? ""}>
        <span class="spin"></span>{progress.name} · {$t(`canvas.floors.phase.${progress.phase}`, { default: progress.phase })}
      </span>
    {/if}
    <span class="sep"></span>
    <button type="button" class="icon" title={$t("canvas.floors.new", { default: "New floor" })} onclick={() => toggle("create")}>＋</button>
    <button type="button" class="icon" title={$t("canvas.floors.hooks.title", { default: "Hooks" })} onclick={() => toggle("hooks")}>⚡</button>
    <button type="button" class="icon" title={$t("canvas.floors.settings", { default: "Floor hooks and environment" })} onclick={() => toggle("settings")}>⚙</button>
    {#if current && !current.isGround}
      {#if current.mode !== "plain"}
        <button type="button" class="chip land" onclick={() => (landing = current)}>✈ {$t("canvas.floors.land", { default: "Land" })}</button>
      {/if}
      <button type="button" class="icon" title={$t("canvas.floors.delete.title", { default: "Delete floor" })} onclick={() => toggle("delete")}>✕</button>
    {/if}
  </div>
</div>

{#if landing}
  <LandDialog
    {workspaceId}
    floor={landing}
    onClose={() => (landing = null)}
    onLanded={(o) => {
      landing = null;
      runs = o.teardown ?? [];
      if (o.floorRemoved) onSwitch("ground");
      refresh();
    }}
  />
{/if}

<style>
  .floorbar {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
    max-width: calc(100vw - 32px);
  }
  .bar,
  .panel {
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.22);
    color: var(--text);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px;
    overflow-x: auto;
    max-width: 100%;
  }
  .panel {
    width: min(380px, calc(100vw - 32px));
    max-height: 60vh;
    overflow: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 13px;
  }
  h4 {
    margin: 4px 0 0;
    font-size: 13px;
  }
  button {
    font: inherit;
    color: inherit;
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 4px 10px;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  button:hover:not(:disabled) {
    background: var(--hover);
  }
  .icon {
    border: none;
    min-width: 28px;
    height: 28px;
    padding: 0 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border-color: transparent;
    white-space: nowrap;
  }
  .chip.on {
    border-color: var(--fc);
    background: color-mix(in srgb, var(--fc) 14%, transparent);
  }
  .chip.missing {
    opacity: 0.55;
    text-decoration: line-through;
  }
  .chip.land {
    border-color: var(--accent);
    color: var(--accent);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--fc);
  }
  .badge {
    font-size: 11px;
    color: var(--text-muted);
  }
  .badge.warn {
    color: var(--danger);
  }
  .sep {
    width: 1px;
    align-self: stretch;
    background: var(--border);
    margin: 0 2px;
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-muted);
    padding: 0 8px;
    white-space: nowrap;
  }
  .spin {
    width: 10px;
    height: 10px;
    border: 2px solid var(--accent);
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  label > span {
    color: var(--text-muted);
    font-size: 12px;
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  input:not([type="checkbox"]),
  select {
    font: inherit;
    color: var(--text);
    background: var(--input-bg, transparent);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 5px 8px;
    min-width: 0;
  }
  .mono,
  code,
  pre {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    justify-content: space-between;
  }
  .primary {
    background: var(--accent);
    color: var(--on-accent, #fff);
    border-color: var(--accent);
  }
  .danger {
    color: var(--danger);
    border-color: var(--danger);
  }
  .link {
    border: none;
    padding: 0;
    color: var(--accent);
    align-self: flex-start;
  }
  .hookgroup {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .hook {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .hook code {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hook.edit .mono {
    flex: 1;
  }
  .muted {
    color: var(--text-muted);
    margin: 0;
  }
  .small {
    font-size: 12px;
    overflow-wrap: anywhere;
  }
  .error {
    color: var(--danger);
    white-space: pre-wrap;
    margin: 0;
  }
  .runs pre {
    max-height: 160px;
    overflow: auto;
    margin: 4px 0;
    padding: 6px;
    background: var(--input-bg, rgba(0, 0, 0, 0.2));
    border-radius: 6px;
    white-space: pre-wrap;
  }
  summary.bad {
    color: var(--danger);
  }
</style>
