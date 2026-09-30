<script lang="ts">
  /**
   * Settings → World: the measured tier, the pinned one, a recalibration and
   * the one destructive action the world has (delete it).
   *
   * The renderer is imported here only when "recalibrate" is clicked, so
   * opening Settings costs nothing.
   */
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { createSettingsDraft } from "$lib/stores/settings-drafts.svelte";
  import SettingGroup from "$components/settings/rebrand/SettingGroup.svelte";
  import SettingRow from "$components/settings/rebrand/SettingRow.svelte";
  import SettingsSaveBar from "$components/settings/rebrand/SettingsSaveBar.svelte";

  type Tier = 0 | 1 | 2 | 3;
  const TIERS: Tier[] = [0, 1, 2, 3];

  let settings = $derived(getSettings());
  let enabled = $derived(settings?.world?.enabled ?? true);
  let measured = $derived(settings?.world?.tier_measured ?? null);
  let medianMs = $derived(settings?.world?.measured_median_ms ?? null);
  let measuredVersion = $derived(settings?.world?.measured_app_version ?? null);
  let pinned = $derived(settings?.world?.tier_override ?? null);
  let thinking = $derived(settings?.world?.thinking ?? false);

  // The room server address is free text: a draft saved with "Save changes".
  const roomServer = createSettingsDraft({
    id: "world-room-server",
    read: (s) => ({ room_server: s.world?.room_server ?? "" }),
    toPatch: (d) => ({ world: { room_server: d.room_server.trim() } }),
  });

  let calibrating = $state(false);
  let lowered = $state<Tier | null>(null);
  let exists = $state<boolean | null>(null);
  let confirmingDelete = $state(false);
  let deleting = $state(false);

  function tierLabel(tier: number | null): string {
    if (tier === null || tier === undefined) return $t("world.settings.tier_none") as string;
    return `${tier} · ${$t(`world.settings.tier_names.${tier}`)}`;
  }

  async function recalibrate() {
    if (calibrating) return;
    calibrating = true;
    try {
      // The one place in Settings that loads the renderer, and only on click.
      const render = await import("$lib/world/render");
      const detail = await render.calibrateDetailed(null);
      let version: string | null = null;
      try {
        version = await getVersion();
      } catch {
        version = null;
      }
      await updateSettings({
        world: {
          tier_measured: detail.tier,
          measured_median_ms: Number.isFinite(detail.medianMs) ? detail.medianMs : null,
          measured_app_version: version,
        },
      });
      lowered = null;
    } catch (error) {
      // The renderer's error text is diagnostic, not user copy.
      console.warn("[world] calibration failed", error);
      showToast("error", $t("world.error") as string);
    } finally {
      calibrating = false;
    }
  }

  async function pin(value: string) {
    const next = value === "auto" ? null : (Number(value) as Tier);
    await updateSettings({ world: { tier_override: next } });
  }

  async function toggleEnabled() {
    const next = !enabled;
    await updateSettings({ world: { enabled: next } });
    if (!next) {
      // Switching the world off has to stop it, not hide it: with the setting
      // already false, `world_close` shuts the tick thread down instead of
      // dropping it to 0.2 Hz.
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        await invoke("world_close");
      } catch {
        // Nothing was running, which is the outcome we wanted anyway.
      }
    }
  }

  async function deleteWorld() {
    if (!confirmingDelete) {
      confirmingDelete = true;
      return;
    }
    deleting = true;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("world_delete");
      exists = false;
      showToast("success", $t("world.settings.deleted") as string);
    } catch (error) {
      console.warn("[world] delete failed", error);
      showToast("error", $t("common.error") as string);
    } finally {
      deleting = false;
      confirmingDelete = false;
    }
  }

  onMount(() => {
    void (async () => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        exists = (await invoke("world_exists")) as boolean;
      } catch {
        // The bridge may not be in this build yet: the section still works.
        exists = null;
      }
    })();
    // Emitted by the renderer's watchdog while /world is open in this webview.
    const onLowered = (event: Event) => {
      const to = (event as CustomEvent<{ to?: Tier }>).detail?.to;
      lowered = typeof to === "number" ? to : 0;
    };
    window.addEventListener("world:tier-changed", onLowered);
    return () => window.removeEventListener("world:tier-changed", onLowered);
  });
</script>

<SettingGroup id="world" title={$t("world.settings.title")} description={$t("world.settings.desc")}>
  <SettingRow id="world-enabled" label={$t("world.settings.enabled")} help={$t("world.settings.enabled_desc")} settingKey="world.settings.enabled" status="world.enabled">
    {#snippet control(c)}
      <button
        type="button"
        class="toggle"
        class:on={enabled}
        id={c.id}
        aria-describedby={c.describedBy}
        onclick={toggleEnabled}
        role="switch"
        aria-checked={enabled}
        aria-label={$t("world.settings.enabled") as string}
      ><span class="toggle-knob"></span></button>
    {/snippet}
  </SettingRow>
</SettingGroup>

<SettingGroup id="world-advanced" title={$t("settings.rebrand.category.advanced")} advanced>
  <SettingRow id="world-thinking" label={$t("world.settings.thinking")} help={$t("world.settings.thinking_desc")} settingKey="world.settings.thinking" status="world.thinking">
    {#snippet control(c)}
      <button
        type="button"
        class="toggle"
        class:on={thinking}
        id={c.id}
        aria-describedby={c.describedBy}
        disabled={!enabled}
        onclick={() => updateSettings({ world: { thinking: !thinking } })}
        role="switch"
        aria-checked={thinking}
        aria-label={$t("world.settings.thinking") as string}
      ><span class="toggle-knob"></span></button>
    {/snippet}
  </SettingRow>

  <form class="room-form" onsubmit={(e) => { e.preventDefault(); void roomServer.save(); }}>
    <SettingRow id="world-room-server" label={$t("world.settings.room_server")} help={$t("world.settings.room_server_desc")} settingKey="world.settings.room_server">
      {#snippet control(c)}
        <input
          id={c.id}
          aria-describedby={c.describedBy}
          class="room-server"
          type="text"
          dir="ltr"
          placeholder="wss://…/v1/room"
          spellcheck="false"
          autocapitalize="off"
          autocomplete="off"
          disabled={!enabled}
          bind:value={roomServer.value.room_server}
        />
      {/snippet}
    </SettingRow>
    <SettingsSaveBar dirty={roomServer.dirty} saving={roomServer.saving} error={roomServer.error} onSave={() => void roomServer.save()} onDiscard={() => roomServer.discard()} />
  </form>

  <SettingRow id="world-recalibrate" label={$t("world.settings.tier_measured")} help={measured === null ? ($t("world.settings.never_measured") as string) : `${tierLabel(measured)}${medianMs !== null ? ` · ${medianMs.toFixed(1)} ms` : ""}${measuredVersion ? ` · v${measuredVersion}` : ""}`} settingKey="world.settings.tier_measured">
    {#snippet control(c)}
      <button type="button" class="btn btn-secondary" id={c.id} aria-describedby={c.describedBy} onclick={recalibrate} disabled={calibrating} aria-busy={calibrating}>
        {calibrating ? $t("world.settings.recalibrating") : $t("world.settings.recalibrate")}
      </button>
    {/snippet}
  </SettingRow>

  <SettingRow id="world-tier-pinned" label={$t("world.settings.tier_pinned")} help={$t("world.settings.tier_pinned_desc")} settingKey="world.settings.tier_pinned" status="world.tier_override">
    {#snippet control(c)}
      <select id={c.id} aria-describedby={c.describedBy} class="input-text select" value={pinned === null ? "auto" : String(pinned)} onchange={(e) => pin((e.target as HTMLSelectElement).value)}>
        <option value="auto">{$t("world.settings.tier_auto")}</option>
        {#each TIERS as tier (tier)}
          <option value={String(tier)}>{tierLabel(tier)}</option>
        {/each}
      </select>
    {/snippet}
  </SettingRow>

  {#if lowered !== null}
    <p class="world-lowered" role="status">
      <strong>{$t("world.settings.tier_lowered")}</strong> {tierLabel(lowered)}
    </p>
  {/if}

  <SettingRow id="world-delete" label={$t("world.settings.delete")} help={enabled && exists === false ? ($t("world.settings.delete_none") as string) : ($t("world.settings.delete_desc") as string)} settingKey="world.settings.delete">
    {#snippet control(c)}
      <!-- With the world switched off `world_exists` answers "no" by design, so
           the button stays available: "I turned it off, now erase it" has to
           work. Deleting nothing is not an error. -->
      <button
        type="button"
        class="btn btn-secondary"
        id={c.id}
        aria-describedby={c.describedBy}
        onclick={deleteWorld}
        disabled={deleting || (enabled && exists === false)}
      >
        {confirmingDelete ? $t("world.settings.delete_confirm") : $t("world.settings.delete")}
      </button>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  .room-form { display: flex; flex-direction: column; min-inline-size: 0; }
  .world-lowered { margin: 0; padding-block: var(--space-2); font-size: 12px; line-height: 18px; color: var(--text-muted); }
  .room-server {
    width: 100%;
    box-sizing: border-box;
    padding: 5px 9px;
    border-radius: 8px;
    border: 1px solid var(--separator, rgba(127, 127, 127, 0.3));
    background: var(--fill-quaternary, rgba(127, 127, 127, 0.08));
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    font-family: var(--font-mono, ui-monospace, monospace);
  }
</style>
