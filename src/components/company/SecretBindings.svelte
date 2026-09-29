<script lang="ts">
  /**
   * Secrets per agent: a value lives in the OS secret store (never in the
   * company database), and only the agents bound to it receive it, as an
   * environment variable, when they run as a command.
   */
  import { t } from "$lib/i18n";
  import { errorText, rpc, type Staff } from "$lib/company/rpc";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { onMount } from "svelte";

  type Binding = { staff_id: string; env_key: string; secret_name: string; has_value: boolean };
  let { company, staff }: { company: string; staff: Staff[] } = $props();
  let list = $state<Binding[]>([]);
  let who = $state("");
  let envKey = $state("");
  let name = $state("");
  let value = $state("");
  let busy = $state(false);

  async function load() {
    try {
      list = await rpc<Binding[]>("secret.list", { company });
    } catch {
      list = [];
    }
  }
  onMount(() => void load());

  const nameOf = (id: string) => staff.find((s) => s.id === id)?.name ?? id;

  async function add() {
    if (!who || !envKey.trim() || !name.trim()) return;
    busy = true;
    try {
      if (value) await rpc("secret.set", { company, name: name.trim(), value });
      await rpc("secret.bind", { company, staff: who, env_key: envKey.trim().toUpperCase(), name: name.trim() });
      envKey = name = value = "";
      await load();
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      busy = false;
    }
  }

  async function remove(b: Binding) {
    try {
      await rpc("secret.unbind", { company, staff: b.staff_id, env_key: b.env_key });
      await load();
    } catch (e) {
      showToast("error", errorText(e));
    }
  }
</script>

<section>
  <h3 class="group-label">{$t("company.team.secrets.title")}</h3>
  <div class="group">
    {#each list as b (b.staff_id + b.env_key)}
      <div class="sec-row">
        <span class="mono">{b.env_key}</span>
        <span class="dim">← {b.secret_name}{b.has_value ? "" : ` (${$t("company.team.secrets.missing")})`}</span>
        <span class="grow">{nameOf(b.staff_id)}</span>
        <button type="button" class="button" onclick={() => remove(b)}>{$t("company.team.secrets.remove")}</button>
      </div>
    {/each}
    <div class="sec-row form">
      <select class="input" bind:value={who} aria-label={$t("company.team.secrets.agent")}>
        <option value="">{$t("company.team.secrets.agent")}</option>
        {#each staff.filter((s) => s.status !== "terminated") as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
      </select>
      <input class="input mono" placeholder="API_TOKEN" bind:value={envKey} aria-label={$t("company.team.secrets.env")} />
      <input class="input" placeholder={$t("company.team.secrets.name")} bind:value={name} />
      <input class="input" type="password" placeholder={$t("company.team.secrets.value")} bind:value={value} autocomplete="off" />
      <button type="button" class="button active" disabled={busy || !who || !envKey || !name} onclick={add}>{$t("company.team.secrets.add")}</button>
    </div>
  </div>
  <p class="hint">{$t("company.team.secrets.hint")}</p>
</section>

<style>
  .sec-row { display: flex; align-items: center; gap: var(--space-3); padding: var(--space-2) var(--space-3); font-size: var(--text-sm); }
  .sec-row + .sec-row { border-top: var(--hairline) solid var(--separator); }
  .sec-row.form { flex-wrap: wrap; }
  .sec-row.form .input { flex: 1; min-width: 120px; }
  .grow { flex: 1; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); }
  .dim { color: var(--text-dim); }
  .hint { font-size: var(--text-xs); color: var(--text-dim); margin: var(--space-1) var(--space-1) 0; }
</style>
