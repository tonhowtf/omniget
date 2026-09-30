<script lang="ts">
  /**
   * Settings → World → Canvas, second half: which environments terminals
   * may run in (switches and custom runtimes), the persistent sessions they
   * left running here and on remote machines ("Processes"), where the
   * canvas skill is installed per agent, "Private network only" for phone
   * access, and Report a Problem (a local diagnostics file; nothing is sent).
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { canvasPrefs, replaceCanvasPrefs, type CanvasPrefs } from "$lib/canvas/prefs.svelte";
  import {
    envSessionKill,
    envSessions,
    errorText,
    getEnvProviders,
    setEnvProviders,
    type CustomRuntime,
    type EnvProviders,
    type EnvSession,
  } from "$lib/canvas/floors/api";

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) }) as string;

  const KINDS: Array<{ k: keyof Omit<EnvProviders, "runtimes">; label: string }> = [
    { k: "tmux", label: "This computer, persistent (tmux)" },
    { k: "ssh", label: "SSH" },
    { k: "docker", label: "Docker" },
    { k: "sandbox", label: "Docker Sandbox" },
    { k: "custom", label: "Custom runtimes" },
    { k: "wsl", label: "WSL" },
  ];
  const AGENTS = ["claude", "codex", "gemini", "opencode"] as const;
  const SKILL_HINT: Record<string, string> = {
    claude: "~/.claude/skills",
    codex: "~/.codex/skills",
    gemini: "~/.gemini/skills",
    opencode: "~/.config/opencode/skills",
  };

  let providers = $state<EnvProviders | null>(null);
  let sessions = $state<EnvSession[] | null>(null);
  let sessionsBusy = $state(false);
  let busy = $state(false);
  const EMPTY_RT = { name: "", exec: "", args: "", interactive: "", shell: "", bridgeHost: "" };
  let rt = $state({ ...EMPTY_RT });

  type SkillsInfo = { pluginDir?: string; agents: Record<string, { root?: string | null; installed?: string | null }> };
  let skills = $state<SkillsInfo | null>(null);
  const prefs = $derived(canvasPrefs());
  let roots = $state<Record<string, string>>({});

  type RemoteStatus = { running: boolean; privateOnly: boolean; lastRefused?: { ip: string; atMs: number } | null };
  let remote = $state<RemoteStatus | null>(null);

  type Report = { path: string; shownPath: string; files: string[]; logLines: number; mailto: string; issueUrl: string };
  let report = $state<Report | null>(null);

  onMount(() => {
    void load();
  });

  async function load() {
    try {
      providers = await getEnvProviders();
    } catch (e) {
      showToast("error", errorText(e));
    }
    roots = { ...(prefs.skills?.roots ?? {}) };
    skills = await invoke<SkillsInfo>("canvas_skills_info").catch(() => null);
    remote = await invoke<RemoteStatus>("remote_status").catch(() => null);
    void loadSessions();
  }

  async function loadSessions() {
    sessionsBusy = true;
    try {
      sessions = await envSessions();
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      sessionsBusy = false;
    }
  }

  async function toggle(k: keyof Omit<EnvProviders, "runtimes">) {
    if (!providers || busy) return;
    busy = true;
    try {
      providers = await setEnvProviders({ [k]: !providers[k] });
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      busy = false;
    }
  }

  async function addRuntime() {
    const name = rt.name.trim();
    if (!name || !rt.exec.trim()) {
      showToast("error", tr("canvas.envSettings.runtimeNeeds", "A runtime needs a name and an executable."));
      return;
    }
    const def: CustomRuntime = { exec: rt.exec.trim(), args: rt.args.trim(), interactive: rt.interactive.trim() };
    if (rt.shell.trim()) def.shell = rt.shell.trim();
    if (rt.bridgeHost.trim()) def.bridgeHost = rt.bridgeHost.trim();
    busy = true;
    try {
      providers = await setEnvProviders({ runtimes: { [name]: def } });
      rt = { ...EMPTY_RT };
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      busy = false;
    }
  }

  async function removeRuntime(name: string) {
    busy = true;
    try {
      providers = await setEnvProviders({ runtimes: { [name]: null } });
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      busy = false;
    }
  }

  async function endSession(s: EnvSession) {
    if (!s.session) return;
    try {
      await envSessionKill(s.session, s.env);
      showToast("success", tr("canvas.envSettings.sessionEnded", "Session ended. The terminal stays on the canvas."));
    } catch (e) {
      showToast("error", errorText(e));
    }
    void loadSessions();
  }

  async function saveRoots(install: boolean) {
    busy = true;
    try {
      const next = structuredClone($state.snapshot(prefs)) as CanvasPrefs;
      next.skills = { roots: { ...(next.skills?.roots ?? {}), ...roots } };
      await replaceCanvasPrefs(next);
      skills = await invoke<SkillsInfo>("canvas_skills_info", { install });
      if (install) showToast("success", tr("canvas.envSettings.skillsInstalled", "Skill installed in the folders that are set."));
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      busy = false;
    }
  }

  async function setPrivateOnly(on: boolean) {
    try {
      remote = await invoke<RemoteStatus>("remote_private_only_set", { on });
    } catch (e) {
      showToast("error", errorText(e));
    }
  }

  async function makeReport() {
    busy = true;
    try {
      report = await invoke<Report>("canvas_report_problem", { reveal: true });
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      busy = false;
    }
  }

  async function openLink(url: string) {
    try {
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(url);
    } catch (e) {
      showToast("error", errorText(e));
    }
  }

  const ago = (s?: number) => (s ? new Date(s * 1000).toLocaleString() : "");
</script>

<section class="settings-section" id="settings-section-canvas-environments">
  <h5 class="section-title">{tr("canvas.envSettings.title", "Environments")}</h5>
  <p class="intro hint">{tr("canvas.envSettings.intro", "Where terminals may run. A kind that is off runs on this computer instead.")}</p>
  <div class="card">
    <div class="setting-row">
      <div class="setting-col"><span class="setting-label">{tr("canvas.envSettings.local", "This computer")}</span><span class="setting-path">{tr("canvas.envSettings.alwaysOn", "Always on")}</span></div>
      <button class="toggle on" disabled role="switch" aria-checked="true" aria-label={tr("canvas.envSettings.local", "This computer")}><span class="toggle-knob"></span></button>
    </div>
    {#if providers}
      {#each KINDS as row (row.k)}
        {@const label = tr(`canvas.envSettings.kind.${row.k}`, row.label)}
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col"><span class="setting-label">{label}</span></div>
          <button class="toggle" class:on={providers[row.k]} disabled={busy} role="switch" aria-checked={providers[row.k]} aria-label={label}
            data-env-switch={row.k} onclick={() => toggle(row.k)}><span class="toggle-knob"></span></button>
        </div>
      {/each}
    {/if}
  </div>

  {#if providers?.custom}
    <div class="card">
      <div class="setting-row"><span class="setting-label">{tr("canvas.envSettings.runtimes", "Custom runtimes")}</span></div>
      {#each Object.entries(providers.runtimes) as [name, def] (name)}
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{name}</span>
            <span class="setting-path mono">{def.exec} {def.interactive}</span>
          </div>
          <button class="button" disabled={busy} onclick={() => removeRuntime(name)}>{tr("canvas.envSettings.remove", "Remove")}</button>
        </div>
      {/each}
      <div class="divider"></div>
      <div class="editor">
        <div class="pair">
          <label>{tr("canvas.envSettings.rtName", "Name")}<input class="input-text" bind:value={rt.name} placeholder="podman" /></label>
          <label>{tr("canvas.envSettings.rtExec", "Executable")}<input class="input-text mono" bind:value={rt.exec} placeholder="podman" /></label>
        </div>
        <label>{tr("canvas.envSettings.rtArgs", "Arguments to run a command (Test, setup)")}<input class="input-text mono" bind:value={rt.args} placeholder="exec -i my-box" /></label>
        <label>{tr("canvas.envSettings.rtInteractive", "Arguments for the terminal")}<input class="input-text mono" bind:value={rt.interactive} placeholder="exec -i -t my-box" /></label>
        <div class="pair">
          <label>{tr("canvas.envSettings.rtShell", "Shell inside")}<input class="input-text mono" bind:value={rt.shell} placeholder="/bin/sh" /></label>
          <label>{tr("canvas.envSettings.rtBridge", "How it reaches this computer")}<input class="input-text mono" bind:value={rt.bridgeHost} placeholder="host.containers.internal" /></label>
        </div>
        <div class="pair end"><button class="button primary" disabled={busy} onclick={addRuntime}>{tr("canvas.envSettings.addRuntime", "Add runtime")}</button></div>
      </div>
    </div>
  {/if}
</section>

<section class="settings-section" id="settings-section-canvas-processes">
  <h5 class="section-title">{tr("canvas.envSettings.processes", "Persistent sessions")}</h5>
  <p class="intro hint">{tr("canvas.envSettings.processesIntro", "Terminal sessions kept in tmux here and on SSH, Docker and runtime machines. Ending one stops its programs; the terminal stays on the canvas and starts fresh.")}</p>
  <div class="card">
    {#if sessions === null}
      <div class="setting-row"><span class="setting-path">…</span></div>
    {:else if !sessions.length}
      <div class="setting-row"><span class="setting-path">{tr("canvas.envSettings.noSessions", "No persistent sessions.")}</span></div>
    {:else}
      {#each sessions as s, i (s.env + (s.session ?? "") + i)}
        {#if i > 0}<div class="divider"></div>{/if}
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label mono">{s.session ?? s.env}</span>
            {#if s.error}
              <span class="setting-path danger">{s.env}: {s.error}</span>
            {:else}
              <span class="setting-path">{s.env} · {s.command}{s.attached ? " · " + tr("canvas.envSettings.attached", "attached") : ""}{s.createdS ? " · " + ago(s.createdS) : ""}</span>
            {/if}
          </div>
          {#if s.session}<button class="button" onclick={() => endSession(s)}>{tr("canvas.envSettings.end", "End")}</button>{/if}
        </div>
      {/each}
    {/if}
    <div class="divider"></div>
    <div class="pair end"><button class="button" disabled={sessionsBusy} onclick={loadSessions}>{tr("canvas.envSettings.refresh", "Refresh")}</button></div>
  </div>
</section>

<section class="settings-section" id="settings-section-canvas-skills">
  <h5 class="section-title">{tr("canvas.envSettings.skills", "Skills")}</h5>
  <p class="intro hint">
    {tr("canvas.envSettings.skillsIntro", "Canvas terminals always load the OmniGet canvas skill from the app's plugin folder. Set a folder to also install it there for each agent.")}
    {#if skills?.pluginDir}<br /><span class="mono">{skills.pluginDir}</span>{/if}
  </p>
  <div class="card">
    {#each AGENTS as a, i (a)}
      {#if i > 0}<div class="divider"></div>{/if}
      <div class="setting-row">
        <div class="setting-col">
          <label class="setting-label" for="sk-{a}">{a}</label>
          {#if skills?.agents?.[a]?.installed}<span class="setting-path mono">{skills.agents[a].installed}</span>{/if}
        </div>
        <input id="sk-{a}" class="input-text mono root" placeholder={SKILL_HINT[a]} bind:value={roots[a]} />
      </div>
    {/each}
    <div class="divider"></div>
    <div class="pair end">
      <button class="button" disabled={busy} onclick={() => saveRoots(false)}>{tr("canvas.envSettings.save", "Save")}</button>
      <button class="button primary" disabled={busy} onclick={() => saveRoots(true)}>{tr("canvas.envSettings.installNow", "Save and install now")}</button>
    </div>
  </div>
</section>

<section class="settings-section" id="settings-section-canvas-privacy">
  <h5 class="section-title">{tr("canvas.envSettings.remoteTitle", "Phone access")}</h5>
  <div class="card">
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{tr("canvas.envSettings.privateOnly", "Private network only")}</span>
        <span class="setting-path">{tr("canvas.envSettings.privateOnlyHint", "Only this computer, the local network, Tailscale and link-local devices get in; anything from the internet is refused.")}</span>
        {#if remote?.lastRefused}<span class="setting-path">{tr("canvas.envSettings.lastRefused", "Last refused: {{ip}}", { ip: remote.lastRefused.ip })}</span>{/if}
      </div>
      <button class="toggle" class:on={remote?.privateOnly} role="switch" aria-checked={!!remote?.privateOnly} aria-label={tr("canvas.envSettings.privateOnly", "Private network only")}
        onclick={() => setPrivateOnly(!remote?.privateOnly)}><span class="toggle-knob"></span></button>
    </div>
  </div>
</section>

<section class="settings-section" id="settings-section-canvas-report">
  <h5 class="section-title">{tr("canvas.envSettings.reportTitle", "Report a problem")}</h5>
  <p class="intro hint">{tr("canvas.envSettings.reportIntro", "Saves a diagnostics file on this computer: versions, counts, recent app log lines and crash reports. No code, notes, prompts or terminal output; tokens, e-mails and your home folder are masked. Nothing is sent: attach it yourself.")}</p>
  <div class="card">
    <div class="pair end"><button class="button primary" disabled={busy} onclick={makeReport}>{tr("canvas.envSettings.reportMake", "Create diagnostics file")}</button></div>
    {#if report}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label mono">{report.shownPath}</span>
          <span class="setting-path">{tr("canvas.envSettings.reportFiles", "{{n}} files, {{lines}} log lines", { n: report.files.length, lines: report.logLines })}</span>
        </div>
      </div>
      <div class="pair end">
        <button class="button" onclick={() => openLink(report!.mailto)}>{tr("canvas.envSettings.reportMail", "Write an e-mail")}</button>
        <button class="button" onclick={() => openLink(report!.issueUrl)}>{tr("canvas.envSettings.reportIssue", "Open a GitHub issue")}</button>
      </div>
    {/if}
  </div>
</section>

<style>
  .pair {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .pair.end {
    justify-content: flex-end;
    padding: 6px 0;
  }
  .mono {
    font-family: var(--font-mono, monospace);
    overflow-wrap: anywhere;
    min-width: 0;
  }
  .setting-col {
    min-width: 0;
  }
  .intro {
    margin: 4px 0 10px;
  }
  .hint {
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: var(--leading-sm);
  }
  .root {
    width: min(320px, 50vw);
  }
  .editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 8px;
  }
  .editor label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-muted);
    flex: 1;
  }
  .danger {
    color: var(--error, #ff453a);
  }
</style>
