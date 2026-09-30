<script lang="ts">
  // Masthead search (spec §2.2): 40px pill + 64px search button on the video
  // face, the rounded 480px box on the music face. Suggestions come from the
  // backend, recent searches from localStorage (clock icon, "Remove"). A pasted
  // YouTube link opens directly.
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { agentActions } from "$lib/native-agent/actions";
  import * as api from "$lib/tube/api";
  import { onFocusSearch } from "$lib/tube/keys.svelte";
  import { current, go, routeFromYoutubeUrl, ui, type Face } from "$lib/tube/ui.svelte";
  import { openVideo } from "$lib/tube/video.svelte";
  import TIcon from "../TIcon.svelte";

  let { face }: { face: Face } = $props();

  let input = $state<HTMLInputElement>();
  let q = $state("");
  let focused = $state(false);
  let open = $state(false);
  let sel = $state(-1);
  let sugg = $state<string[]>([]);
  let recent = $state<string[]>([]);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let seq = 0;

  const key = () => `omnitube.searches.${face}`;

  function loadRecent() {
    try {
      recent = JSON.parse(localStorage.getItem(key()) || "[]");
    } catch {
      recent = [];
    }
  }
  function saveRecent(list: string[]) {
    recent = list.slice(0, 12);
    try {
      localStorage.setItem(key(), JSON.stringify(recent));
    } catch {
      /* storage off */
    }
  }

  onMount(() => onFocusSearch(() => input?.focus()));

  // keep the box in sync with the route (back/forward, face switch)
  $effect(() => {
    void ui.nav;
    const r = current();
    loadRecent();
    q = r.name === "search" || r.name === "m_search" ? r.q : "";
  });

  let rows = $derived.by(() => {
    const s = q.trim().toLowerCase();
    const hist = recent.filter((r) => !s || r.toLowerCase().startsWith(s)).slice(0, s ? 3 : 10);
    const rest = sugg.filter((x) => !hist.some((h) => h.toLowerCase() === x.toLowerCase())).slice(0, 12 - hist.length);
    return [...hist.map((text) => ({ text, recent: true })), ...rest.map((text) => ({ text, recent: false }))];
  });

  function typed() {
    open = true;
    sel = -1;
    if (timer) clearTimeout(timer);
    const text = q.trim();
    if (!text || /^https?:\/\//i.test(text)) {
      sugg = [];
      return;
    }
    timer = setTimeout(async () => {
      const my = ++seq;
      try {
        const r = await api.suggest(text, face === "music");
        if (my === seq) sugg = r;
      } catch {
        if (my === seq) sugg = [];
      }
    }, 120);
  }

  function submit(text = q) {
    const s = text.trim();
    if (!s) return;
    open = false;
    input?.blur();
    const link = /^(https?:\/\/)?(www\.|m\.|music\.)?(youtube\.com|youtu\.be)\//i.test(s) ? routeFromYoutubeUrl(s.startsWith("http") ? s : `https://${s}`) : null;
    if (link) {
      q = "";
      if (link.name === "watch") openVideo(link.id, { list: link.list, t: link.t });
      else go(link);
      return;
    }
    q = s;
    saveRecent([s, ...recent.filter((r) => r.toLowerCase() !== s.toLowerCase())]);
    go(face === "music" ? { name: "m_search", q: s } : { name: "search", q: s });
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (!rows.length) return;
      e.preventDefault();
      open = true;
      const n = rows.length;
      sel = e.key === "ArrowDown" ? (sel + 1) % n : (sel - 1 + n) % n;
    } else if (e.key === "Enter") {
      e.preventDefault();
      submit(sel >= 0 ? rows[sel]?.text : q);
    } else if (e.key === "Escape") {
      if (open) {
        e.stopPropagation();
        open = false;
      } else input?.blur();
    }
  }

  function remove(text: string) {
    saveRecent(recent.filter((r) => r !== text));
    input?.focus();
  }

  /** "how to" + typed "how" -> "how" normal, " to" bold, like YouTube. */
  function split(text: string): [string, string] {
    const s = q.trim();
    return s && text.toLowerCase().startsWith(s.toLowerCase()) ? [text.slice(0, s.length), text.slice(s.length)] : ["", text];
  }
</script>

<form
  class="sb"
  class:music={face === "music"}
  class:focused
  role="search"
  onsubmit={(e) => {
    e.preventDefault();
    submit();
  }}
>
  <div class="box">
    {#if focused || face === "music"}<span class="lead"><TIcon name="search" size={20} /></span>{/if}
    <input
      use:agentActions={["fill"]}
      bind:this={input}
      bind:value={q}
      type="text"
      spellcheck="false"
      autocomplete="off"
      enterkeyhint="search"
      placeholder={$t(face === "music" ? "tube.shell.search_music" : "tube.shell.search") as string}
      aria-label={$t("tube.shell.search") as string}
      aria-expanded={open && rows.length > 0}
      aria-controls="tube-sugg"
      aria-autocomplete="list"
      role="combobox"
      oninput={typed}
      onkeydown={keydown}
      onfocus={() => {
        focused = true;
        open = true;
        loadRecent();
      }}
      onblur={() => {
        focused = false;
        setTimeout(() => (open = false), 150);
      }}
    />
    {#if q}
      <button type="button" class="clear" onclick={() => ((q = ""), (sugg = []), input?.focus())} aria-label={$t("tube.shell.clear") as string}>
        <TIcon name="close" size={20} />
      </button>
    {/if}
    {#if open && rows.length}
      <ul class="sugg" id="tube-sugg" role="listbox">
        {#each rows as r, i (r.text + i)}
          {@const [head, tail] = split(r.text)}
          <li role="option" aria-selected={i === sel} class:sel={i === sel}>
            <button type="button" class="opt" onmousedown={(e) => e.preventDefault()} onclick={() => submit(r.text)}>
              <TIcon name={r.recent ? "history" : "search"} size={18} />
              <span class="txt">{head}<b>{tail}</b></span>
            </button>
            {#if r.recent}
              <button type="button" class="rm" onmousedown={(e) => e.preventDefault()} onclick={() => remove(r.text)}>{$t("tube.shell.remove")}</button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
  {#if face !== "music"}
    <button use:agentActions={["click"]} type="submit" class="go" aria-label={$t("tube.shell.search") as string} title={$t("tube.shell.search") as string}>
      <TIcon name="search" size={22} />
    </button>
  {/if}
</form>

<style>
  .sb {
    display: flex;
    flex: 1;
    max-width: var(--yt-search-max-w);
    min-width: 0;
    height: var(--yt-search-h);
  }
  .box {
    position: relative;
    display: flex;
    align-items: center;
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 4px 0 16px;
    border: 1px solid var(--yt-search-border);
    border-right: 0;
    border-radius: 40px 0 0 40px;
    background: var(--yt-search-bg);
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.12);
    transition: border-color var(--yt-dur-fast);
  }
  .focused .box {
    border-color: var(--yt-link);
    padding-left: 12px;
  }
  .focused .box::after {
    content: "";
    position: absolute;
    inset: -1px 0 -1px -1px;
    border-radius: inherit;
    border: 1px solid var(--yt-link);
    border-right: 0;
    pointer-events: none;
  }
  .lead {
    display: grid;
    place-items: center;
    margin-right: 10px;
    color: var(--yt-text);
    flex: none;
  }
  input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--yt-text);
    font: 400 16px/22px var(--yt-font);
  }
  input::placeholder {
    color: var(--yt-text-2);
  }
  .clear {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--yt-text);
    cursor: pointer;
    flex: none;
  }
  .clear:hover {
    background: var(--yt-hover);
  }
  .go {
    display: grid;
    place-items: center;
    width: 64px;
    height: 100%;
    border: 1px solid var(--yt-search-border);
    border-radius: 0 40px 40px 0;
    background: var(--yt-search-btn);
    color: var(--yt-text);
    cursor: pointer;
    flex: none;
    transition: background var(--yt-dur-base);
  }
  .go:hover {
    background: var(--yt-tonal-hover);
  }
  .go:focus-visible,
  .clear:focus-visible {
    outline: 2px solid var(--yt-link);
    outline-offset: 1px;
  }
  .sugg {
    position: absolute;
    left: -1px;
    right: -65px;
    top: calc(100% + 4px);
    margin: 0;
    padding: 16px 0;
    list-style: none;
    border-radius: var(--yt-radius-menu);
    background: var(--yt-bg-menu);
    box-shadow: 0 4px 32px rgba(0, 0, 0, 0.1), 0 16px 24px var(--yt-shadow);
    z-index: 50;
    animation: pop var(--yt-dur-fast) var(--yt-ease-enter);
  }
  @keyframes pop {
    from {
      opacity: 0;
    }
  }
  li {
    display: flex;
    align-items: center;
  }
  li:hover,
  li.sel {
    background: var(--yt-tonal);
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 14px;
    flex: 1;
    min-width: 0;
    height: 32px;
    padding: 0 16px;
    border: 0;
    background: transparent;
    color: var(--yt-text);
    font: 400 16px/22px var(--yt-font);
    text-align: left;
    cursor: pointer;
  }
  .txt {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .txt b {
    font-weight: 700;
  }
  .rm {
    margin-right: 16px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--yt-link);
    font: 400 13px/18px var(--yt-font);
    cursor: pointer;
    opacity: 0.9;
  }
  .rm:hover {
    text-decoration: underline;
  }

  /* music face: 480px rounded box (spec §3.1) */
  .music {
    max-width: 480px;
  }
  .music .box {
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.1);
    box-shadow: none;
    padding-left: 12px;
  }
  .music.focused .box {
    background: var(--ytm-bg-raised);
    border-color: rgba(255, 255, 255, 0.2);
  }
  .music.focused .box::after {
    display: none;
  }
  .music .sugg {
    right: -1px;
    background: var(--ytm-bg-raised);
  }
  .music .lead {
    color: var(--ytm-text-2);
  }
  @media (prefers-reduced-motion: reduce) {
    .sugg {
      animation: none;
    }
  }
</style>
