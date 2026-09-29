<script lang="ts">
  /**
   * Settings → World: the world block (phase 7, in `components/world`) plus
   * the desktop pet of phase 5, which shares the section because both are the
   * same thing to the user — the agents, outside the app.
   */
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import WorldSettings from "$components/world/WorldSettings.svelte";

  // Desktop pet (phase 5). State comes from `pet_capabilities`, which also says
  // whether this desktop can draw a transparent window at all.
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
      showToast("error", String(error));
    } finally {
      petBusy = false;
    }
  }

  onMount(loadPet);
</script>

<WorldSettings />

<section class="settings-section" id="settings-pet">
  <h5 class="section-title">{$t("pet.title")}</h5>
  <div class="card">
    {#if petLoading}
      <p role="status">{$t("common.loading")}</p>
    {:else if petError}
      <div role="alert">
        <p>{$t("settings.world.pet_load_failed")}</p>
        <button class="button" onclick={loadPet}>{$t("error.remedy.retry")}</button>
      </div>
    {:else if pet?.fallback_recommended}
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-path">{$t("pet.unsupported")}</span>
        </div>
      </div>
    {:else}
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t("pet.enable")}</span>
        </div>
        <button
          class="toggle"
          class:on={pet?.open ?? false}
          disabled={pet === null || petBusy}
          onclick={() => petCall(pet?.open ? "pet_close" : "pet_open")}
          role="switch"
          aria-checked={pet?.open ?? false}
          aria-label={$t("pet.enable") as string}
        ><span class="toggle-knob"></span></button>
      </div>

      <div class="divider"></div>

      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t("pet.click_through")}</span>
        </div>
        <button
          class="toggle"
          class:on={pet?.click_through_on ?? false}
          disabled={pet === null || petBusy || pet?.click_through === false}
          onclick={() => petCall("pet_set_click_through", { enabled: !(pet?.click_through_on ?? false) })}
          role="switch"
          aria-checked={pet?.click_through_on ?? false}
          aria-label={$t("pet.click_through") as string}
        ><span class="toggle-knob"></span></button>
      </div>

      <div class="divider"></div>

      <div class="setting-row">
        <div class="setting-col">
          <label class="setting-label" for="settings-pet-corner">{$t("pet.menu.corner")}</label>
        </div>
        <select
          id="settings-pet-corner"
          class="input-text select"
          disabled={pet === null || petBusy}
          value={pet?.corner ?? "bottom-right"}
          onchange={(e) => petCall("pet_set_corner", { corner: (e.target as HTMLSelectElement).value })}
        >
          {#each CORNERS as corner (corner)}
            <option value={corner}>{$t(`pet.corner.${corner}`)}</option>
          {/each}
        </select>
      </div>
    {/if}
  </div>
</section>
