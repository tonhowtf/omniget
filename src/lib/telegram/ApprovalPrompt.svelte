<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import ApprovalDialog from "#components/shell/ApprovalDialog.svelte";
  import { tg3, errText, formatBytes, type ApprovalRequest } from "./api";

  const TOOLS = ["tg_send_message", "tg_send_files", "tg_forward", "tg_delete_messages"];
  const TIMEOUT_SEC = 300;

  let queue = $state<ApprovalRequest[]>([]);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let now = $state(Date.now());
  let current = $derived(queue[0] ?? null);
  let details = $derived(current ? detailRows(current.details) : []);
  let left = $derived.by(() => {
    if (!current) return null;
    const end = current.expires_at ?? (current.created ? current.created + TIMEOUT_SEC : null);
    return end ? Math.max(0, Math.round(end - now / 1000)) : null;
  });
  let clientName = $derived.by(() => {
    const c = current?.client?.trim();
    if (!c || c === "agent") return null;
    return c === "cli" ? "OmniGet CLI" : c;
  });

  async function refresh() {
    try {
      queue = await tg3.approvalList();
    } catch {
    }
  }

  async function answer(approve: boolean) {
    const c = current;
    if (!c) return;
    busy = true;
    error = null;
    try {
      await tg3.approvalAnswer(c.id, approve);
      queue = queue.filter((q) => q.id !== c.id);
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
      await refresh();
    }
  }

  type Row = { key: string; value: string };

  const FIELDS = ["chat", "from", "to", "text", "caption", "files", "message_ids", "messages", "as_document"];

  function chatText(v: Record<string, unknown>): string {
    const title = v.title ?? v.chat_title;
    const id = v.id ?? v.chat_id;
    return title ? `${title}${id != null ? ` (${id})` : ""}` : JSON.stringify(v);
  }

  function valueText(key: string, v: unknown): string {
    if (typeof v === "string") return v;
    if (typeof v === "boolean") return ($t(v ? "telegram.approval.yes" : "telegram.approval.no") as string);
    if (typeof v === "number") return String(v);
    if (Array.isArray(v)) {
      if (key === "files") {
        return v
          .map((f) => {
            if (typeof f !== "object" || !f) return String(f);
            const o = f as Record<string, unknown>;
            const size = typeof o.size === "number" && o.size > 0 ? ` · ${formatBytes(o.size)}` : "";
            return `${o.path ?? ""}${o.folder ? "/" : ""}${size}`;
          })
          .join("\n");
      }
      if (v.every((x) => typeof x !== "object")) return v.join(key === "messages" ? "\n" : ", ");
      return v.map((x) => (typeof x === "string" ? x : JSON.stringify(x))).join("\n");
    }
    if (v && typeof v === "object") return chatText(v as Record<string, unknown>);
    return String(v);
  }

  function detailRows(d: unknown): Row[] {
    if (d == null || d === "") return [];
    if (typeof d !== "object" || Array.isArray(d)) return [{ key: "", value: valueText("", d) }];
    return Object.entries(d as Record<string, unknown>)
      .filter(([, v]) => v != null && v !== "" && !(Array.isArray(v) && !v.length))
      .map(([k, v]) => ({
        key: FIELDS.includes(k) ? ($t(`telegram.approval.fields.${k}`) as string) : k,
        value: valueText(k, v),
      }));
  }

  function toolLabel(tool: string): string {
    return TOOLS.includes(tool) ? ($t(`telegram.approval.tools.${tool}`) as string) : tool;
  }

  function clock(sec: number): string {
    return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, "0")}`;
  }

  onMount(() => {
    void refresh();
    const uns: (() => void)[] = [];
    let disposed = false;
    const keep = (f: () => void) => {
      if (disposed) f();
      else uns.push(f);
    };
    listen<{ id?: string }>("telegram-approval-resolved", (ev) => {
      const id = ev.payload?.id;
      if (id) queue = queue.filter((q) => q.id !== id);
    })
      .then(keep)
      .catch(() => {});
    listen<Record<string, unknown>>("telegram-approval-request", (ev) => {
      const p = ev.payload;
      const id = String(p?.id ?? "");
      if (id && !queue.some((q) => q.id === id)) {
        queue = [
          ...queue,
          {
            id,
            tool: String(p.tool ?? ""),
            summary: String(p.summary ?? ""),
            details: p.details ?? null,
            client: (p.client as string | null | undefined) ?? null,
            created: (p.created as number | null | undefined) ?? Math.floor(Date.now() / 1000),
            expires_at: (p.expires_at as number | null | undefined) ?? null,
          },
        ];
      }
      void refresh();
    })
      .then(keep)
      .catch(() => {});
    const timer = setInterval(() => {
      now = Date.now();
      if (queue.some((q) => q.expires_at && q.expires_at <= now / 1000)) {
        queue = queue.filter((q) => !q.expires_at || q.expires_at > now / 1000);
      }
      if (queue.length && Math.floor(now / 1000) % 15 === 0) void refresh();
    }, 1000);
    return () => {
      disposed = true;
      for (const f of uns) f();
      clearInterval(timer);
    };
  });
</script>

{#if current}
  {#key current.id}
  <ApprovalDialog
    titleId="tg-ap-title"
    describedBy="tg-ap-sum"
    {error}
    {busy}
    width="lg"
    denyLabel={$t("telegram.approval.deny") as string}
    allowLabel={$t("telegram.approval.approve") as string}
    onDeny={() => answer(false)}
    onAllow={() => answer(true)}
  >
    <header class="tg-ap-head">
      <img src="/icons/menu/telegram-64.webp" alt="" width="40" height="40" />
      <div class="tg-ap-titles">
        <h3 id="tg-ap-title">
          {clientName ? $t("telegram.approval.title_client", { client: clientName }) : $t("telegram.approval.title")}
        </h3>
        <span class="tg-ap-tool">{toolLabel(current.tool)}</span>
      </div>
      {#if queue.length > 1}<span class="tg-ap-count">{$t("telegram.approval.queue", { count: 1, total: queue.length })}</span>{/if}
    </header>

    {#if current.summary}<p class="tg-ap-sum" id="tg-ap-sum">{current.summary}</p>{/if}

    {#if details.length}
      <dl class="tg-ap-details">
        {#each details as r, i (i)}
          <div class="row">
            {#if r.key}<dt>{r.key}</dt>{/if}
            <dd>{r.value}</dd>
          </div>
        {/each}
      </dl>
    {/if}

    <p class="tg-ap-hint">
      {$t("telegram.approval.hint")}
      {#if left !== null}<span class="tg-ap-left">{$t("telegram.approval.expires", { time: clock(left) })}</span>{/if}
    </p>
  </ApprovalDialog>
  {/key}
{/if}

<style>
  .tg-ap-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .tg-ap-titles {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  h3 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
  }

  .tg-ap-tool {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .tg-ap-count {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .tg-ap-sum {
    margin: 0;
    font-size: var(--text-base);
    line-height: 1.45;
    overflow-wrap: anywhere;
  }

  .tg-ap-details {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
    max-height: 240px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--text-sm);
  }

  .row {
    display: flex;
    gap: var(--space-2);
    min-width: 0;
  }

  dt {
    flex-shrink: 0;
    min-width: 88px;
    color: var(--text-dim);
  }

  dd {
    margin: 0;
    min-width: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .tg-ap-hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    line-height: 1.45;
  }

  .tg-ap-left {
    margin-left: 4px;
    font-variant-numeric: tabular-nums;
  }
</style>
