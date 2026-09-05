<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t, locale } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import type { Champion } from "./shared";

  let {
    games,
    phase,
    championById,
    active,
  }: {
    games: any[];
    phase: string;
    championById: Map<number, Champion>;
    active?: boolean;
  } = $props();

  const LANGUAGE_NAMES: Record<string, string> = {
    en: "English",
    pt: "Brazilian Portuguese",
    es: "Spanish",
    fr: "French",
    it: "Italian",
    ja: "Japanese",
    ru: "Russian",
    el: "Greek",
    zh: "Simplified Chinese",
    "zh-TW": "Traditional Chinese",
    fa: "Persian",
  };

  let settings = $derived(getSettings());
  let style = $derived(settings?.league?.coach_style ?? "objective");
  let language = $derived(LANGUAGE_NAMES[$locale ?? "en"] ?? "English");

  let ready = $state<boolean | null>(null);
  let checked = false;
  $effect(() => {
    if (active !== false && !checked) {
      checked = true;
      invoke<boolean>("league_coach_ready").then((v) => (ready = v)).catch(() => (ready = false));
    }
  });

  let busy = $state<"" | "review" | "trends" | "ask">("");
  let output = $state("");
  let outputTitle = $state("");
  let error = $state("");
  let copied = $state(false);

  let selectedGame = $state<number>(0);
  let trendCount = $state(20);
  let question = $state("");

  $effect(() => {
    if (selectedGame === 0 && games.length > 0) selectedGame = games[0].gameId;
  });

  function errText(e: unknown): string {
    return typeof e === "string" ? e : ((e as any)?.message ?? String(e));
  }

  async function run(kind: "review" | "trends" | "ask", title: string, call: () => Promise<string>) {
    if (busy) return;
    busy = kind;
    error = "";
    output = "";
    outputTitle = title;
    try {
      output = await call();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = "";
    }
  }

  function review() {
    const game = games.find((g) => g.gameId === selectedGame);
    const label = game ? gameLabel(game) : String(selectedGame);
    run("review", `${$t("league.coach_review_title")} · ${label}`, () =>
      invoke<string>("league_coach_review", { gameId: selectedGame, style, language }),
    );
  }

  function trends() {
    run("trends", `${$t("league.coach_trends_title")} · ${trendCount}`, () =>
      invoke<string>("league_coach_trends", { count: trendCount, style, language }),
    );
  }

  function ask(text = question) {
    const q = text.trim();
    if (!q) return;
    question = q;
    run("ask", q, () => invoke<string>("league_coach_ask", { question: q, style, language }));
  }

  function gameLabel(game: any): string {
    const p = game?.participants?.[0];
    const champ = championById.get(p?.championId ?? 0)?.name ?? "?";
    const s = p?.stats ?? {};
    const result = s.win ? ($t("league.victory") as string) : ($t("league.defeat") as string);
    return `${champ} · ${s.kills ?? 0}/${s.deaths ?? 0}/${s.assists ?? 0} · ${result}`;
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(output);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      copied = false;
    }
  }

  const QUICK: { key: string; phases: readonly string[] }[] = [
    { key: "league.coach_quick_draft", phases: ["ChampSelect"] },
    { key: "league.coach_quick_early", phases: ["ChampSelect", "InProgress"] },
    { key: "league.coach_quick_now", phases: ["InProgress"] },
    { key: "league.coach_quick_improve", phases: [] },
  ];

  let quickPrompts = $derived(QUICK.filter((q) => q.phases.length === 0 || q.phases.includes(phase)));
</script>

{#if active !== false}
  <section class="card">
    <div class="card-head">
      <h3>{$t("league.coach_title")}</h3>
      <span class="feature-badge">{$t("league.badge_beta")}</span>
    </div>
    <p class="win-disclaimer">{$t("league.coach_desc")}</p>
    {#if ready === false}
      <div class="action-error" role="alert">
        {$t("league.coach_not_configured")}
        <a href="/settings">{$t("league.coach_open_settings")}</a>
      </div>
    {/if}
    <div class="action-row">
      <div class="action-col">
        <span class="action-label">{$t("league.coach_style")}</span>
      </div>
      <div class="seg-group" role="radiogroup" aria-label={$t("league.coach_style") as string}>
        {#each ["objective", "roast", "praise"] as mode (mode)}
          <button
            class="seg"
            class:on={style === mode}
            role="radio"
            aria-checked={style === mode}
            onclick={() => updateSettings({ league: { coach_style: mode } })}
          >{$t(`league.coach_style_${mode}`)}</button>
        {/each}
      </div>
    </div>
    <p class="action-hint">{$t("league.coach_privacy")}</p>
  </section>

  <section class="card">
    <div class="card-head"><h3>{$t("league.coach_ask_title")}</h3></div>
    {#if quickPrompts.length > 0}
      <div class="quick-row">
        {#each quickPrompts as q (q.key)}
          <button class="button subtle" disabled={!!busy || ready === false} onclick={() => ask($t(q.key) as string)}>{$t(q.key)}</button>
        {/each}
      </div>
    {/if}
    <form class="search-form" onsubmit={(e) => { e.preventDefault(); ask(); }}>
      <input class="input-text" placeholder={$t("league.coach_ask_placeholder") as string} bind:value={question} maxlength="2000" />
      <button class="button primary" type="submit" disabled={!!busy || ready === false || !question.trim()}>
        {busy === "ask" ? $t("league.coach_working") : $t("league.coach_ask_run")}
      </button>
    </form>
  </section>

  <section class="card">
    <div class="card-head"><h3>{$t("league.coach_review_title")}</h3></div>
    {#if games.length === 0}
      <p class="empty-hint">{$t("league.coach_error_no_games")}</p>
    {:else}
      <div class="profile-tool-row">
        <select class="select-role wide" bind:value={selectedGame} aria-label={$t("league.coach_review_pick") as string}>
          {#each games as game (game.gameId)}
            <option value={game.gameId}>{gameLabel(game)}</option>
          {/each}
        </select>
        <button class="button" disabled={!!busy || ready === false} onclick={review}>
          {busy === "review" ? $t("league.coach_working") : $t("league.coach_review_run")}
        </button>
      </div>
    {/if}
  </section>

  <section class="card">
    <div class="card-head"><h3>{$t("league.coach_trends_title")}</h3></div>
    <div class="slider-row">
      <span class="slider-edge">5</span>
      <input type="range" min="5" max="40" step="5" bind:value={trendCount} aria-label={$t("league.coach_trends_title") as string} />
      <span class="slider-edge">40</span>
      <span class="list-hint">{trendCount} {$t("league.coach_trends_games")}</span>
      <button class="button" disabled={!!busy || ready === false} onclick={trends}>
        {busy === "trends" ? $t("league.coach_working") : $t("league.coach_trends_run")}
      </button>
    </div>
  </section>

  {#if error}
    <p class="action-error" role="alert">{error}</p>
  {/if}
  {#if busy}
    <section class="card" aria-busy="true">
      <p class="dim">{$t("league.coach_working")}</p>
    </section>
  {:else if output}
    <section class="card">
      <div class="card-head">
        <h3>{$t("league.coach_result")}</h3>
        <button class="button subtle" onclick={copy}>{copied ? $t("league.coach_copied") : $t("league.coach_copy")}</button>
      </div>
      <p class="dim output-title">{outputTitle}</p>
      <div class="coach-output">{output}</div>
    </section>
  {/if}
{/if}

<style>
  .quick-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .coach-output {
    white-space: pre-wrap;
    line-height: 1.5;
    font-size: 13.5px;
  }
  .output-title {
    font-size: 12px;
  }
  :global(.select-role.wide) {
    flex: 1;
    min-width: 200px;
  }
</style>
