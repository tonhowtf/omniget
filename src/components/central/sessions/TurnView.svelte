<script lang="ts">
  /**
   * Uma mensagem da sessão: papel, hora, modelo, texto (blocos ``` viram
   * código) e as tool calls pareadas com o resultado, cada uma recolhível.
   * `mark` destaca o termo da busca dentro da sessão.
   */
  import { t } from "$lib/i18n";
  import { compact, totalTokens, usd, type ToolCall, type Turn } from "$lib/central/sessions";

  interface Props {
    turn: Turn;
    index: number;
    showTools: boolean;
    mark?: string;
    current?: boolean;
  }

  let { turn, index, showTools, mark = "", current = false }: Props = $props();

  const LIMIT = 4000;
  let expanded = $state<Set<string>>(new Set());

  type Block = { code: boolean; lang: string; text: string };

  let blocks = $derived.by<Block[]>(() => {
    const out: Block[] = [];
    const re = /```([^\n`]*)\n([\s\S]*?)```/g;
    let last = 0;
    let m: RegExpExecArray | null;
    const s = turn.text ?? "";
    while ((m = re.exec(s))) {
      if (m.index > last) out.push({ code: false, lang: "", text: s.slice(last, m.index) });
      out.push({ code: true, lang: m[1].trim(), text: m[2] });
      last = m.index + m[0].length;
    }
    if (last < s.length) out.push({ code: false, lang: "", text: s.slice(last) });
    return out.filter((b) => b.text.trim().length > 0);
  });

  /** Divide o texto em pedaços marcados/não marcados (sem regex do usuário). */
  function parts(text: string): { t: string; m: boolean }[] {
    const q = mark.trim().toLowerCase();
    if (!q) return [{ t: text, m: false }];
    const low = text.toLowerCase();
    const out: { t: string; m: boolean }[] = [];
    let i = 0;
    let guard = 0;
    while (i < text.length && guard++ < 500) {
      const j = low.indexOf(q, i);
      if (j < 0) break;
      if (j > i) out.push({ t: text.slice(i, j), m: false });
      out.push({ t: text.slice(j, j + q.length), m: true });
      i = j + q.length;
    }
    if (i < text.length) out.push({ t: text.slice(i), m: false });
    return out;
  }

  function summary(c: ToolCall): string {
    const inp = c.input as Record<string, unknown> | null;
    if (!inp || typeof inp !== "object") return typeof c.input === "string" ? c.input : "";
    for (const k of ["command", "file_path", "path", "pattern", "url", "query", "description", "prompt", "subject", "skill", "name"]) {
      const v = inp[k];
      if (typeof v === "string" && v) return v.split("\n")[0];
    }
    return "";
  }

  function inputText(c: ToolCall): string {
    if (typeof c.input === "string") return c.input;
    try {
      return JSON.stringify(c.input, null, 2);
    } catch {
      return String(c.input);
    }
  }

  function clip(id: string, s: string): string {
    return expanded.has(id) || s.length <= LIMIT ? s : `${s.slice(0, LIMIT)}…`;
  }

  function expand(id: string) {
    expanded = new Set([...expanded, id]);
  }

  let time = $derived(turn.ts ? new Date(turn.ts).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }) : "");
  let tokens = $derived(totalTokens(turn.usage));
  let isUser = $derived(turn.role === "user");
</script>

<article class="turn {turn.role}" class:current data-turn={index} aria-label={`${$t(`llm.central.sessions.role.${turn.role}`)} ${time}`}>
  <header>
    <span class="role">{$t(`llm.central.sessions.role.${turn.role}`)}</span>
    <time datetime={turn.ts} title={turn.ts ? new Date(turn.ts).toLocaleString() : ""}>{time}</time>
    {#if turn.model}<span class="model">{turn.model}</span>{/if}
    {#if tokens > 0}<span class="tok" title={`in ${turn.usage.input} · out ${turn.usage.output} · cache r ${turn.usage.cache_read} · cache w ${turn.usage.cache_write} · reasoning ${turn.usage.reasoning}`}>{compact(tokens)} tok</span>{/if}
    {#if turn.cost_usd}<span class="tok">{usd(turn.cost_usd)}</span>{/if}
    <span class="idx">#{index + 1}</span>
  </header>
  {#if blocks.length}
    <div class="text" class:bubble={isUser}>
      {#each blocks as b, i (i)}
        {#if b.code}
          <pre class="code"><code>{#each parts(b.text) as p, k (k)}{#if p.m}<mark>{p.t}</mark>{:else}{p.t}{/if}{/each}</code></pre>
        {:else}
          <p class="para">{#each parts(b.text) as p, k (k)}{#if p.m}<mark>{p.t}</mark>{:else}{p.t}{/if}{/each}</p>
        {/if}
      {/each}
    </div>
  {/if}
  {#if turn.tool_calls.length}
    {#if showTools}
      <div class="calls">
        {#each turn.tool_calls as c (c.id)}
          <details class="call {c.status}">
            <summary>
              <span class="c-name">{c.name_raw || c.name_canonical}</span>
              {#if c.subagent}<span class="c-sub">{c.subagent}</span>{/if}
              <span class="c-sum">{#each parts(summary(c)) as p, k (k)}{#if p.m}<mark>{p.t}</mark>{:else}{p.t}{/if}{/each}</span>
              {#if c.status === "error"}<span class="c-status err">{$t("llm.central.sessions.tool_error")}</span>{:else if c.status === "pending"}<span class="c-status">{$t("llm.central.sessions.tool_pending")}</span>{/if}
              {#if c.ms != null}<span class="c-ms">{c.ms < 1000 ? `${c.ms} ms` : `${(c.ms / 1000).toFixed(1)} s`}</span>{/if}
            </summary>
            <div class="c-body">
              <div class="c-label">{$t("llm.central.sessions.tool_input")}</div>
              <pre class="code">{clip(`${c.id}:in`, inputText(c))}</pre>
              {#if inputText(c).length > LIMIT && !expanded.has(`${c.id}:in`)}<button type="button" class="btn btn-ghost btn-sm" onclick={() => expand(`${c.id}:in`)}>{$t("llm.central.sessions.show_all")}</button>{/if}
              {#if c.result != null}
                <div class="c-label">{$t("llm.central.sessions.tool_result")}</div>
                <pre class="code result">{#each parts(clip(`${c.id}:out`, c.result)) as p, k (k)}{#if p.m}<mark>{p.t}</mark>{:else}{p.t}{/if}{/each}</pre>
                {#if c.result.length > LIMIT && !expanded.has(`${c.id}:out`)}<button type="button" class="btn btn-ghost btn-sm" onclick={() => expand(`${c.id}:out`)}>{$t("llm.central.sessions.show_all")}</button>{/if}
              {/if}
            </div>
          </details>
        {/each}
      </div>
    {:else}
      <p class="calls-hidden">{turn.tool_calls.length} {$t("llm.central.sessions.tool_calls_unit")}: {turn.tool_calls.map((c) => c.name_canonical).join(", ")}</p>
    {/if}
  {/if}
</article>

<style>
  .turn {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border-radius: var(--radius-lg);
    scroll-margin: 80px;
    transition: box-shadow var(--duration-base) var(--ease-out);
  }
  .turn.current {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .turn.system,
  .turn.tool {
    opacity: 0.85;
  }
  header {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 10px;
    align-items: baseline;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .role {
    font-weight: 600;
    color: var(--text-muted);
  }
  .user .role {
    color: var(--accent-hi);
  }
  .model,
  .tok {
    font-variant-numeric: tabular-nums;
  }
  .idx {
    margin-left: auto;
    color: var(--text-faint);
    font-variant-numeric: tabular-nums;
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    font-size: var(--text-base);
    line-height: 1.55;
    color: var(--text);
  }
  .text.bubble {
    background: var(--fill-1);
    border-radius: var(--radius-md);
    padding: 8px 10px;
  }
  .para {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .code {
    margin: 0;
    padding: 8px 10px;
    background: var(--fill-1);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 420px;
    overflow: auto;
    color: var(--text);
  }
  mark {
    background: var(--selection);
    color: var(--text);
    border-radius: 2px;
  }
  .calls {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .call {
    border-radius: var(--radius-md);
    box-shadow: inset 0 0 0 1px var(--content-border);
    background: var(--surface);
  }
  .call summary {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 6px 10px;
    cursor: pointer;
    font-size: var(--text-sm);
    min-width: 0;
    list-style: none;
  }
  .call summary::-webkit-details-marker {
    display: none;
  }
  .call summary::before {
    content: "›";
    color: var(--text-dim);
    transition: transform var(--duration-fast) var(--ease-out);
  }
  .call[open] summary::before {
    transform: rotate(90deg);
  }
  .call summary:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
    border-radius: var(--radius-md);
  }
  .c-name {
    font-weight: 600;
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    flex-shrink: 0;
  }
  .c-sub {
    font-size: var(--text-xs);
    color: var(--accent-hi);
    flex-shrink: 0;
  }
  .c-sum {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-muted);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }
  .c-status {
    font-size: var(--text-caption);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .c-status.err {
    color: var(--danger);
    font-weight: 600;
  }
  .call.error {
    box-shadow: inset 2px 0 0 var(--danger), inset 0 0 0 1px var(--content-border);
  }
  .c-ms {
    font-size: var(--text-caption);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .c-body {
    padding: 0 10px 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .c-label {
    font-size: var(--text-caption);
    color: var(--text-dim);
    margin-top: 4px;
  }
  .calls-hidden {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
