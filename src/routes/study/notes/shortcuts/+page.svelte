<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import PageHero from "$lib/study-components/PageHero.svelte";

  let isMac = $state(false);

  onMount(() => {
    if (typeof navigator !== "undefined") {
      isMac = /Mac|iPhone|iPad|iPod/i.test(navigator.platform || navigator.userAgent || "");
    }
  });

  const meta = $derived(isMac ? "Cmd" : "Ctrl");

  type Row = { keys: string[]; desc: string };
  type Section = { title: string; rows: Row[] };

  const SECTIONS = $derived([
    {
      title: $t("study.notes.shortcuts.sec_structure"),
      rows: [
        { keys: ["Tab"], desc: $t("study.notes.shortcuts.indent") },
        { keys: ["Shift+Tab"], desc: $t("study.notes.shortcuts.outdent") },
        { keys: ["Alt+↑"], desc: $t("study.notes.shortcuts.move_up") },
        { keys: ["Alt+↓"], desc: $t("study.notes.shortcuts.move_down") },
        { keys: [`${meta}+Shift+K`], desc: $t("study.notes.shortcuts.delete_block") },
        { keys: [`${meta}+/`], desc: $t("study.notes.shortcuts.collapse") },
        { keys: [`${meta}+D`], desc: $t("study.notes.shortcuts.duplicate") },
      ],
    },
    {
      title: $t("study.notes.shortcuts.sec_todo"),
      rows: [
        { keys: [`${meta}+Enter`], desc: $t("study.notes.shortcuts.cycle_status") },
      ],
    },
    {
      title: $t("study.notes.shortcuts.sec_inline"),
      rows: [
        { keys: [`${meta}+B`], desc: $t("study.notes.shortcuts.bold") },
        { keys: [`${meta}+I`], desc: $t("study.notes.shortcuts.italic") },
        { keys: [`${meta}+Shift+S`], desc: $t("study.notes.shortcuts.strike") },
        { keys: [`${meta}+Shift+C`], desc: $t("study.notes.shortcuts.code_inline") },
        { keys: [`${meta}+Shift+.`], desc: $t("study.notes.shortcuts.blockquote") },
      ],
    },
    {
      title: $t("study.notes.shortcuts.sec_slash"),
      rows: [
        { keys: ["/"], desc: $t("study.notes.shortcuts.slash_open") },
        { keys: ["/todo /doing /done /later /now /waiting /canceled"], desc: $t("study.notes.shortcuts.slash_status") },
        { keys: ["/today"], desc: $t("study.notes.shortcuts.slash_today") },
        { keys: ["/date"], desc: $t("study.notes.shortcuts.slash_date") },
        { keys: ["/page /tag /block"], desc: $t("study.notes.shortcuts.slash_page_tag_block") },
        { keys: ["/code"], desc: $t("study.notes.shortcuts.slash_code") },
        { keys: ["/query"], desc: $t("study.notes.shortcuts.slash_query", { skeleton: "{{query (and (todo TODO))}}" }) },
        { keys: ["/embed page", "/embed block"], desc: $t("study.notes.shortcuts.slash_embed", { a: "{{embed [[…]]}}", b: "{{embed ((…))}}" }) },
      ],
    },
    {
      title: $t("study.notes.shortcuts.sec_autocomplete"),
      rows: [
        { keys: ["[["], desc: $t("study.notes.shortcuts.ac_pages") },
        { keys: ["#"], desc: $t("study.notes.shortcuts.ac_tags") },
        { keys: ["(("], desc: $t("study.notes.shortcuts.ac_blocks") },
      ],
    },
    {
      title: $t("study.notes.shortcuts.sec_history"),
      rows: [
        { keys: [`${meta}+Z`], desc: $t("study.notes.shortcuts.undo_content") },
        { keys: [`${meta}+Alt+Z`], desc: $t("study.notes.shortcuts.undo_structural") },
        { keys: [`${meta}+Shift+Z`, `${meta}+Y`], desc: $t("study.notes.shortcuts.redo_structural") },
      ],
    },
    {
      title: $t("study.notes.shortcuts.sec_exit"),
      rows: [
        { keys: ["Esc"], desc: $t("study.notes.shortcuts.esc") },
      ],
    },
    {
      title: $t("study.notes.shortcuts.sec_md_preview"),
      rows: [
        { keys: ["`> [!note]` `[!warn]` `[!info]` `[!success]` `[!tip]`"], desc: $t("study.notes.shortcuts.callout") },
        { keys: ["` ```lang `\\n`code`\\n` ``` "], desc: $t("study.notes.shortcuts.code_block") },
        { keys: [$t("study.notes.shortcuts.latex_sample")], desc: $t("study.notes.shortcuts.latex") },
        { keys: ["`| col1 | col2 |`\\n`|---|---|`\\n`|...|...|`"], desc: $t("study.notes.shortcuts.table") },
        { keys: ["`{{query (...)}}` `:sort X :limit N :offset M`"], desc: $t("study.notes.shortcuts.query_inline") },
      ],
    },
    {
      title: $t("study.notes.shortcuts.sec_search"),
      rows: [
        { keys: ["`tag:project`"], desc: $t("study.notes.shortcuts.search_tag") },
        { keys: ["`page:Daily`"], desc: $t("study.notes.shortcuts.search_page") },
        { keys: ["`status:DOING`"], desc: $t("study.notes.shortcuts.search_status") },
        { keys: ["`before:2026-05-01`", "`after:2026-04-01`"], desc: $t("study.notes.shortcuts.search_date") },
        { keys: ["`tag:\"two words\"`"], desc: $t("study.notes.shortcuts.search_quotes") },
      ],
    },
  ]);
</script>

<section class="shortcuts-page">
  <PageHero
    title={$t("study.notes.shortcuts.page_title")}
    subtitle={$t("study.notes.shortcuts.detected", {
      platform: isMac ? "Mac" : "Windows/Linux",
      meta,
    })}
  />

  <p class="muted small">
    {$t("study.notes.shortcuts.intro")}
  </p>

  {#each SECTIONS as section (section.title)}
    <section class="sec">
      <h2>{section.title}</h2>
      <table class="sc-table">
        <tbody>
          {#each section.rows as row (row.desc)}
            <tr>
              <td class="keys-cell">
                {#each row.keys as k, i (i)}
                  {#if i > 0} {$t("study.notes.shortcuts.or")} {/if}
                  {#each k.split("+") as part, j (j)}
                    {#if j > 0}<span class="plus">+</span>{/if}
                    <kbd>{part}</kbd>
                  {/each}
                {/each}
              </td>
              <td class="desc-cell">{row.desc}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/each}
</section>

<style>
  .shortcuts-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.25);
    width: 100%;
    max-width: 880px;
    margin-inline: auto;
  }
  .muted {
    color: var(--tertiary);
  }
  .small {
    font-size: 12px;
  }
  .sec {
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    padding: calc(var(--padding) * 0.9);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .sec h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--accent);
  }
  .sc-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  .sc-table td {
    padding: 6px 8px;
    border-bottom: none;
    vertical-align: top;
  }
  .sc-table tr:last-child td {
    border-bottom: 0;
  }
  .keys-cell {
    width: 35%;
    white-space: nowrap;
  }
  .desc-cell {
    color: var(--secondary);
  }
  kbd {
    display: inline-block;
    padding: 2px 6px;
    background: var(--bg);
    border: none;
    border-bottom-width: 2px;
    border-radius: 4px;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: var(--text);
    line-height: 1;
  }
  .plus {
    margin: 0 2px;
    color: var(--tertiary);
    font-size: 11px;
  }
  code {
    padding: 1px 4px;
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border-radius: 3px;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
  }
</style>
