<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { canvasAccounts, colorFor, type SessionUser } from "$lib/canvas/accounts/store.svelte";

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) }) as string;

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

<section class="settings-section" id="settings-section-canvas-portal-sessions">
  <h5 class="section-title">{tr("canvas.accounts.sessions.title", "Portal sessions")}</h5>
  <div class="card">
    <p class="setting-path intro">
      {tr(
        "canvas.accounts.sessions.hint",
        "A session is a named browser profile that portals can share, like \"Teams work\". Log in once inside a portal that uses it; renaming keeps the login. Agents see only the names, never cookies or passwords.",
      )}
    </p>
    {#if !canvasAccounts.supported}
      <p class="setting-path warn">{tr("canvas.accounts.sessions.unsupported", "Named portal sessions need macOS 14 or later.")}</p>
    {/if}
    {#each canvasAccounts.sessions as s (s.id)}
      <div class="setting-row">
        <div class="setting-col">
          {#if editing === s.id}
            <input
              class="input-text"
              maxlength="60"
              bind:value={editName}
              aria-label={tr("canvas.accounts.sessions.name", "Session name")}
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
              ? tr("canvas.accounts.sessions.usedBy", "Used by {{names}}", { names: names(s.id) })
              : tr("canvas.accounts.sessions.unused", "Not used by any portal")}
          </span>
        </div>
        <div class="pair">
          {#if confirming === s.id}
            <span class="setting-path">{tr("canvas.accounts.sessions.deleteAsk", "Delete \"{{name}}\" and its logins?", { name: s.label })}</span>
            <button class="button" disabled={busy} onclick={() => (confirming = null)}>{tr("canvas.accounts.sessions.cancel", "Cancel")}</button>
            <button class="button danger" disabled={busy} onclick={() => remove(s.id)}>{tr("canvas.accounts.sessions.delete", "Delete")}</button>
          {:else if editing === s.id}
            <button class="button" disabled={busy} onclick={() => (editing = null)}>{tr("canvas.accounts.sessions.cancel", "Cancel")}</button>
            <button class="button primary" disabled={busy || !editName.trim()} onclick={() => rename(s.id)}>{tr("canvas.accounts.sessions.save", "Save")}</button>
          {:else}
            <input
              class="color"
              type="color"
              value={s.color || colorFor(s.id)}
              disabled={busy}
              aria-label={tr("canvas.accounts.sessions.color", "Color")}
              onchange={(e) => recolor(s.id, (e.currentTarget as HTMLInputElement).value)}
            />
            <button class="button" disabled={busy} onclick={() => { editing = s.id; editName = s.label; }}>{tr("canvas.accounts.sessions.rename", "Rename")}</button>
            <button class="button danger" disabled={busy} onclick={() => (confirming = s.id)}>{tr("canvas.accounts.sessions.delete", "Delete")}</button>
          {/if}
        </div>
      </div>
      <div class="divider"></div>
    {:else}
      <p class="setting-path">{tr("canvas.accounts.sessions.empty", "No sessions yet.")}</p>
    {/each}
    <div class="pair add">
      <input
        class="input-text"
        maxlength="60"
        bind:value={draft}
        disabled={!canvasAccounts.supported}
        placeholder={tr("canvas.accounts.sessions.namePlaceholder", "Teams work")}
        aria-label={tr("canvas.accounts.sessions.name", "Session name")}
        onkeydown={(e) => e.key === "Enter" && create()}
      />
      <button class="button primary" disabled={busy || !draft.trim() || !canvasAccounts.supported} onclick={create}>
        {tr("canvas.accounts.sessions.create", "Create")}
      </button>
    </div>
  </div>
</section>

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
  .intro {
    margin: 4px 0 10px;
  }
  .warn {
    color: var(--error, #ff453a);
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
    color: var(--error, #ff453a);
  }
</style>
