<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { canvasPrefs } from "$lib/canvas/prefs.svelte";
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
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingsEmptyState from "./rebrand/SettingsEmptyState.svelte";
  import Subgroup from "./ai/Subgroup.svelte";
  import { visibleEnvKinds, type EnvKind } from "./ai/ai-settings";
  import { canvasSaving, saveCanvas } from "./ai/canvas-save.svelte";

  const AGENTS = ["claude", "codex", "gemini", "opencode"] as const;
  const SKILL_HINT: Record<string, string> = {
    claude: "~/.claude/skills",
    codex: "~/.codex/skills",
    gemini: "~/.gemini/skills",
    opencode: "~/.config/opencode/skills",
  };
  const onWindows = typeof navigator !== "undefined" && /Win/i.test(navigator.platform || navigator.userAgent);

  let providers = $state<EnvProviders | null>(null);
  let providersFailed = $state(false);
  let sessions = $state<EnvSession[] | null>(null);
  let sessionsFailed = $state(false);
  let sessionsBusy = $state(false);
  let busy = $state(false);
  const EMPTY_RT = { name: "", exec: "", args: "", interactive: "", shell: "", bridgeHost: "" };
  let rt = $state({ ...EMPTY_RT });
  let rtMissing = $state(false);

  type SkillsInfo = { pluginDir?: string; agents: Record<string, { root?: string | null; installed?: string | null }> };
  let skills = $state<SkillsInfo | null>(null);
  const prefs = $derived(canvasPrefs());
  let roots = $state<Record<string, string>>({});
  let rootsEdited = $state(false);
  let rootsBusy = $state(false);

  $effect(() => {
    const saved = prefs.skills?.roots ?? {};
    if (!rootsEdited) roots = { ...saved };
  });

  type RemoteStatus = { running: boolean; privateOnly: boolean; lastRefused?: { ip: string; atMs: number } | null };
  let remote = $state<RemoteStatus | null>(null);
  let remoteFailed = $state(false);

  const kinds = $derived(visibleEnvKinds(onWindows, providers));

  onMount(() => {
    void load();
  });

  async function loadProviders() {
    providersFailed = false;
    try {
      providers = await getEnvProviders();
    } catch {
      providersFailed = true;
    }
  }

  async function loadRemote() {
    remoteFailed = false;
    try {
      remote = await invoke<RemoteStatus>("remote_status");
    } catch {
      remote = null;
      remoteFailed = true;
    }
  }

  async function load() {
    await Promise.all([
      loadProviders(),
      loadRemote(),
      invoke<SkillsInfo>("canvas_skills_info").then((s) => (skills = s)).catch(() => (skills = null)),
      loadSessions(),
    ]);
  }

  async function loadSessions() {
    sessionsBusy = true;
    sessionsFailed = false;
    try {
      sessions = await envSessions();
    } catch {
      sessionsFailed = true;
    } finally {
      sessionsBusy = false;
    }
  }

  async function toggle(k: EnvKind) {
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
      rtMissing = true;
      return;
    }
    rtMissing = false;
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
      showToast("success", $t("canvas.envSettings.sessionEnded") as string);
    } catch (e) {
      showToast("error", errorText(e));
    }
    void loadSessions();
  }

  async function saveRoots(install: boolean) {
    if (rootsBusy || canvasSaving.busy) return;
    rootsBusy = true;
    try {
      const edited = $state.snapshot(roots);
      const ok = await saveCanvas((c) => {
        c.skills = { roots: { ...(c.skills?.roots ?? {}), ...edited } };
      });
      if (!ok) return;
      rootsEdited = false;
      skills = await invoke<SkillsInfo>("canvas_skills_info", { install });
      showToast("success", (install ? $t("canvas.envSettings.skillsInstalled") : $t("settings.rebrand.state.saved")) as string);
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      rootsBusy = false;
    }
  }

  async function setPrivateOnly(on: boolean) {
    try {
      remote = await invoke<RemoteStatus>("remote_private_only_set", { on });
    } catch (e) {
      showToast("error", errorText(e));
    }
  }

  const ago = (s?: number) => (s ? new Date(s * 1000).toLocaleString() : "");
</script>

<Subgroup id="canvas-environments" title={$t("settings.ai.env_title") as string} intro={$t("canvas.envSettings.intro") as string} settingKey="canvas.envSettings.title">
  {#if providersFailed}
    <SettingsEmptyState error title={$t("settings.rebrand.state.unavailable") as string} onRetry={loadProviders} />
  {:else if !providers}
    <p class="state" role="status">{$t("settings.rebrand.state.loading")}</p>
  {:else}
    {#each kinds as k (k)}
      <SettingRow id={`canvas-env-${k}`} label={$t(`canvas.envSettings.kind.${k}`)}>
        {#snippet control({ id, describedBy })}
          <button {id} type="button" class="toggle" class:on={providers![k]} role="switch" aria-checked={providers![k]} aria-labelledby={`${id}-label`} aria-describedby={describedBy}
            disabled={busy} data-env-switch={k} onclick={() => toggle(k)}><span class="toggle-knob"></span></button>
        {/snippet}
      </SettingRow>
    {/each}

    {#if providers.custom}
      <div class="runtimes">
        <span class="label">{$t("canvas.envSettings.runtimes")}</span>
        {#each Object.entries(providers.runtimes) as [name, def] (name)}
          <div class="item">
            <span class="item-text"><bdi>{name}</bdi><small class="mono"><bdi>{def.exec} {def.interactive}</bdi></small></span>
            <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => removeRuntime(name)}>{$t("canvas.envSettings.remove")}</button>
          </div>
        {/each}
        <form class="editor" onsubmit={(e) => { e.preventDefault(); void addRuntime(); }}>
          <div class="pair">
            <label>{$t("canvas.envSettings.rtName")}<input class="input-text" bind:value={rt.name} placeholder="podman" aria-invalid={rtMissing && !rt.name.trim()} /></label>
            <label>{$t("canvas.envSettings.rtExec")}<input class="input-text mono" bind:value={rt.exec} placeholder="podman" aria-invalid={rtMissing && !rt.exec.trim()} /></label>
          </div>
          <label>{$t("canvas.envSettings.rtArgs")}<input class="input-text mono" bind:value={rt.args} placeholder="exec -i my-box" /></label>
          <label>{$t("canvas.envSettings.rtInteractive")}<input class="input-text mono" bind:value={rt.interactive} placeholder="exec -i -t my-box" /></label>
          <div class="pair">
            <label>{$t("canvas.envSettings.rtShell")}<input class="input-text mono" bind:value={rt.shell} placeholder="/bin/sh" /></label>
            <label>{$t("canvas.envSettings.rtBridge")}<input class="input-text mono" bind:value={rt.bridgeHost} placeholder="host.containers.internal" /></label>
          </div>
          {#if rtMissing}<p class="error" role="alert"><bdi>{$t("canvas.envSettings.runtimeNeeds")}</bdi></p>{/if}
          <div class="buttons"><button type="submit" class="btn btn-primary btn-sm" disabled={busy}>{$t("canvas.envSettings.addRuntime")}</button></div>
        </form>
      </div>
    {/if}
  {/if}
</Subgroup>

<Subgroup id="canvas-processes" title={$t("canvas.envSettings.processes") as string} intro={$t("canvas.envSettings.processesIntro") as string} settingKey="canvas.envSettings.processes">
  {#if sessionsFailed}
    <SettingsEmptyState error title={$t("settings.rebrand.state.unavailable") as string} onRetry={loadSessions} />
  {:else if sessions === null}
    <p class="state" role="status">{$t("settings.rebrand.state.loading")}</p>
  {:else if !sessions.length}
    <p class="state">{$t("canvas.envSettings.noSessions")}</p>
  {:else}
    {#each sessions as s, i (s.env + (s.session ?? "") + i)}
      <div class="item">
        <span class="item-text">
          <bdi class="mono">{s.session ?? s.env}</bdi>
          {#if s.error}
            <small class="danger"><bdi>{s.env}: {s.error}</bdi></small>
          {:else}
            <small><bdi>{s.env} · {s.command}{s.attached ? " · " + $t("canvas.envSettings.attached") : ""}{s.createdS ? " · " + ago(s.createdS) : ""}</bdi></small>
          {/if}
        </span>
        {#if s.session}<button type="button" class="btn btn-secondary btn-sm" onclick={() => endSession(s)}>{$t("canvas.envSettings.end")}</button>{/if}
      </div>
    {/each}
  {/if}
  <div class="buttons"><button type="button" class="btn btn-secondary btn-sm" disabled={sessionsBusy} onclick={loadSessions}>{$t("canvas.envSettings.refresh")}</button></div>
</Subgroup>

<Subgroup id="canvas-skills" title={$t("settings.ai.skills_title") as string} intro={$t("canvas.envSettings.skillsIntro") as string} settingKey="canvas.envSettings.skills">
  {#if skills?.pluginDir}<p class="state mono"><bdi>{skills.pluginDir}</bdi></p>{/if}
  {#each AGENTS as a (a)}
    <SettingRow id={`sk-${a}`} label={$t(`canvas.term.agent.${a}`, { default: a })} help={skills?.agents?.[a]?.installed ?? undefined}>
      {#snippet control({ id, describedBy })}
        <input {id} class="input-text mono" placeholder={SKILL_HINT[a]} aria-describedby={describedBy} bind:value={roots[a]} oninput={() => (rootsEdited = true)} />
      {/snippet}
    </SettingRow>
  {/each}
  <div class="buttons">
    <button type="button" class="btn btn-secondary btn-sm" disabled={rootsBusy || canvasSaving.busy} onclick={() => saveRoots(false)}>{$t("canvas.envSettings.save")}</button>
    <button type="button" class="btn btn-primary btn-sm" disabled={rootsBusy || canvasSaving.busy} onclick={() => saveRoots(true)}>{$t("canvas.envSettings.installNow")}</button>
  </div>
</Subgroup>

<Subgroup id="canvas-privacy" title={$t("canvas.envSettings.remoteTitle") as string}>
  <SettingRow id="canvas-private-only" label={$t("canvas.envSettings.privateOnly")} help={$t("canvas.envSettings.privateOnlyHint")} settingKey="canvas.envSettings.privateOnly"
    disabledReason={remoteFailed ? ($t("settings.ai.remote_unavailable") as string) : undefined}>
    {#snippet control({ id, describedBy })}
      <button {id} type="button" class="toggle" class:on={!!remote?.privateOnly} role="switch" aria-checked={!!remote?.privateOnly} aria-labelledby={`${id}-label`} aria-describedby={describedBy}
        disabled={!remote} onclick={() => setPrivateOnly(!remote?.privateOnly)}><span class="toggle-knob"></span></button>
    {/snippet}
  </SettingRow>
  {#if remote?.lastRefused}<p class="state"><bdi>{$t("canvas.envSettings.lastRefused", { ip: remote.lastRefused.ip })}</bdi></p>{/if}
  {#if remoteFailed}<div class="buttons"><button type="button" class="btn btn-secondary btn-sm" onclick={loadRemote}>{$t("settings.rebrand.state.retry")}</button></div>{/if}
</Subgroup>

<style>
  .state { margin: 0; padding-block: var(--space-1); font-size: var(--text-sm); line-height: 18px; color: var(--text-muted); overflow-wrap: anywhere; }
  .label { display: block; padding-block: var(--space-2) var(--space-1); font-size: var(--text-base); color: var(--text); }
  .runtimes { border-block-start: 1px solid var(--separator); }
  .item { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); padding-block: var(--space-2); }
  .item + .item { border-block-start: 1px solid var(--separator); }
  .item-text { display: flex; flex-direction: column; min-inline-size: 0; font-size: var(--text-base); color: var(--text); overflow-wrap: anywhere; }
  .item-text small { font-size: var(--text-sm); color: var(--text-muted); }
  .mono { font-family: var(--font-mono); overflow-wrap: anywhere; }
  .danger { color: var(--error); }
  .editor { display: flex; flex-direction: column; gap: var(--space-2); padding-block: var(--space-2); }
  .editor label { display: flex; flex-direction: column; gap: var(--space-1); flex: 1 1 12rem; min-inline-size: 0; font-size: var(--text-sm); color: var(--text-muted); }
  .pair { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .error { margin: 0; font-size: var(--text-sm); color: var(--error); }
  .buttons { display: flex; justify-content: flex-end; gap: var(--space-2); padding-block: var(--space-2); }
</style>
