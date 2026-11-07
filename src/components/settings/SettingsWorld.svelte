<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SettingGroup from "./rebrand/SettingGroup.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingsEmptyState from "./rebrand/SettingsEmptyState.svelte";
  import GeneralChoice from "./rebrand/GeneralChoice.svelte";
  import IntegrationsDisclosure from "./integrations/IntegrationsDisclosure.svelte";
  import { CORNER_POINTS, PET_CORNERS, petCornerOrDefault } from "./integrations/pet-corners";

  type PetCaps = {
    transparency?: string;
    click_through?: boolean;
    fallback_recommended?: boolean;
    open?: boolean;
    corner?: string;
    click_through_on?: boolean;
  };
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

{#snippet cornerArt(corner: string)}
  {@const point = CORNER_POINTS[corner as keyof typeof CORNER_POINTS]}
  <rect class="pet-screen" x="9" y="7" width="62" height="36" rx="3" />
  <circle class="pet-dot" cx={point.x} cy={point.y} r="5" />
  <circle class="pet-eye" cx={point.x - 1.8} cy={point.y - 0.8} r="0.9" />
  <circle class="pet-eye" cx={point.x + 1.8} cy={point.y - 0.8} r="0.9" />
{/snippet}
{#snippet artTopLeft()}{@render cornerArt("top-left")}{/snippet}
{#snippet artTopRight()}{@render cornerArt("top-right")}{/snippet}
{#snippet artBottomLeft()}{@render cornerArt("bottom-left")}{/snippet}
{#snippet artBottomRight()}{@render cornerArt("bottom-right")}{/snippet}

<SettingGroup id="pet" icon="pet" title={$t("settings.integrations.pet.title")} description={$t("settings.integrations.pet.desc")}>
  {#if petLoading}
    <p class="pet-state" role="status" aria-busy="true">{$t("settings.rebrand.state.loading")}</p>
  {:else if petError}
    <SettingsEmptyState error title={$t("settings.world.pet_load_failed")} onRetry={() => void loadPet()} />
  {:else if pet?.fallback_recommended}
    <SettingsEmptyState icon="pet" title={$t("pet.unsupported")} />
  {:else}
    {@const open = pet?.open ?? false}
    <SettingRow id="settings-pet-enabled" label={$t("pet.enable")} settingKey="pet.enable">
      {#snippet control(c)}
        <button
          type="button"
          class="toggle"
          class:on={open}
          id={c.id}
          aria-labelledby={`${c.id}-label`}
          aria-describedby={c.describedBy}
          disabled={pet === null || petBusy}
          onclick={() => petCall(open ? "pet_close" : "pet_open")}
          role="switch"
          aria-checked={open}
        ><span class="toggle-knob"></span></button>
      {/snippet}
    </SettingRow>

    <div class="pet-corner" data-setting-key="pet.menu.corner" role="group" aria-describedby={open ? undefined : "pet-corner-reason"}>
      <GeneralChoice
        name="settings-pet-corner"
        label={$t("pet.menu.corner") as string}
        disabled={!open || petBusy}
        value={petCornerOrDefault(pet?.corner)}
        options={PET_CORNERS.map((corner) => ({
          value: corner,
          label: $t(`pet.corner.${corner}`) as string,
          help: "",
          draw: { "top-left": artTopLeft, "top-right": artTopRight, "bottom-left": artBottomLeft, "bottom-right": artBottomRight }[corner],
        }))}
        onchange={(corner) => petCall("pet_set_corner", { corner })}
      />
      {#if !open}<p class="pet-reason" id="pet-corner-reason"><bdi>{$t("settings.integrations.pet.corner_off")}</bdi></p>{/if}
    </div>

    <IntegrationsDisclosure id="pet-advanced">
      <SettingRow
        id="settings-pet-click-through"
        label={$t("pet.click_through")}
        help={$t("settings.integrations.pet.click_through_desc")}
        settingKey="pet.click_through"
        disabledReason={pet?.click_through === false ? ($t("settings.integrations.pet.click_through_unsupported") as string) : undefined}
      >
        {#snippet control(c)}
          <button
            type="button"
            class="toggle"
            class:on={pet?.click_through_on ?? false}
            id={c.id}
            aria-labelledby={`${c.id}-label`}
            aria-describedby={c.describedBy}
            disabled={pet === null || petBusy || pet?.click_through === false}
            onclick={() => petCall("pet_set_click_through", { enabled: !(pet?.click_through_on ?? false) })}
            role="switch"
            aria-checked={pet?.click_through_on ?? false}
          ><span class="toggle-knob"></span></button>
        {/snippet}
      </SettingRow>
    </IntegrationsDisclosure>
  {/if}
</SettingGroup>

<style>
  .pet-state { margin: 0; padding-block: var(--space-2); font-size: 13px; color: var(--text-muted); }
  .pet-corner { padding-block-end: var(--space-2); }
  .pet-corner :global(.choices) { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  @container (max-width: 420px) { .pet-corner :global(.choices) { grid-template-columns: minmax(0, 1fr); } }
  .pet-corner :global(.choice-help:empty), .pet-corner :global(.choice-help:has(bdi:empty)) { display: none; }
  .pet-reason { margin: var(--space-1) 0 0; font-size: 12px; line-height: 18px; color: var(--text-muted); }
  .pet-corner :global(.pet-screen) { fill: none; stroke: var(--content-border); }
  .pet-corner :global(.pet-dot) { fill: var(--accent); }
  .pet-corner :global(.pet-eye) { fill: var(--on-accent, var(--primary)); }
</style>
