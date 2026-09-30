<script lang="ts">
  // Import review of a partitura before anything runs: the author's
  // description, every startup command (editable here), role conflicts
  // (Keep mine / Use theirs / Keep both), portals, hooks, budget and
  // permissions. "Adding a partitura to a canvas starts its terminals and
  // runs their commands on your machine."
  import { t } from "$lib/i18n";
  import { toBody } from "$lib/canvas/to-body";
  import { errorText } from "$lib/canvas/floors/api";
  import {
    importPartitura,
    previewImport,
    type ImportPreview,
    type ImportResult,
    type LibraryPartitura,
    type RoleAction,
  } from "$lib/canvas/partituras/api";

  interface Props {
    workspaceId: string;
    floorId: string;
    /** Library id, or a partitura read from a file. */
    source: { id?: string; partitura?: LibraryPartitura };
    at: { x: number; y: number };
    onClose: () => void;
    onImported: (r: ImportResult) => void;
  }
  let { workspaceId, floorId, source, at, onClose, onImported }: Props = $props();

  let preview = $state<ImportPreview | null>(null);
  let commands = $state<Record<string, string>>({});
  let roleActions = $state<Record<string, RoleAction>>({});
  let applyHooks = $state(false);
  let acceptChat = $state(false);
  let busy = $state(false);
  let error = $state("");

  $effect(() => {
    const src = source;
    previewImport(workspaceId, src)
      .then((p) => {
        preview = p;
        commands = Object.fromEntries(p.commands.map((c) => [c.nodeId, c.command ?? ""]));
        roleActions = Object.fromEntries(p.roles.map((r) => [r.role.id, r.defaultAction]));
      })
      .catch((e) => (error = errorText(e)));
  });

  async function add() {
    if (!preview || busy) return;
    busy = true;
    error = "";
    try {
      const edited: Record<string, string | null> = {};
      for (const c of preview.commands) {
        const v = commands[c.nodeId]?.trim() ?? "";
        if (v !== (c.command ?? "")) edited[c.nodeId] = v || null;
      }
      const r = await importPartitura(workspaceId, floorId, source, at, {
        commands: edited,
        roles: $state.snapshot(roleActions),
        applyHooks,
        acceptChat,
      });
      onImported(r);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  const ACTIONS: Array<[RoleAction, string, string]> = [
    ["keepMine", "canvas.partituras.keepMine", "Keep mine"],
    ["useTheirs", "canvas.partituras.useTheirs", "Use theirs"],
    ["rename", "canvas.partituras.keepBoth", "Keep both"],
  ];
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onClose()} />

<div class="scrim" use:toBody role="presentation" onclick={(e) => e.target === e.currentTarget && onClose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={$t("canvas.partituras.review", { default: "Review partitura" })}>
    <header>
      <h3>{preview?.partitura.name ?? $t("canvas.partituras.review", { default: "Review partitura" })}</h3>
      <button type="button" class="icon" onclick={onClose} aria-label={$t("canvas.partituras.close", { default: "Close" })}>✕</button>
    </header>
    {#if preview}
      {#if preview.partitura.summary}<p>{preview.partitura.summary}</p>{/if}
      {#each preview.warnings as w (w)}<p class="warn">{w}</p>{/each}

      <section>
        <h4>{$t("canvas.partituras.commands", { default: "Commands that will run" })} ({preview.commands.length})</h4>
        {#each preview.commands as c (c.nodeId)}
          <label class="cmd">
            <span>{c.title ?? c.agent}{#if c.roleName}<em> · {c.roleName}</em>{/if}</span>
            <input class="mono" bind:value={commands[c.nodeId]} placeholder={c.agent === "shell" ? $t("canvas.partituras.shell", { default: "login shell" }) : c.agent} />
          </label>
        {:else}
          <p class="muted">{$t("canvas.partituras.noCommands", { default: "No terminals" })}</p>
        {/each}
      </section>

      {#if preview.roles.length}
        <section>
          <h4>{$t("canvas.partituras.roles", { default: "Roles" })}</h4>
          {#each preview.roles as r (r.role.id)}
            <div class="role">
              <div>
                <strong>{r.role.name}</strong>
                <span class="muted">
                  {r.status === "new"
                    ? $t("canvas.partituras.roleNew", { default: "new, will be added" })
                    : r.status === "same"
                      ? $t("canvas.partituras.roleSame", { default: "same as yours" })
                      : $t("canvas.partituras.roleConflict", { default: "you have a different '{{name}}'", name: r.local?.name ?? r.role.name })}
                </span>
              </div>
              {#if r.status === "conflict"}
                <div class="seg" role="radiogroup">
                  {#each ACTIONS as [a, key, label] (a)}
                    <button type="button" class:on={roleActions[r.role.id] === a} onclick={() => (roleActions[r.role.id] = a)}>
                      {$t(key, { default: label })}
                    </button>
                  {/each}
                </div>
                <details>
                  <summary class="muted">{$t("canvas.partituras.comparePrompts", { default: "Compare prompts" })}</summary>
                  <div class="cmp"><pre>{r.local?.prompt}</pre><pre>{r.role.prompt}</pre></div>
                </details>
              {/if}
            </div>
          {/each}
        </section>
      {/if}

      {#if preview.portals.length}
        <section>
          <h4>{$t("canvas.partituras.portals", { default: "Portals" })}</h4>
          {#each preview.portals as u (u)}<code class="block">{u}</code>{/each}
        </section>
      {/if}

      {#if preview.chatTranscripts}
        <section>
          <h4>{$t("canvas.partituras.chatHistory", { default: "Chat history" })}</h4>
          <label class="check"><input type="checkbox" bind:checked={acceptChat} />{$t("canvas.partituras.acceptChat", { default: "Copy the chat history of {{count}} terminal(s) into new conversations", count: preview.chatTranscripts })}</label>
        </section>
      {/if}

      {#if preview.setup}
        <section>
          <h4>{$t("canvas.partituras.setup", { default: "Floor hooks" })}</h4>
          {#each [["setup", preview.setup.setup], ["run", preview.setup.run], ["teardown", preview.setup.teardown]] as [k, v] (k)}
            {#if v}<div class="kv"><span class="muted">{k}</span><code>{v}</code></div>{/if}
          {/each}
          <label class="check"><input type="checkbox" bind:checked={applyHooks} />{$t("canvas.partituras.applyHooks", { default: "Add these to this workspace's floor hooks" })}</label>
        </section>
      {/if}

      {#if preview.budget || preview.permissions}
        <section>
          <h4>{$t("canvas.partituras.limits", { default: "Budget and permissions" })}</h4>
          {#if preview.budget?.maxUsd != null}<div class="kv"><span class="muted">{$t("canvas.partituras.maxUsd", { default: "Max spend" })}</span>US$ {preview.budget.maxUsd}</div>{/if}
          {#if preview.budget?.maxTokens != null}<div class="kv"><span class="muted">{$t("canvas.partituras.maxTokens", { default: "Max tokens" })}</span>{preview.budget.maxTokens}</div>{/if}
          {#if preview.budget?.maxMinutes != null}<div class="kv"><span class="muted">{$t("canvas.partituras.maxMinutes", { default: "Max minutes" })}</span>{preview.budget.maxMinutes}</div>{/if}
          {#if preview.permissions?.mode}<div class="kv"><span class="muted">{$t("canvas.partituras.permMode", { default: "Permission mode" })}</span>{preview.permissions.mode}</div>{/if}
          {#if preview.permissions?.allowRaw}<div class="kv warn">{$t("canvas.partituras.allowRaw", { default: "Cords may type raw keys" })}</div>{/if}
          {#if preview.permissions?.allowEvaluate}<div class="kv warn">{$t("canvas.partituras.allowEvaluate", { default: "Portals may run JavaScript" })}</div>{/if}
        </section>
      {/if}
    {:else if !error}
      <p class="muted">{$t("canvas.partituras.loading", { default: "Reading…" })}</p>
    {/if}
    {#if error}<p class="warn">{error}</p>{/if}
    <footer>
      <button type="button" onclick={onClose}>{$t("canvas.partituras.cancel", { default: "Cancel" })}</button>
      <button type="button" class="primary" disabled={!preview || busy} onclick={add}>
        {$t("canvas.partituras.addToCanvas", { default: "Add to canvas" })}
      </button>
    </footer>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    display: grid;
    place-items: center;
    z-index: 50;
    padding: 16px;
  }
  .dialog {
    width: min(560px, 100%);
    max-height: 88vh;
    overflow: auto;
    background: var(--popup-bg, var(--surface));
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.35);
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    font-size: 13px;
  }
  header,
  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  footer {
    justify-content: flex-end;
  }
  h3,
  h4,
  p {
    margin: 0;
  }
  h3 {
    font-size: 15px;
  }
  h4 {
    font-size: 13px;
    margin-bottom: 4px;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .warn {
    color: var(--danger);
  }
  .muted {
    color: var(--text-muted);
    font-size: 12px;
  }
  .cmd {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .cmd em {
    color: var(--text-muted);
    font-style: normal;
  }
  .role {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .seg {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .seg .on {
    border-color: var(--accent);
    color: var(--accent);
  }
  .cmp {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }
  pre {
    margin: 0;
    padding: 6px;
    max-height: 140px;
    overflow: auto;
    white-space: pre-wrap;
    background: var(--input-bg, rgba(0, 0, 0, 0.2));
    border-radius: 6px;
    font-size: 11px;
  }
  .kv {
    display: flex;
    gap: 8px;
    align-items: baseline;
  }
  .mono,
  code {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
  }
  .block {
    overflow-wrap: anywhere;
  }
  label.check {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  input:not([type="checkbox"]) {
    font: inherit;
    color: var(--text);
    background: var(--input-bg, transparent);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 5px 8px;
  }
  button {
    font: inherit;
    color: inherit;
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 4px 10px;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .icon {
    border: none;
  }
  .primary {
    background: var(--accent);
    color: var(--on-accent, #fff);
    border-color: var(--accent);
  }
</style>
