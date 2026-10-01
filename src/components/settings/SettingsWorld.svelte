<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SettingGroup from "./rebrand/SettingGroup.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingsEmptyState from "./rebrand/SettingsEmptyState.svelte";

  let settings = $derived(getSettings());
  let enabled = $derived(settings?.world?.enabled ?? true);

  async function toggleEnabled() {
    await updateSettings({ world: { enabled: !enabled } });
  }

  type PetCaps = {
    transparency?: string;
    click_through?: boolean;
    fallback_recommended?: boolean;
    open?: boolean;
    corner?: string;
    click_through_on?: boolean;
  };
  const CORNERS = ["top-left", "top-right", "bottom-left", "bottom-right"];
  let pet = $state<PetCaps | null>(null);
  let petBusy = $state(false);
  let petLoading = $state(true);
  let petError = $state(false);

  async function loadPet() {
    petLoading = true;
    petError = false;
    try {
      pet = await invoke<PetCaps>("pet_capabilities");
    } catch {
      petError = true;
    } finally {
      petLoading = false;
    }
  }

  async function petCall(command: string, args?: Record<string, unknown>) {
    if (petBusy) return;
    petBusy = true;
    try {
      await invoke(command, args);
      pet = (await invoke("pet_capabilities")) as PetCaps;
    } catch (error) {
      petError = true;
      console.warn("[pet] command failed", command, error);
      showToast("error", $t("common.error") as string);
    } finally {
      petBusy = false;
    }
  }

  onMount(loadPet);
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

<SettingGroup id="pet" title={$t("pet.title")}>
  {#if petLoading}
    <p class="pet-state" role="status" aria-busy="true">{$t("settings.rebrand.state.loading")}</p>
  {:else if petError}
    <SettingsEmptyState error title={$t("settings.world.pet_load_failed")} onRetry={() => void loadPet()} />
  {:else if pet?.fallback_recommended}
    <SettingsEmptyState title={$t("pet.unsupported")} />
  {:else}
    <SettingRow id="settings-pet-enabled" label={$t("pet.enable")} settingKey="pet.enable">
      {#snippet control(c)}
        <button
          type="button"
          class="toggle"
          class:on={pet?.open ?? false}
          id={c.id}
          aria-describedby={c.describedBy}
          disabled={pet === null || petBusy}
          onclick={() => petCall(pet?.open ? "pet_close" : "pet_open")}
          role="switch"
          aria-checked={pet?.open ?? false}
          aria-label={$t("pet.enable") as string}
        ><span class="toggle-knob"></span></button>
      {/snippet}
    </SettingRow>

    <SettingRow id="settings-pet-click-through" label={$t("pet.click_through")} settingKey="pet.click_through">
      {#snippet control(c)}
        <button
          type="button"
          class="toggle"
          class:on={pet?.click_through_on ?? false}
          id={c.id}
          aria-describedby={c.describedBy}
          disabled={pet === null || petBusy || pet?.click_through === false}
          onclick={() => petCall("pet_set_click_through", { enabled: !(pet?.click_through_on ?? false) })}
          role="switch"
          aria-checked={pet?.click_through_on ?? false}
          aria-label={$t("pet.click_through") as string}
        ><span class="toggle-knob"></span></button>
      {/snippet}
    </SettingRow>

    <SettingRow id="settings-pet-corner" label={$t("pet.menu.corner")} settingKey="pet.menu.corner">
      {#snippet control(c)}
        <select
          id={c.id}
          aria-describedby={c.describedBy}
          class="input-text select"
          disabled={pet === null || petBusy}
          value={pet?.corner ?? "bottom-right"}
          onchange={(e) => petCall("pet_set_corner", { corner: (e.target as HTMLSelectElement).value })}
        >
          {#each CORNERS as corner (corner)}
            <option value={corner}>{$t(`pet.corner.${corner}`)}</option>
          {/each}
        </select>
      {/snippet}
    </SettingRow>
  {/if}
</SettingGroup>

<style>
  .pet-state { margin: 0; padding-block: var(--space-2); font-size: 13px; color: var(--text-muted); }
</style>
