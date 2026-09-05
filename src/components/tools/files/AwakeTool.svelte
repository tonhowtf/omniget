<script lang="ts">
  /** Manter o computador acordado (estudo 29, PowerToys Awake). */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/tools/rt";

  let active = $state(false);
  onMount(async () => { active = await invoke<boolean>("tool_awake_get"); });
  async function toggle() {
    try { active = await invoke<boolean>("tool_awake_set", { active: !active }); }
    catch (e) { showToast("error", errText(e)); }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{active ? $t("tools.awake.on") : $t("tools.awake.off")}</div>
          <div class="group-row-sub">{$t("tools.awake.intro")}</div>
        </div>
        <div class="group-row-trailing"><button class="btn {active ? 'btn-secondary' : 'btn-primary'}" type="button" onclick={toggle}>{active ? $t("tools.awake.stop") : $t("tools.awake.start")}</button></div>
      </div>
      <div class="group-row"><div class="group-row-sub">{$t("tools.awake.note")}</div></div>
    </div>
  </section>
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
</style>
