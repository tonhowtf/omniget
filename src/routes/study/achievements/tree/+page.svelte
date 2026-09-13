<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import PageHero from "$lib/study-components/PageHero.svelte";

  type Achievement = {
    code: string;
    tier: string;
    unlocked_at: number;
  };

  type GamificationState = {
    xp: number;
    level: number;
  };

  type TreeNode = {
    code: string;
    label: string;
    description: string;
    family: "xp" | "lessons" | "focus" | "streak";
    tier: "bronze" | "silver" | "gold";
    threshold: number;
    counterKey?: string;
    icon: string;
    requires?: string;
  };

  const TREE_NODES = $derived([
    { code: "xp:100", label: $t("study.achv.tree.xp_100_label"), description: $t("study.achv.tree.xp_100_desc"), family: "xp", tier: "bronze", threshold: 100, icon: "✨" },
    { code: "xp:500", label: $t("study.achv.tree.xp_500_label"), description: $t("study.achv.tree.xp_500_desc"), family: "xp", tier: "bronze", threshold: 500, icon: "⭐", requires: "xp:100" },
    { code: "xp:1000", label: $t("study.achv.tree.xp_1000_label"), description: $t("study.achv.tree.xp_1000_desc"), family: "xp", tier: "silver", threshold: 1000, icon: "🌟", requires: "xp:500" },
    { code: "xp:5000", label: $t("study.achv.tree.xp_5000_label"), description: $t("study.achv.tree.xp_5000_desc"), family: "xp", tier: "silver", threshold: 5000, icon: "💫", requires: "xp:1000" },
    { code: "xp:10000", label: $t("study.achv.tree.xp_10000_label"), description: $t("study.achv.tree.xp_10000_desc"), family: "xp", tier: "gold", threshold: 10000, icon: "🏆", requires: "xp:5000" },

    { code: "lessons:1", label: $t("study.achv.tree.lessons_1_label"), description: $t("study.achv.tree.lessons_1_desc"), family: "lessons", tier: "bronze", threshold: 1, counterKey: "lessons_completed", icon: "🎓" },
    { code: "lessons:10", label: $t("study.achv.tree.lessons_10_label"), description: $t("study.achv.tree.lessons_10_desc"), family: "lessons", tier: "bronze", threshold: 10, counterKey: "lessons_completed", icon: "📚", requires: "lessons:1" },
    { code: "lessons:50", label: $t("study.achv.tree.lessons_50_label"), description: $t("study.achv.tree.lessons_50_desc"), family: "lessons", tier: "silver", threshold: 50, counterKey: "lessons_completed", icon: "📖", requires: "lessons:10" },
    { code: "lessons:100", label: $t("study.achv.tree.lessons_100_label"), description: $t("study.achv.tree.lessons_100_desc"), family: "lessons", tier: "gold", threshold: 100, counterKey: "lessons_completed", icon: "🥇", requires: "lessons:50" },

    { code: "focus:60", label: $t("study.achv.tree.focus_60_label"), description: $t("study.achv.tree.focus_60_desc"), family: "focus", tier: "bronze", threshold: 60, counterKey: "focus_minutes", icon: "🧘" },
    { code: "focus:600", label: $t("study.achv.tree.focus_600_label"), description: $t("study.achv.tree.focus_600_desc"), family: "focus", tier: "silver", threshold: 600, counterKey: "focus_minutes", icon: "🎯", requires: "focus:60" },
    { code: "focus:6000", label: $t("study.achv.tree.focus_6000_label"), description: $t("study.achv.tree.focus_6000_desc"), family: "focus", tier: "gold", threshold: 6000, counterKey: "focus_minutes", icon: "🔥", requires: "focus:600" },

    { code: "streak:3", label: $t("study.achv.tree.streak_3_label"), description: $t("study.achv.tree.streak_3_desc"), family: "streak", tier: "bronze", threshold: 3, icon: "🔥" },
    { code: "streak:7", label: $t("study.achv.tree.streak_7_label"), description: $t("study.achv.tree.streak_7_desc"), family: "streak", tier: "bronze", threshold: 7, icon: "🔥🔥", requires: "streak:3" },
    { code: "streak:30", label: $t("study.achv.tree.streak_30_label"), description: $t("study.achv.tree.streak_30_desc"), family: "streak", tier: "silver", threshold: 30, icon: "🔥🔥🔥", requires: "streak:7" },
    { code: "streak:100", label: $t("study.achv.tree.streak_100_label"), description: $t("study.achv.tree.streak_100_desc"), family: "streak", tier: "gold", threshold: 100, icon: "🏅", requires: "streak:30" },
  ]);

  type Family = "xp" | "lessons" | "focus" | "streak";
  const FAMILIES = $derived([
    { key: "xp", title: "XP", emoji: "✨" },
    { key: "lessons", title: $t("study.achv.tree.fam_lessons"), emoji: "📚", counterKey: "lessons_completed" },
    { key: "focus", title: $t("study.achv.tree.fam_focus"), emoji: "🧘", counterKey: "focus_minutes" },
    { key: "streak", title: "Streak", emoji: "🔥" },
  ]);

  let loading = $state(true);
  let error = $state("");
  let xpState = $state<GamificationState | null>(null);
  let unlocked = $state<Achievement[]>([]);
  let counters = $state<Record<string, number>>({});
  let selectedCode = $state<string | null>(null);

  async function load() {
    loading = true;
    error = "";
    try {
      const [st, achs] = await Promise.all([
        pluginInvoke<GamificationState>("study", "study:gamification:state"),
        pluginInvoke<Achievement[]>("study", "study:gamification:achievements"),
      ]);
      xpState = st;
      unlocked = Array.isArray(achs) ? achs : [];

      const counterKeys = Array.from(
        new Set(
          TREE_NODES.map((n) => n.counterKey).filter((k): k is string => !!k),
        ),
      );
      const counterEntries = await Promise.all(
        counterKeys.map((key) =>
          pluginInvoke<{ value: number }>(
            "study",
            "study:gamification:counter:get",
            { key },
          )
            .then((v) => [key, v.value ?? 0] as const)
            .catch(() => [key, 0] as const),
        ),
      );
      counters = Object.fromEntries(counterEntries);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  const unlockedSet = $derived(new Set(unlocked.map((a) => a.code)));

  function progressForNode(node: TreeNode): number {
    if (node.family === "xp") {
      return Math.min(node.threshold, xpState?.xp ?? 0);
    }
    if (node.counterKey) {
      return Math.min(node.threshold, counters[node.counterKey] ?? 0);
    }
    return 0;
  }

  function nodeState(node: TreeNode): "unlocked" | "available" | "locked" {
    if (unlockedSet.has(node.code)) return "unlocked";
    if (!node.requires) return "available";
    return unlockedSet.has(node.requires) ? "available" : "locked";
  }

  function tierColor(tier: string): string {
    switch (tier) {
      case "gold":
        return "#f59e0b";
      case "silver":
        return "#9ca3af";
      case "bronze":
        return "#b45309";
      default:
        return "var(--accent)";
    }
  }

  const detail = $derived(
    selectedCode ? TREE_NODES.find((n) => n.code === selectedCode) ?? null : null,
  );

  const familyStats = $derived.by(() => {
    return FAMILIES.map((f) => {
      const nodes = TREE_NODES.filter((n) => n.family === f.key);
      const got = nodes.filter((n) => unlockedSet.has(n.code)).length;
      return { ...f, total: nodes.length, got };
    });
  });

  function fmtRel(secs: number): string {
    const days = Math.floor((Date.now() / 1000 - secs) / 86400);
    if (days <= 0) return "hoje";
    if (days === 1) return "ontem";
    if (days < 30) return $t("study.achv.tree.d_ago", { n: days });
    if (days < 365) return $t("study.achv.tree.mo_ago", { n: Math.floor(days / 30) });
    return $t("study.achv.tree.y_ago", { n: Math.floor(days / 365) });
  }

  onMount(load);
</script>

<section class="tree-page">
  <PageHero title="Skill tree" subtitle={$t("study.achv.tree.subtitle")} />

  <header class="actions">
    <a class="link" href="/study/achievements">← Voltar pra dashboard</a>
    <a class="link" href="/study/achievements/charts">{$t("study.achv.charts.link")} →</a>
  </header>

  {#if loading}
    <div class="state">Carregando…</div>
  {:else if error}
    <div class="state err">{error}</div>
  {:else}
    <div class="legend">
      <span class="leg unlocked"><span class="dot"></span>desbloqueado</span>
      <span class="leg available"><span class="dot"></span>{$t("study.achv.tree.available")}</span>
      <span class="leg locked"><span class="dot"></span>bloqueado</span>
    </div>

    <div class="trees">
      {#each familyStats as fam (fam.key)}
        {@const nodes = TREE_NODES.filter((n) => n.family === fam.key)}
        <article class="family">
          <header class="family-head">
            <span class="family-emoji">{fam.emoji}</span>
            <h3>{fam.title}</h3>
            <span class="family-count">{fam.got} / {fam.total}</span>
          </header>

          <ol class="chain">
            {#each nodes as node, idx (node.code)}
              {@const state = nodeState(node)}
              {@const value = progressForNode(node)}
              {@const pct = Math.min(100, Math.round((value / node.threshold) * 100))}
              {#if idx > 0}
                <li
                  class="connector"
                  class:reached={state === "unlocked"}
                ></li>
              {/if}
              <li>
                <button
                  type="button"
                  class="node"
                  class:unlocked={state === "unlocked"}
                  class:available={state === "available"}
                  class:locked={state === "locked"}
                  class:active={selectedCode === node.code}
                  style:--tier={tierColor(node.tier)}
                  onclick={() => (selectedCode = node.code)}
                >
                  <span class="node-icon" aria-hidden="true">{node.icon}</span>
                  <div class="node-meta">
                    <span class="node-label">{node.label}</span>
                    <span class="node-tier">{node.tier}</span>
                  </div>
                  {#if state !== "unlocked"}
                    <span
                      class="node-progress"
                      title={`${value} / ${node.threshold}`}
                      style:--pct={`${pct}%`}
                    ></span>
                  {/if}
                </button>
              </li>
            {/each}
          </ol>
        </article>
      {/each}
    </div>

    {#if detail}
      {@const dState = nodeState(detail)}
      {@const dValue = progressForNode(detail)}
      {@const dUnlock = unlocked.find((a) => a.code === detail.code)}
      <aside class="detail-card" role="region" aria-live="polite">
        <header class="detail-head">
          <span class="detail-icon">{detail.icon}</span>
          <div class="detail-title">
            <h3>{detail.label}</h3>
            <span class="detail-tier" style:color={tierColor(detail.tier)}>
              {detail.tier}
            </span>
          </div>
          <button
            type="button"
            class="detail-close"
            onclick={() => (selectedCode = null)}
            aria-label="Fechar"
          >×</button>
        </header>
        <p>{detail.description}</p>
        {#if dState === "unlocked"}
          <p class="detail-status ok">
            ✓ Desbloqueado{dUnlock ? ` ${fmtRel(dUnlock.unlocked_at)}` : ""}
          </p>
        {:else}
          {#if detail.requires}
            {@const reqState = unlockedSet.has(detail.requires) ? "ok" : "locked"}
            <p class="detail-status">
              <span class="muted">{$t("study.achv.tree.prereq")}</span>
              <code>{detail.requires}</code>
              {#if reqState === "ok"}
                <span class="ok">✓</span>
              {:else}
                <span class="warn">pendente</span>
              {/if}
            </p>
          {/if}
          <div class="detail-progress">
            <div
              class="detail-fill"
              style:width={`${Math.min(100, (dValue / detail.threshold) * 100)}%`}
            ></div>
          </div>
          <p class="detail-prog-text mono">
            {dValue.toLocaleString()} / {detail.threshold.toLocaleString()}
          </p>
        {/if}
      </aside>
    {/if}
  {/if}
</section>

<style>
  .tree-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.25);
    width: 100%;
    max-width: 1100px;
    margin-inline: auto;
  }
  .actions {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .link {
    color: var(--accent);
    text-decoration: none;
    font-size: 12px;
  }
  .link:hover {
    text-decoration: underline;
  }
  .state {
    padding: calc(var(--padding) * 2);
    text-align: center;
    color: var(--secondary);
    font-size: 14px;
  }
  .state.err {
    color: var(--error, var(--accent));
  }

  .legend {
    display: flex;
    gap: 16px;
    flex-wrap: wrap;
    font-size: 12px;
    color: var(--tertiary);
  }
  .leg {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .leg .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 2px solid var(--input-border);
    background: transparent;
  }
  .leg.unlocked .dot {
    background: var(--success, var(--accent));
    border-color: var(--success, var(--accent));
  }
  .leg.available .dot {
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 15%, transparent);
  }
  .leg.locked .dot {
    border-style: dashed;
  }

  .trees {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: var(--padding);
  }
  .family {
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    padding: calc(var(--padding) * 0.9);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .family-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .family-head h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
  }
  .family-emoji {
    font-size: 16px;
  }
  .family-count {
    margin-left: auto;
    font-size: 11px;
    color: var(--tertiary);
    font-family: var(--font-mono, ui-monospace, monospace);
  }

  .chain {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0;
  }
  .connector {
    width: 2px;
    height: 16px;
    margin-left: 22px;
    background: color-mix(in oklab, var(--input-border) 70%, transparent);
  }
  .connector.reached {
    background: var(--success, var(--accent));
  }

  .node {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-radius: var(--border-radius);
    background: var(--bg);
    border: 2px solid var(--input-border);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    cursor: pointer;
    text-align: left;
    transition: border-color 120ms ease, background 120ms ease;
    position: relative;
  }
  .node-icon {
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    background: color-mix(in oklab, var(--tier) 18%, var(--bg));
    font-size: 14px;
    flex-shrink: 0;
  }
  .node-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .node-label {
    font-weight: 500;
  }
  .node-tier {
    font-size: 10px;
    color: var(--tier);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-weight: 600;
  }
  .node-progress {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 2px;
    width: var(--pct);
    background: var(--tier);
    border-radius: 0 0 var(--border-radius) var(--border-radius);
  }

  .node.unlocked {
    border-color: var(--success, var(--accent));
    background: color-mix(in oklab, var(--success, var(--accent)) 8%, var(--bg));
  }
  .node.available {
    border-color: var(--accent);
  }
  .node.locked {
    border-style: dashed;
    opacity: 0.55;
    filter: grayscale(0.5);
  }
  .node:hover:not(.locked) {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .node.active {
    box-shadow: 0 0 0 3px color-mix(in oklab, var(--accent) 30%, transparent);
  }

  .detail-card {
    background: var(--surface);
    border: 1px solid color-mix(in oklab, var(--accent) 40%, var(--input-border));
    border-radius: var(--border-radius);
    padding: calc(var(--padding) * 1.25);
    display: flex;
    flex-direction: column;
    gap: 10px;
    position: sticky;
    bottom: 12px;
    box-shadow: 0 8px 28px color-mix(in oklab, black 16%, transparent);
  }
  .detail-head {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .detail-icon {
    font-size: 24px;
  }
  .detail-title {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .detail-title h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .detail-tier {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-weight: 600;
  }
  .detail-close {
    background: transparent;
    border: 0;
    color: var(--tertiary);
    cursor: pointer;
    font-size: 18px;
    padding: 0 6px;
    line-height: 1;
  }
  .detail-card p {
    margin: 0;
    font-size: 13px;
    color: var(--secondary);
    line-height: 1.5;
  }
  .detail-status {
    font-size: 12px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .detail-status.ok,
  .detail-status .ok {
    color: var(--success, var(--accent));
  }
  .detail-status .warn {
    color: var(--warning, var(--accent));
  }
  .detail-status .muted {
    color: var(--tertiary);
  }
  .detail-status code {
    font-size: 11px;
    padding: 1px 5px;
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border-radius: 4px;
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .detail-progress {
    height: 6px;
    background: color-mix(in oklab, var(--input-border) 50%, transparent);
    border-radius: 999px;
    overflow: hidden;
  }
  .detail-fill {
    height: 100%;
    background: var(--accent);
    border-radius: 999px;
  }
  .detail-prog-text {
    font-size: 11px;
    color: var(--tertiary);
  }
  .mono {
    font-family: var(--font-mono, ui-monospace, monospace);
  }

  @media (prefers-reduced-motion: reduce) {
    .node,
    .detail-fill {
      transition: none;
    }
  }
</style>
