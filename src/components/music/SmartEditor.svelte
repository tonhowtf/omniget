<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import type { SmartQuery, SmartRule, Track } from "$lib/music/types";
  import MusicIcon from "./MusicIcon.svelte";

  let {
    name = $bindable(""),
    query = $bindable(),
    onsave,
    oncancel,
  }: {
    name?: string;
    query: SmartQuery;
    onsave: () => void;
    oncancel: () => void;
  } = $props();

  type Kind = "text" | "number" | "bool" | "date";
  const FIELDS: [string, Kind][] = [
    ["title", "text"],
    ["artist", "text"],
    ["album", "text"],
    ["album_artist", "text"],
    ["genre", "text"],
    ["composer", "text"],
    ["path", "text"],
    ["format", "text"],
    ["camelot", "text"],
    ["year", "number"],
    ["duration", "number"],
    ["bpm", "number"],
    ["rating", "number"],
    ["play_count", "number"],
    ["skip_count", "number"],
    ["bitrate", "number"],
    ["loudness", "number"],
    ["liked", "bool"],
    ["has_lyrics", "bool"],
    ["added", "date"],
    ["last_played", "date"],
  ];
  const OPS: Record<Kind, string[]> = {
    text: ["contains", "not_contains", "is", "is_not", "starts_with", "empty", "not_empty"],
    number: ["is", "is_not", "gt", "lt", "between", "empty"],
    bool: ["is_true", "is_false"],
    date: ["in_last_days", "not_in_last_days", "never"],
  };
  const ORDERS = ["random", "title", "artist", "album", "year", "added", "last_played", "play_count", "rating", "bpm", "duration"];

  const kindOf = (f: string): Kind => FIELDS.find((x) => x[0] === f)?.[1] ?? "text";
  const needsValue = (op: string) => !["empty", "not_empty", "is_true", "is_false", "never"].includes(op);

  let preview = $state<Track[] | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    const snapshot = JSON.stringify(query);
    if (timer) clearTimeout(timer);
    timer = setTimeout(async () => {
      try {
        preview = await invoke<Track[]>("music_smart_preview", { query: JSON.parse(snapshot) });
      } catch {
        preview = null;
      }
    }, 300);
    return () => {
      if (timer) clearTimeout(timer);
    };
  });

  function addRule() {
    query.rules = [...query.rules, { field: "genre", op: "contains", value: "" }];
  }

  function setField(r: SmartRule, f: string) {
    r.field = f;
    r.op = OPS[kindOf(f)][0];
    r.value = kindOf(f) === "date" ? 30 : "";
    query.rules = [...query.rules];
  }
</script>

<div class="se">
  <label class="field">
    <span class="field-label">{$t("music.playlist.name")}</span>
    <input class="input" bind:value={name} placeholder={$t("music.playlist.untitled") as string} />
  </label>

  <div class="match">
    <span>{$t("music.smart.match")}</span>
    <div class="mac-segmented">
      <button class="mac-segmented-btn" class:active={query.match_all} onclick={() => (query.match_all = true)}>{$t("music.smart.all")}</button>
      <button class="mac-segmented-btn" class:active={!query.match_all} onclick={() => (query.match_all = false)}>{$t("music.smart.any")}</button>
    </div>
    <span>{$t("music.smart.of_rules")}</span>
  </div>

  <ul class="rules">
    {#each query.rules as r, i (i)}
      {@const kind = kindOf(r.field)}
      <li>
        <select class="input" value={r.field} onchange={(e) => setField(r, (e.currentTarget as HTMLSelectElement).value)} aria-label={$t("music.smart.field") as string}>
          {#each FIELDS as [f] (f)}<option value={f}>{$t(`music.smart.f.${f}`)}</option>{/each}
        </select>
        <select class="input" bind:value={r.op} aria-label={$t("music.smart.op") as string}>
          {#each OPS[kind] as op (op)}<option value={op}>{$t(`music.smart.op_${op}`)}</option>{/each}
        </select>
        {#if needsValue(r.op)}
          <input class="input val" type={kind === "number" || kind === "date" ? "number" : "text"} bind:value={r.value} aria-label={$t("music.smart.value") as string} />
          {#if r.op === "between"}
            <span class="and">{$t("music.smart.and")}</span>
            <input class="input val" type="number" bind:value={r.value2} aria-label={$t("music.smart.value") as string} />
          {/if}
          {#if kind === "date"}<span class="and">{$t("music.smart.days")}</span>{/if}
        {/if}
        <button class="btn btn-icon btn-ghost" onclick={() => (query.rules = query.rules.filter((_, j) => j !== i))} aria-label={$t("music.remove") as string}><MusicIcon name="close" size={13} /></button>
      </li>
    {/each}
  </ul>
  <button class="btn btn-sm add" onclick={addRule}><MusicIcon name="plus" size={13} /> {$t("music.smart.add_rule")}</button>

  <div class="order">
    <label>
      <span>{$t("music.smart.order")}</span>
      <select class="input" bind:value={query.order}>
        <option value={null}>—</option>
        {#each ORDERS as o (o)}<option value={o}>{$t(`music.smart.o.${o}`)}</option>{/each}
      </select>
    </label>
    <label class="chk"><input type="checkbox" bind:checked={query.descending} /> {$t("music.smart.desc")}</label>
    <label>
      <span>{$t("music.smart.limit")}</span>
      <input class="input lim" type="number" min="0" bind:value={query.limit} placeholder="∞" />
    </label>
  </div>

  <footer>
    <span class="count">{preview ? $t("music.smart.matches", { count: preview.length }) : ""}</span>
    <button class="btn" onclick={oncancel}>{$t("music.cancel")}</button>
    <button class="btn btn-primary" onclick={onsave}>{$t("music.save")}</button>
  </footer>
</div>

<style>
  .se {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .match {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .rules {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .rules li {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .rules select {
    width: auto;
    min-width: 120px;
  }
  .val {
    width: 150px;
  }
  .and {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .add {
    align-self: flex-start;
  }
  .order {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    font-size: var(--text-sm);
  }
  .order label {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .lim {
    width: 80px;
  }
  .chk input {
    accent-color: var(--accent);
  }
  footer {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    justify-content: flex-end;
  }
  .count {
    flex: 1;
    font-size: var(--text-sm);
    color: var(--accent-hi);
  }
</style>
