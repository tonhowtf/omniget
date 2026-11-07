<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import Subgroup from "./ai/Subgroup.svelte";
  import { canvasAccounts, colorFor, type SessionUser } from "$lib/canvas/accounts/store.svelte";


  let busy = $state(false);
  let draft = $state("");
  let editing = $state<string | null>(null);
  let editName = $state("");
  let confirming = $state<string | null>(null);
  let users = $state<Record<string, SessionUser[]>>({});

  onMount(() => {
    void canvasAccounts.load(true).then(loadUsage);
  });

  async function loadUsage() {
    const next: Record<string, SessionUser[]> = {};
    for (const s of canvasAccounts.sessions) {
      next[s.id] = await canvasAccounts.usage(s.id).catch(() => []);
    }
    users = next;
  }

  async function run(f: () => Promise<unknown>) {
    if (busy) return;
    busy = true;
    try {
      await f();
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = false;
    }
  }

  function create() {
    const label = draft.trim();
    if (!label) return;
    void run(async () => {
      await canvasAccounts.createSession(label);
      draft = "";
      await loadUsage();
    });
  }

  function rename(id: string) {
    const label = editName.trim();
    if (!label) return;
    void run(async () => {
      await canvasAccounts.updateSession(id, { label });
      editing = null;
    });
  }

  function recolor(id: string, color: string) {
    void run(() => canvasAccounts.updateSession(id, { color }));
  }

  function remove(id: string) {
    void run(async () => {
      await canvasAccounts.deleteSession(id);
      confirming = null;
      await loadUsage();
    });
  }

  function names(id: string): string {
    return (users[id] ?? []).map((u) => [u.workspaceName, u.title || u.nodeId].filter(Boolean).join(" / ")).join(", ");
  }
</script>

<Subgroup id="canvas-portal-sessions" title={($t("canvas.accounts.sessions.title") as string)} intro={($t("canvas.accounts.sessions.hint") as string)} settingKey="canvas.accounts.sessions.title">
  <div class="sessions">
    {#if !canvasAccounts.supported}
      <p class="setting-path warn">{($t("canvas.accounts.sessions.unsupported") as string)}</p>
    {/if}
    {#each canvasAccounts.sessions as s (s.id)}
      <div class="setting-row">
        <div class="setting-col">
          {#if editing === s.id}
            <input
              class="input-text"
              maxlength="60"
              bind:value={editName}
              aria-label={($t("canvas.accounts.sessions.name") as string)}
              onkeydown={(e) => {
                if (e.key === "Enter") rename(s.id);
                if (e.key === "Escape") editing = null;
              }}
            />
          {:else}
            <span class="setting-label"><span class="swatch" style:background={s.color || colorFor(s.id)}></span>{s.label}</span>
          {/if}
          <span class="setting-path">
            {(users[s.id] ?? []).length
              ? ($t("canvas.accounts.sessions.usedBy", { names: names(s.id) }) as string)
              : ($t("canvas.accounts.sessions.unused") as string)}
          </span>
        </div>
        <div class="pair">
          {#if confirming === s.id}
            <span class="setting-path">{($t("canvas.accounts.sessions.deleteAsk", { name: s.label }) as string)}</span>
            <button class="btn btn-secondary btn-sm" disabled={busy} onclick={() => (confirming = null)}>{($t("canvas.accounts.sessions.cancel") as string)}</button>
            <button class="btn btn-secondary btn-sm danger" disabled={busy} onclick={() => remove(s.id)}>{($t("canvas.accounts.sessions.delete") as string)}</button>
          {:else if editing === s.id}
            <button class="btn btn-secondary btn-sm" disabled={busy} onclick={() => (editing = null)}>{($t("canvas.accounts.sessions.cancel") as string)}</button>
            <button class="btn btn-primary btn-sm" disabled={busy || !editName.trim()} onclick={() => rename(s.id)}>{($t("canvas.accounts.sessions.save") as string)}</button>
          {:else}
            <input
              class="color"
              type="color"
              value={s.color || colorFor(s.id)}
              disabled={busy}
              aria-label={($t("canvas.accounts.sessions.color") as string)}
              onchange={(e) => recolor(s.id, (e.currentTarget as HTMLInputElement).value)}
            />
            <button class="btn btn-secondary btn-sm" disabled={busy} onclick={() => { editing = s.id; editName = s.label; }}>{($t("canvas.accounts.sessions.rename") as string)}</button>
            <button class="btn btn-secondary btn-sm danger" disabled={busy} onclick={() => (confirming = s.id)}>{($t("canvas.accounts.sessions.delete") as string)}</button>
          {/if}
        </div>
      </div>
      <div class="divider"></div>
    {:else}
      <p class="setting-path">{($t("canvas.accounts.sessions.empty") as string)}</p>
    {/each}
    <div class="pair add">
      <input
        class="input-text"
        maxlength="60"
        bind:value={draft}
        disabled={!canvasAccounts.supported}
        placeholder={($t("canvas.accounts.sessions.namePlaceholder") as string)}
        aria-label={($t("canvas.accounts.sessions.name") as string)}
        onkeydown={(e) => e.key === "Enter" && create()}
      />
      <button class="btn btn-primary btn-sm" disabled={busy || !draft.trim() || !canvasAccounts.supported} onclick={create}>
        {($t("canvas.accounts.sessions.create") as string)}
      </button>
    </div>
  </div>
</Subgroup>

<style>
  .pair {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .pair.add {
    padding-top: 8px;
  }
  .pair.add input {
    flex: 1;
  }
  .warn {
    color: var(--error);
  }
  .swatch {
    display: inline-block;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    margin-right: 6px;
    vertical-align: middle;
  }
  .color {
    width: 28px;
    height: 24px;
    padding: 0;
    border: 0;
    background: transparent;
    cursor: pointer;
  }
  .danger {
    color: var(--error);
  }
</style>
