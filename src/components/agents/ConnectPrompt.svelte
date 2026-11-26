<script lang="ts" module>
  let done = $state<{ name: string; message: string } | null>(null);
</script>

<script lang="ts">
  import { agentActions } from "$lib/native-agent/actions";
  import { onMount } from "svelte";
  import ApprovalDialog from "#components/shell/ApprovalDialog.svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { shortPath } from "$lib/stores/assist-missions-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Request = {
    id: string;
    client: string;
    client_name: string;
    workspace: string | null;
    browser: boolean;
    code: string;
    created_ms: number;
  };

  let queue = $state<Request[]>([]);
  let current = $derived(queue[0] ?? null);
  let agents = $state(true);
  let approvals = $state(false);
  let browser = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  async function refresh() {
    try {
      queue = await invoke<Request[]>("connect_pending");
    } catch {
      queue = [];
    }
  }

  $effect(() => {
    void current?.id;
    agents = true;
    approvals = false;
    browser = current?.browser ?? false;
    error = null;
  });

  async function answer(allow: boolean) {
    if (!current) return;
    busy = true;
    error = null;
    const name = current.client_name;
    try {
      const r = await invoke<{ message: string } | { status: string }>("connect_answer", {
        id: current.id,
        allow,
        agents: agents && !!current.workspace,
        approvals: approvals && agents && !!current.workspace,
        browser,
      });
      if (allow && "message" in r) {
        done = { name, message: r.message };
        showToast("success", $t("connect.done_title", { client: name }) as string);
      }
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
      await refresh();
    }
  }

  onMount(() => {
    void refresh();
    const un = listen("connect-request", () => {
      done = null;
      void refresh();
    });
    return () => void un.then((f) => f());
  });
</script>

{#if current}
  {#key current.id}
  <ApprovalDialog
    titleId="cp-title"
    describedBy="cp-code"
    {error}
    {busy}
    align="center"
    denyLabel={$t("connect.deny") as string}
    allowLabel={(busy ? $t("connect.connecting") : $t("connect.allow")) as string}
    onDeny={() => answer(false)}
    onAllow={() => answer(true)}
  >
    <h3 id="cp-title">{$t("connect.request_title", { client: current.client_name })}</h3>
    <p class="cp-sub">{$t("connect.request_code_hint")}</p>
    <p class="cp-code" id="cp-code">{current.code}</p>

    <ul class="cp-list">
      <li>
        <span class="cp-check on" aria-hidden="true">✓</span>
        <span>{$t("connect.scope_downloads")}</span>
      </li>
      {#if current.workspace}
        <li>
          <label use:agentActions={[]} class="cp-toggle">
            <input use:agentActions={[]} type="checkbox" bind:checked={agents} disabled={busy} />
            <span>
              {$t("connect.scope_agents")}
              <small title={current.workspace}>{shortPath(current.workspace)}</small>
            </span>
          </label>
        </li>
        <li>
          <label use:agentActions={[]} class="cp-toggle">
            <input use:agentActions={[]} type="checkbox" bind:checked={approvals} disabled={busy || !agents} />
            <span>
              {$t("connect.scope_approvals")}
              <small>{$t("connect.scope_approvals_hint")}</small>
            </span>
          </label>
        </li>
      {/if}
      <li>
        <label use:agentActions={[]} class="cp-toggle">
          <input use:agentActions={[]} type="checkbox" bind:checked={browser} disabled={busy} />
          <span>
            {$t("connect.scope_browser")}
            <small>{$t("connect.scope_browser_hint")}</small>
          </span>
        </label>
      </li>
    </ul>

    <p class="cp-hint">{$t("connect.request_hint", { client: current.client_name })}</p>

  </ApprovalDialog>
  {/key}
{:else if done}
  <ApprovalDialog titleId="cp-done-title" align="center" allowLabel={$t("connect.close") as string} onAllow={() => (done = null)}>
    <h3 id="cp-done-title">{$t("connect.done_title", { client: done.name })}</h3>
    <p class="cp-hint">{$t("connect.done_hint", { client: done.name })}</p>
    <p class="cp-msg">{done.message}</p>
  </ApprovalDialog>
{/if}

<style>
  h3 { margin: 0; font-size: 16px; }
  .cp-sub { margin: 0; font-size: 12px; color: var(--text-muted); }
  .cp-code {
    margin: 0; font-size: 26px; font-weight: 700; letter-spacing: 0.12em;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-variant-numeric: tabular-nums;
  }
  .cp-list {
    list-style: none; margin: 0; padding: 0; text-align: left; border-radius: 10px;
    background: color-mix(in srgb, currentColor 5%, transparent);
  }
  .cp-list li { display: flex; gap: 8px; align-items: flex-start; padding: 8px 12px; font-size: 13px; }
  .cp-list li + li { border-top: 1px solid var(--border); }
  .cp-check { width: 16px; text-align: center; color: var(--accent); }
  .cp-toggle { display: flex; gap: 8px; align-items: flex-start; cursor: pointer; }
  .cp-toggle input { margin-top: 2px; }
  .cp-toggle span, .cp-list li > span:last-child { display: flex; flex-direction: column; min-width: 0; }
  .cp-list small {
    color: var(--text-muted); font-size: 11px;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .cp-hint { margin: 0; font-size: 12px; color: var(--text-muted); }
  .cp-msg {
    margin: 0; font-size: 12px; text-align: left; overflow-wrap: anywhere;
    color: var(--text-muted);
  }
</style>
