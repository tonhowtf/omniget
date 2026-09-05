<script lang="ts">
  type DiffLine = { type: " " | "+" | "-"; line: string };

  type Props = {
    oldText: string;
    newText: string;
    maxLines?: number;
  };

  let { oldText, newText, maxLines = 500 }: Props = $props();

  function truncate(text: string, max: number): { text: string; truncated: boolean } {
    const lines = text.split("\n");
    if (lines.length <= max) return { text, truncated: false };
    return { text: lines.slice(0, max).join("\n"), truncated: true };
  }

  function lineDiff(a: string, b: string): DiffLine[] {
    const A = a.split("\n");
    const B = b.split("\n");
    const m = A.length;
    const n = B.length;
    const dp: number[][] = Array.from({ length: m + 1 }, () =>
      new Array(n + 1).fill(0),
    );
    for (let i = 1; i <= m; i++) {
      for (let j = 1; j <= n; j++) {
        if (A[i - 1] === B[j - 1]) dp[i][j] = dp[i - 1][j - 1] + 1;
        else dp[i][j] = Math.max(dp[i - 1][j], dp[i][j - 1]);
      }
    }
    const out: DiffLine[] = [];
    let i = m;
    let j = n;
    while (i > 0 && j > 0) {
      if (A[i - 1] === B[j - 1]) {
        out.push({ type: " ", line: A[i - 1] });
        i--;
        j--;
      } else if (dp[i - 1][j] >= dp[i][j - 1]) {
        out.push({ type: "-", line: A[i - 1] });
        i--;
      } else {
        out.push({ type: "+", line: B[j - 1] });
        j--;
      }
    }
    while (i > 0) {
      out.push({ type: "-", line: A[i - 1] });
      i--;
    }
    while (j > 0) {
      out.push({ type: "+", line: B[j - 1] });
      j--;
    }
    return out.reverse();
  }

  let oldTrunc = $derived(truncate(oldText, maxLines));
  let newTrunc = $derived(truncate(newText, maxLines));
  let diff = $derived(lineDiff(oldTrunc.text, newTrunc.text));
  let truncated = $derived(oldTrunc.truncated || newTrunc.truncated);
  let stats = $derived.by(() => {
    let added = 0;
    let removed = 0;
    for (const d of diff) {
      if (d.type === "+") added++;
      else if (d.type === "-") removed++;
    }
    return { added, removed };
  });
</script>

<div class="diff-view" role="region" aria-label="Comparação de versões">
  <header class="diff-head">
    <span class="stat added">+{stats.added}</span>
    <span class="stat removed">−{stats.removed}</span>
    {#if truncated}
      <span class="trunc">truncado a {maxLines} linhas</span>
    {/if}
  </header>
  {#if diff.length === 0}
    <p class="empty">Sem diferenças.</p>
  {:else}
    <pre class="diff-body"><code>{#each diff as d (d.type + ":" + d.line)}<span
            class="line {d.type === '+' ? 'add' : d.type === '-' ? 'rem' : 'eq'}"
            ><span class="marker">{d.type}</span><span class="content">{d.line}</span
            ></span
          >
{/each}</code></pre>
  {/if}
</div>

<style>
  .diff-view {
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex: 1 1 auto;
    min-height: 0;
  }
  .diff-head {
    display: flex;
    gap: 12px;
    align-items: center;
    font-size: 11px;
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .stat {
    padding: 1px 6px;
    border-radius: 999px;
    font-weight: 600;
  }
  .stat.added {
    background: color-mix(in oklab, var(--success) 18%, transparent);
    color: var(--success);
  }
  .stat.removed {
    background: color-mix(in oklab, var(--error) 18%, transparent);
    color: var(--error);
  }
  .trunc {
    color: var(--tertiary);
    font-style: italic;
  }
  .empty {
    margin: 0;
    color: var(--tertiary);
    font-size: 12px;
    font-style: italic;
  }
  .diff-body {
    margin: 0;
    padding: 8px;
    background: var(--bg);
    border: none;
    border-radius: var(--border-radius);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
    line-height: 1.5;
    overflow: auto;
    flex: 1 1 auto;
    min-height: 0;
    max-height: 100%;
    white-space: pre;
  }
  .line {
    display: block;
  }
  .line.add {
    background: color-mix(in oklab, var(--success) 14%, transparent);
    color: var(--text);
  }
  .line.rem {
    background: color-mix(in oklab, var(--error) 14%, transparent);
    color: var(--text);
  }
  .line.eq {
    color: color-mix(in oklab, var(--text) 70%, transparent);
  }
  .marker {
    display: inline-block;
    width: 16px;
    text-align: center;
    color: var(--tertiary);
    user-select: none;
  }
  .line.add .marker {
    color: var(--success);
    font-weight: 700;
  }
  .line.rem .marker {
    color: var(--error);
    font-weight: 700;
  }
  .content {
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
