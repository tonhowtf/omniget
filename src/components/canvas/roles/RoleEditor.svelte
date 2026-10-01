<script lang="ts">
  import { t } from "$lib/i18n";
  import { roles } from "$lib/canvas/roles/store.svelte";
  import { errorText, type RoleInfo, type RoleScope } from "$lib/canvas/roles/client";
  import { ICON_NAMES, ROLE_COLORS } from "$lib/canvas/roles/icons";
  import type { AgentKind } from "$lib/canvas/model/types";
  import RoleBadge from "./RoleBadge.svelte";

  let {
    role = null,
    onsaved,
    oncancel,
  }: {
    role?: RoleInfo | null;
    onsaved?: (id: string) => void;
    oncancel?: () => void;
  } = $props();

  const AGENTS: AgentKind[] = ["claude", "codex", "gemini", "opencode", "ollama", "shell", "native"];

  // svelte-ignore state_referenced_locally
  const initial = role;
  let name = $state(initial?.name ?? "");
  let summary = $state(initial?.summary ?? "");
  let prompt = $state(initial?.prompt ?? "");
  let icon = $state(initial?.icon ?? "user");
  let color = $state(initial?.color ?? ROLE_COLORS[7]);
  let agent = $state<AgentKind | "">(initial?.agent ?? "");
  let scope = $state<RoleScope>(initial?.scope ?? "workspace");
  let saving = $state(false);
  let error = $state("");

  let formEl = $state<HTMLFormElement | null>(null);
  $effect(() => {
    const f = formEl;
    if (!f) return;
    f.addEventListener("keydown", onkey);
    return () => f.removeEventListener("keydown", onkey);
  });

  const valid = $derived(name.trim().length > 0 && prompt.trim().length > 0);

  async function save() {
    if (!valid || saving) return;
    saving = true;
    error = "";
    try {
      const r = await roles.save(scope, {
        id: initial?.id,
        name: name.trim(),
        prompt,
        summary: summary.trim() || undefined,
        icon: icon.trim() || undefined,
        color: color || undefined,
        agent: agent || undefined,
      });
      onsaved?.(r.id);
    } catch (e) {
      error = errorText(e);
    } finally {
      saving = false;
    }
  }

  function onkey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      void save();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      oncancel?.();
    }
  }
</script>

<form class="editor" bind:this={formEl} onsubmit={(e) => { e.preventDefault(); void save(); }}>
  <div class="head">
    <RoleBadge {icon} {color} size={32} />
    <input
      class="name"
      bind:value={name}
      maxlength="80"
      placeholder={$t("canvas.roles.editor.name", { default: "Role name" })}
      aria-label={$t("canvas.roles.editor.name", { default: "Role name" })}
    />
  </div>

  <input
    class="field"
    bind:value={summary}
    maxlength="140"
    placeholder={$t("canvas.roles.editor.summary", { default: "One line: what this role is for" })}
    aria-label={$t("canvas.roles.editor.summary", { default: "One line: what this role is for" })}
  />

  <div class="row">
    <span class="label">{$t("canvas.roles.editor.icon", { default: "Icon" })}</span>
    <div class="icons">
      {#each ICON_NAMES as i (i)}
        <button type="button" class="swatch" class:on={icon === i} aria-label={i} aria-pressed={icon === i} onclick={() => (icon = i)}>
          <RoleBadge icon={i} color={icon === i ? color : undefined} size={24} />
        </button>
      {/each}
    </div>
  </div>

  <div class="row">
    <span class="label">{$t("canvas.roles.editor.color", { default: "Color" })}</span>
    <div class="colors">
      {#each ROLE_COLORS as c (c)}
        <button type="button" class="color" class:on={color === c} style:background={c} aria-label={c} aria-pressed={color === c} onclick={() => (color = c)}></button>
      {/each}
    </div>
  </div>

  <div class="grid">
    <label>
      <span class="label">{$t("canvas.roles.editor.agent", { default: "Default agent" })}</span>
      <select bind:value={agent}>
        <option value="">{$t("canvas.roles.editor.agent_any", { default: "Any" })}</option>
        {#each AGENTS as a (a)}
          <option value={a}>{$t(`canvas.term.agent.${a}`, { default: a })}</option>
        {/each}
      </select>
    </label>
    <label>
      <span class="label">{$t("canvas.roles.editor.scope", { default: "Available in" })}</span>
      <select bind:value={scope}>
        <option value="workspace">{$t("canvas.roles.scope.workspace", { default: "This workspace" })}</option>
        <option value="global">{$t("canvas.roles.scope.global", { default: "Library" })}</option>
      </select>
    </label>
  </div>

  <label class="prompt">
    <span class="label">{$t("canvas.roles.editor.instructions", { default: "Instructions" })}</span>
    <textarea
      bind:value={prompt}
      rows="12"
      spellcheck="false"
      placeholder={$t("canvas.roles.editor.instructions_hint", { default: "What this agent should focus on, who it talks to, and what done means." })}
    ></textarea>
  </label>
  <p class="hint">{$t("canvas.roles.editor.restart_hint", { default: "Changing the instructions restarts the terminals that use this role." })}</p>

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <div class="actions">
    <button type="button" class="ghost" onclick={() => oncancel?.()}>{$t("canvas.roles.cancel", { default: "Cancel" })}</button>
    <button type="submit" class="primary" disabled={!valid || saving}>
      {saving ? $t("canvas.roles.saving", { default: "Saving…" }) : $t("canvas.roles.save", { default: "Save" })}
    </button>
  </div>
</form>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .name {
    flex: 1;
    min-width: 0;
    font-size: 17px;
    font-weight: 600;
    padding: 6px 8px;
    border-radius: 8px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--text);
  }
  .name:hover,
  .name:focus {
    border-color: var(--border);
  }
  .field,
  select,
  textarea {
    padding: 7px 10px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--input-bg, transparent);
    color: var(--text);
    font-size: 13px;
  }
  textarea {
    resize: vertical;
    font: 12.5px/1.5 var(--font-mono, ui-monospace, monospace);
    min-height: 160px;
  }
  .row {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }
  .icons,
  .colors {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .swatch {
    all: unset;
    padding: 2px;
    border-radius: 8px;
    cursor: pointer;
    border: 1px solid transparent;
  }
  .swatch.on {
    border-color: var(--accent);
  }
  .color {
    all: unset;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    cursor: pointer;
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.15);
  }
  .color.on {
    outline: 2px solid var(--text);
    outline-offset: 2px;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .grid label,
  .prompt {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .hint {
    margin: -4px 0 0;
    font-size: 12px;
    color: var(--text-muted);
  }
  .error {
    margin: 0;
    font-size: 12px;
    color: var(--danger, #e5484d);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .actions button {
    all: unset;
    padding: 7px 14px;
    border-radius: 8px;
    font-size: 13px;
    cursor: pointer;
  }
  .ghost {
    color: var(--text-muted);
  }
  .ghost:hover {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  .primary {
    background: var(--accent);
    color: var(--on-accent, #fff);
    font-weight: 600;
  }
  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
