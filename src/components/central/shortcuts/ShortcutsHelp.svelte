<script lang="ts">
  /**
   * Ajuda de atalhos (overlay aberto com `?`): a tabela inteira agrupada, com
   * as teclas no formato do SO e a condição `when` de cada uma. Filtro por
   * texto; Esc ou `?` fecham.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { COMMANDS, commandInfo, commandLabelKey, keyLabel, type ShortcutGroup, type ShortcutRule } from "$lib/central/shortcuts";

  let {
    rules,
    isMac,
    available,
    onclose,
  }: { rules: ShortcutRule[]; isMac: boolean; available: (command: string) => boolean; onclose: () => void } = $props();

  const GROUPS: ShortcutGroup[] = ["threads", "panel", "composer", "approvals", "navigation", "terminal", "app"];
  let filter = $state("");
  let inputEl = $state<HTMLInputElement | null>(null);

  type Row = { command: string; label: string; keys: string[]; whens: string[]; native: boolean; live: boolean };

  let rows = $derived.by(() => {
    const byCmd = new Map<string, Row>();
    for (const r of rules) {
      const cmd = r.command.startsWith("thread.jump.") ? "thread.jump" : r.command;
      let row = byCmd.get(cmd);
      if (!row) {
        const info = commandInfo(r.command);
        row = {
          command: cmd,
          label: $t(commandLabelKey(r.command)) as string,
          keys: [],
          whens: [],
          native: !!info?.native,
          live: available(r.command),
        };
        byCmd.set(cmd, row);
      }
      const label = cmd === "thread.jump" ? keyLabel("mod+1", isMac).replace(/1$/, "1…9") : keyLabel(r.key, isMac);
      if (!row.keys.includes(label)) row.keys.push(label);
      if (r.when && !row.whens.includes(r.when)) row.whens.push(r.when);
      row.live = row.live || available(r.command);
    }
    return [...byCmd.values()];
  });

  let grouped = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return GROUPS.map((g) => ({
      group: g,
      rows: rows.filter((r) => {
        const info = COMMANDS.find((c) => c.id === r.command || (r.command === "thread.jump" && c.id.startsWith("thread.jump.")));
        if ((info?.group ?? "app") !== g) return false;
        return !q || r.label.toLowerCase().includes(q) || r.keys.some((k) => k.toLowerCase().includes(q)) || r.command.includes(q);
      }),
    })).filter((g) => g.rows.length > 0);
  });

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape" || (e.key === "?" && !filter)) {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    }
  }

  onMount(() => inputEl?.focus());
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="backdrop" role="presentation" data-central-overlay onclick={onclose}>
  <div class="help" role="dialog" aria-modal="true" aria-label={$t("llm.central.shortcuts.help.title")} tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={onkey}>
    <header>
      <h2>{$t("llm.central.shortcuts.help.title")}</h2>
      <input bind:this={inputEl} bind:value={filter} class="input" type="search" placeholder={$t("llm.central.shortcuts.help.filter") as string} />
      <button class="button" type="button" onclick={onclose}>{$t("llm.central.shortcuts.help.close")}</button>
    </header>
    <div class="body">
      {#each grouped as g (g.group)}
        <section>
          <h3>{$t(`llm.central.shortcuts.group.${g.group}`)}</h3>
          <table>
            <tbody>
              {#each g.rows as r (r.command)}
                <tr class:dim={!r.live && !r.native}>
                  <td class="label">{r.label}</td>
                  <td class="keys">{#each r.keys as k (k)}<kbd>{k}</kbd>{/each}</td>
                  <td class="when" title={r.whens.join("\n")}>
                    {#if r.whens.length}<code>{r.whens[0]}</code>{#if r.whens.length > 1}<span> +{r.whens.length - 1}</span>{/if}{/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </section>
      {/each}
    </div>
    <footer>{$t("llm.central.shortcuts.help.footer")}</footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 1000;
    background: var(--dialog-backdrop, rgba(0, 0, 0, 0.35));
    display: flex;
    justify-content: center;
    align-items: center;
    padding: 16px;
  }
  .help {
    width: min(880px, 100%);
    max-height: min(86vh, 900px);
    background: var(--popup-bg, var(--surface));
    color: var(--text);
    border-radius: var(--radius-xl, 14px);
    box-shadow: var(--elev-3);
    border: 1px solid var(--separator);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 14px 16px;
    border-bottom: 1px solid var(--separator);
  }
  h2 {
    margin: 0;
    font-size: var(--text-md);
    flex: 1;
  }
  header .input {
    width: 220px;
  }
  .body {
    overflow-y: auto;
    padding: 8px 16px 16px;
    columns: 2 380px;
    column-gap: 28px;
  }
  section {
    break-inside: avoid;
    margin-bottom: 12px;
  }
  h3 {
    margin: 8px 0 4px;
    font-size: var(--text-caption);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
  }
  td {
    padding: 3px 0;
    vertical-align: middle;
  }
  tr.dim {
    opacity: 0.55;
  }
  .label {
    padding-right: 8px;
  }
  .keys {
    white-space: nowrap;
    text-align: right;
  }
  .when {
    width: 1%;
    white-space: nowrap;
    padding-left: 8px;
    color: var(--text-dim);
    font-size: var(--text-caption);
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  code {
    font-family: var(--font-mono);
    font-size: 10px;
  }
  kbd {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    background: var(--fill-1);
    border-radius: 5px;
    padding: 1px 6px;
    margin-left: 4px;
  }
  footer {
    padding: 8px 16px;
    border-top: 1px solid var(--separator);
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
</style>
