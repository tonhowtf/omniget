<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import { social, errText, type ApprovalRequest } from "./api";

  const LABEL: Record<string, string> = { x: "X", instagram: "Instagram", tiktok: "TikTok" };
  const FIELDS = ["post", "text", "handle", "username", "reply_to", "quote", "undo"];

  let queue = $state<ApprovalRequest[]>([]);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let now = $state(Date.now());
  let denyBtn: HTMLButtonElement | undefined = $state();
  let current = $derived(queue[0] ?? null);
  let details = $derived(current ? detailRows(current.details) : []);
  let left = $derived(current ? Math.max(0, Math.round(current.expires_at - now / 1000)) : null);
  let clientName = $derived.by(() => {
    const c = current?.client?.trim();
    if (!c || c === "agent") return null;
    return c === "cli" ? "OmniGet CLI" : c;
  });

  async function refresh() {
    try {
      queue = await social.approvalList();
    } catch {
    }
  }

  async function answer(approve: boolean) {
    const c = current;
    if (!c) return;
    busy = true;
    error = null;
    try {
      await social.approvalAnswer(c.id, approve);
      queue = queue.filter((q) => q.id !== c.id);
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
      await refresh();
    }
  }

  function detailRows(d: Record<string, unknown> | null): { key: string; value: string }[] {
    if (!d) return [];
    return Object.entries(d)
      .filter(([, v]) => v != null && v !== "" && v !== false)
      .map(([k, v]) => ({
        key: FIELDS.includes(k) ? ($t(`social.approval.fields.${k}`) as string) : k,
        value: typeof v === "boolean" ? ($t("social.approval.yes") as string) : typeof v === "string" ? v : JSON.stringify(v),
      }));
  }

  function clock(sec: number): string {
    return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, "0")}`;
  }

  let focusedId = "";
  $effect(() => {
    const id = current?.id ?? "";
    if (id && denyBtn && id !== focusedId) {
      focusedId = id;
      denyBtn.focus();
    }
  });

  onMount(() => {
    void refresh();
    const uns: (() => void)[] = [];
    let disposed = false;
    const keep = (f: () => void) => {
      if (disposed) f();
      else uns.push(f);
    };
    listen<{ id?: string }>("social-approval-resolved", (ev) => {
      const id = ev.payload?.id;
      if (id) queue = queue.filter((q) => q.id !== id);
    })
      .then(keep)
      .catch(() => {});
    listen<ApprovalRequest>("social-approval-request", (ev) => {
      const p = ev.payload;
      if (p?.id && !queue.some((q) => q.id === p.id)) queue = [...queue, p];
    })
      .then(keep)
      .catch(() => {});
    const timer = setInterval(() => {
      now = Date.now();
      if (queue.some((q) => q.expires_at <= now / 1000)) queue = queue.filter((q) => q.expires_at > now / 1000);
    }, 1000);
    return () => {
      disposed = true;
      for (const f of uns) f();
      clearInterval(timer);
    };
  });
</script>

{#if current}
  <div class="so-ap-backdrop" role="presentation"></div>
  <div class="so-ap" role="alertdialog" aria-modal="true" aria-labelledby="so-ap-title" aria-describedby="so-ap-sum">
    <header class="so-ap-head">
      <NavIcon icon={current.platform} size={40} />
      <div class="so-ap-titles">
        <h3 id="so-ap-title">
          {clientName ? $t("social.approval.title_client", { client: clientName, platform: LABEL[current.platform] }) : $t("social.approval.title", { platform: LABEL[current.platform] })}
        </h3>
        <span class="so-ap-tool">{current.tool}</span>
      </div>
      {#if queue.length > 1}<span class="so-ap-count">{$t("social.approval.queue", { count: 1, total: queue.length })}</span>{/if}
    </header>

    {#if current.summary}<p class="so-ap-sum" id="so-ap-sum">{current.summary}</p>{/if}

    {#if details.length}
      <dl class="so-ap-details">
        {#each details as r, i (i)}
          <div class="row">
            <dt>{r.key}</dt>
            <dd>{r.value}</dd>
          </div>
        {/each}
      </dl>
    {/if}

    <p class="so-ap-hint">
      {$t("social.approval.hint", { platform: LABEL[current.platform] })}
      {#if left !== null}<span class="so-ap-left">{$t("social.approval.expires", { time: clock(left) })}</span>{/if}
    </p>
    {#if error}<p class="so-ap-err">{error}</p>{/if}

    <div class="so-ap-row">
      <button class="btn btn-secondary" bind:this={denyBtn} disabled={busy} onclick={() => answer(false)}>{$t("social.approval.deny")}</button>
      <button class="btn btn-primary" disabled={busy} onclick={() => answer(true)}>
        {#if busy}<span class="spinner"></span>{/if}
        {$t("social.approval.approve")}
      </button>
    </div>
  </div>
{/if}

<style>
  .so-ap-backdrop {
    position: fixed;
    inset: 0;
    z-index: 1000;
    background: rgb(0 0 0 / 0.32);
  }

  .so-ap {
    position: fixed;
    left: 50%;
    top: 50%;
    z-index: 1001;
    transform: translate(-50%, -50%);
    width: min(500px, calc(100vw - 32px));
    max-height: calc(100vh - 48px);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-5);
    border-radius: var(--radius-xl, 16px);
    background: var(--surface-hi, var(--surface));
    box-shadow: 0 24px 64px rgb(0 0 0 / 0.35), inset 0 0 0 var(--hairline, 1px) var(--content-border, transparent);
    color: var(--text);
    animation: so-ap-in 160ms var(--ease-out, ease-out);
  }

  @keyframes so-ap-in {
    from {
      opacity: 0;
      transform: translate(-50%, -48%) scale(0.98);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .so-ap {
      animation: none;
    }
  }

  .so-ap-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .so-ap-titles {
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

  .so-ap-tool {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .so-ap-count {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .so-ap-sum {
    margin: 0;
    font-size: var(--text-base);
    line-height: 1.45;
    overflow-wrap: anywhere;
  }

  .so-ap-details {
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

  .so-ap-hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    line-height: 1.45;
  }

  .so-ap-left {
    margin-left: 4px;
    font-variant-numeric: tabular-nums;
  }

  .so-ap-err {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--danger);
  }

  .so-ap-row {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
