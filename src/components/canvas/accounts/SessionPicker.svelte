<script lang="ts">
  import { t } from "$lib/i18n";
  import { canvasAccounts, colorFor, type PortalSession } from "$lib/canvas/accounts/store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  let {
    current,
    onpick,
  }: {
    current: string | null;
    onpick: (id: string | null) => void;
  } = $props();

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) }) as string;

  let mode = $state<"pick" | "new" | "rename" | "delete">("pick");
  let name = $state("");
  let users = $state<string[]>([]);
  let busy = $state(false);
  let inputEl = $state<HTMLInputElement | null>(null);

  const session = $derived<PortalSession | undefined>(canvasAccounts.session(current));
  const missing = $derived(!!current && canvasAccounts.loaded && !session);
  const color = $derived(current ? session?.color || colorFor(current) : "transparent");

  $effect(() => {
    if ((mode === "new" || mode === "rename") && inputEl) inputEl.focus();
  });

  async function change(e: Event) {
    const el = e.currentTarget as HTMLSelectElement;
    const v = el.value;
    el.value = current ?? "";
    if (v === "__new") {
      if (!canvasAccounts.supported) {
        showToast("error", tr("canvas.accounts.sessions.unsupported", "Named portal sessions need macOS 14 or later."));
        return;
      }
      name = "";
      mode = "new";
    } else if (v === "__rename" && session) {
      name = session.label;
      mode = "rename";
    } else if (v === "__delete" && session) {
      users = [];
      mode = "delete";
      try {
        const list = await canvasAccounts.usage(session.id);
        users = list.map((u) => u.title || u.nodeId);
      } catch {
        users = [];
      }
    } else if (!v.startsWith("__")) {
      onpick(v === "" ? null : v);
    }
  }

  async function submit() {
    const label = name.trim();
    if (!label || busy) return;
    busy = true;
    try {
      if (mode === "new") {
        const s = await canvasAccounts.createSession(label);
        onpick(s.id);
      } else if (mode === "rename" && session) {
        await canvasAccounts.updateSession(session.id, { label });
      }
      mode = "pick";
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = false;
    }
  }

  async function remove() {
    if (!session || busy) return;
    busy = true;
    try {
      await canvasAccounts.deleteSession(session.id);
      mode = "pick";
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      void submit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      mode = "pick";
    }
  }
</script>

{#if mode === "new" || mode === "rename"}
  <span class="edit">
    <input
      bind:this={inputEl}
      bind:value={name}
      class="name"
      type="text"
      maxlength="60"
      spellcheck="false"
      autocomplete="off"
      placeholder={tr("canvas.accounts.sessions.namePlaceholder", "Teams work")}
      aria-label={tr("canvas.accounts.sessions.name", "Session name")}
      onkeydown={onKey}
    />
    <button type="button" class="go" disabled={busy || !name.trim()} onclick={submit}>
      {mode === "new" ? tr("canvas.accounts.sessions.create", "Create") : tr("canvas.accounts.sessions.save", "Save")}
    </button>
    <button type="button" class="ghost" onclick={() => (mode = "pick")}>{tr("canvas.accounts.sessions.cancel", "Cancel")}</button>
  </span>
{:else if mode === "delete" && session}
  <span class="edit" role="alert">
    <span class="ask">
      {users.length
        ? tr("canvas.accounts.sessions.deleteUsed", "Delete \"{{name}}\" and its logins? Used by: {{names}}", {
            name: session.label,
            names: users.join(", "),
          })
        : tr("canvas.accounts.sessions.deleteAsk", "Delete \"{{name}}\" and its logins?", { name: session.label })}
    </span>
    <button type="button" class="go danger" disabled={busy} onclick={remove}>{tr("canvas.accounts.sessions.delete", "Delete")}</button>
    <button type="button" class="ghost" onclick={() => (mode = "pick")}>{tr("canvas.accounts.sessions.cancel", "Cancel")}</button>
  </span>
{:else}
  <span class="pick" class:missing class:named={!!current} style:--sess={color}>
    {#if current}<span class="dot" aria-hidden="true">●</span>{/if}
    <select
      title={missing
        ? tr("canvas.accounts.sessions.missing", "This session no longer exists; pick another")
        : tr("canvas.accounts.sessions.label", "Session: the logins this portal uses")}
      aria-label={tr("canvas.accounts.sessions.label", "Session: the logins this portal uses")}
      value={current ?? ""}
      onchange={change}
    >
      <option value="">{tr("canvas.accounts.sessions.own", "Own storage")}</option>
      {#each canvasAccounts.sessions as s (s.id)}
        <option value={s.id}>{s.label}</option>
      {/each}
      {#if missing && current}
        <option value={current} disabled>{tr("canvas.accounts.sessions.missingOption", "(deleted session)")}</option>
      {/if}
      <option value="__new" disabled={!canvasAccounts.supported}>{tr("canvas.accounts.sessions.new", "New session…")}</option>
      {#if session}
        <option value="__rename">{tr("canvas.accounts.sessions.renameThis", "Rename this session…")}</option>
        <option value="__delete">{tr("canvas.accounts.sessions.deleteThis", "Delete this session…")}</option>
      {/if}
    </select>
  </span>
{/if}

<style>
  .pick {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    max-width: 150px;
    border-radius: 6px;
  }
  .pick.named {
    padding-left: 5px;
    background: color-mix(in srgb, var(--sess) 18%, transparent);
    border: 1px solid color-mix(in srgb, var(--sess) 45%, transparent);
  }
  .pick.missing {
    --sess: var(--error, #ff453a);
    color: var(--error, #ff453a);
  }
  .dot {
    color: var(--sess);
    font-size: 9px;
  }
  select {
    min-width: 0;
    max-width: 140px;
    height: 22px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: inherit;
    font-size: 11px;
    cursor: pointer;
  }
  .edit {
    flex: 1;
    min-width: 0;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
  }
  .name {
    flex: 1;
    min-width: 60px;
    height: 22px;
    border-radius: 6px;
    border: 1px solid color-mix(in srgb, var(--text, #fff) 16%, transparent);
    background: transparent;
    color: inherit;
    font: inherit;
    padding: 0 6px;
  }
  .ask {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  button {
    flex: none;
    height: 22px;
    border: 0;
    border-radius: 6px;
    padding: 0 8px;
    font-size: 11px;
    cursor: pointer;
  }
  .go {
    background: var(--accent, #ff9f0a);
    color: #111;
  }
  .go.danger {
    background: var(--error, #ff453a);
    color: #fff;
  }
  .ghost {
    background: transparent;
    color: var(--text-dim, #98989d);
  }
</style>
