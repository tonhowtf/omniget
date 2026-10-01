<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { formatBytes } from "$lib/stores/download-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Task = { gid: string; status: string; name: string; total: number; done: number; speed: number; error: string | null };
  type Overview = { version: string; download_speed: number; tasks: Task[] };

  const ADDRESS_KEY = "omniget.tools.aria2-remote.address";
  const KNOWN = ["ARIA2_NO_ADDRESS", "ARIA2_BAD_ADDRESS", "ARIA2_UNREACHABLE", "ARIA2_UNAUTHORIZED", "ARIA2_NOT_ARIA2", "ARIA2_NO_LINKS"];

  function stored(): string {
    try {
      return localStorage.getItem(ADDRESS_KEY) ?? "";
    } catch {
      return "";
    }
  }

  let address = $state(stored());
  let secret = $state("");
  let links = $state("");
  let overview = $state<Overview | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let timer: ReturnType<typeof setInterval> | undefined;

  function explain(e: unknown): string {
    const msg = typeof e === "string" ? e : ((e as { message?: string })?.message ?? "");
    if (KNOWN.includes(msg)) return $t(`downloads.tools.aria2_remote.error_${msg.slice(6).toLowerCase()}`);
    if (msg.startsWith("ARIA2_ERROR|")) return $t("downloads.tools.aria2_remote.error_server", { message: msg.slice(12) });
    return msg || $t("common.error");
  }

  async function refresh(quiet = false) {
    try {
      overview = await invoke<Overview>("aria2_remote_overview", { address, secret });
      error = null;
    } catch (e) {
      if (!quiet) overview = null;
      error = explain(e);
      clearInterval(timer);
      timer = undefined;
    }
  }

  async function connect() {
    if (!address.trim() || busy) return;
    busy = true;
    try {
      localStorage.setItem(ADDRESS_KEY, address.trim());
    } catch {
    }
    await refresh();
    busy = false;
    clearInterval(timer);
    if (overview) timer = setInterval(() => refresh(true), 2000);
  }

  async function send() {
    if (!links.trim() || busy) return;
    busy = true;
    try {
      const r = await invoke<{ added: number; failed: string[] }>("aria2_remote_add", { address, secret, links });
      showToast(r.failed.length ? "error" : "success", $t("downloads.tools.aria2_remote.sent", { count: r.added, failed: r.failed.length }));
      if (r.added) links = "";
      await refresh(true);
    } catch (e) {
      error = explain(e);
    } finally {
      busy = false;
    }
  }

  async function act(task: Task, action: "pause" | "resume" | "remove" | "clear") {
    try {
      await invoke("aria2_remote_task", { address, secret, gid: task.gid, action });
      await refresh(true);
    } catch (e) {
      showToast("error", explain(e));
    }
  }

  onDestroy(() => clearInterval(timer));

  const pct = (x: Task) => (x.total ? Math.floor((x.done / x.total) * 100) : 0);
  const stopped = (x: Task) => ["complete", "error", "removed"].includes(x.status);
</script>

<div class="ar" data-agent-readiness={busy ? "loading" : "ready"} data-connected={overview ? "yes" : "no"}>
  <p class="lead">{$t("downloads.tools.aria2_remote.lead")}</p>
  <form
    class="row"
    onsubmit={(e) => {
      e.preventDefault();
      connect();
    }}
  >
    <input bind:value={address} placeholder={$t("downloads.tools.aria2_remote.address")} aria-label={$t("downloads.tools.aria2_remote.address")} spellcheck="false" autocomplete="off" />
    <input bind:value={secret} type="password" placeholder={$t("downloads.tools.aria2_remote.secret")} aria-label={$t("downloads.tools.aria2_remote.secret")} autocomplete="off" />
    <button class="button" type="submit" disabled={busy || !address.trim()}>{$t("downloads.tools.aria2_remote.connect")}</button>
  </form>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if overview}
    <p class="dim">
      {$t("downloads.tools.aria2_remote.connected", { version: overview.version, speed: formatBytes(overview.download_speed) })}
    </p>
    <textarea bind:value={links} rows="3" placeholder={$t("downloads.tools.aria2_remote.links")} aria-label={$t("downloads.tools.aria2_remote.links")} spellcheck="false"></textarea>
    <div class="actions">
      <button class="button" onclick={send} disabled={busy || !links.trim()}>{$t("downloads.tools.aria2_remote.send")}</button>
    </div>

    {#if overview.tasks.length}
      <ul class="list">
        {#each overview.tasks as task (task.gid)}
          <li class="task" data-status={task.status}>
            <div class="info">
              <strong class="name">{task.name || task.gid}</strong>
              <span class="dim small">
                {$t(`downloads.tools.aria2_remote.status_${task.status}`)} · {pct(task)}% · {formatBytes(task.done)} / {formatBytes(task.total)}{#if task.status === "active"} · {formatBytes(task.speed)}/s{/if}{#if task.error} · {task.error}{/if}
              </span>
            </div>
            <div class="buttons">
              {#if task.status === "active" || task.status === "waiting"}
                <button class="link" onclick={() => act(task, "pause")}>{$t("downloads.tools.aria2_remote.pause")}</button>
              {:else if task.status === "paused"}
                <button class="link" onclick={() => act(task, "resume")}>{$t("downloads.tools.aria2_remote.resume")}</button>
              {/if}
              {#if stopped(task)}
                <button class="link" onclick={() => act(task, "clear")}>{$t("downloads.tools.aria2_remote.clear")}</button>
              {:else}
                <button class="link" onclick={() => act(task, "remove")}>{$t("downloads.tools.aria2_remote.remove")}</button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="dim small">{$t("downloads.tools.aria2_remote.empty")}</p>
    {/if}
  {/if}
  <p class="dim small">{$t("downloads.tools.aria2_remote.note")}</p>
</div>

<style>
  .ar {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .small {
    font-size: var(--text-xs, 11px);
  }
  .row {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .row input,
  textarea {
    flex: 1;
    min-width: 8rem;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
  }
  textarea {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-xs, 11px);
    resize: vertical;
  }
  .row input:focus-visible,
  textarea:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .task {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
  }
  .info {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
  }
  .buttons {
    display: flex;
    gap: var(--space-2);
    flex-shrink: 0;
  }
  .link {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    font-size: var(--text-sm);
    cursor: pointer;
  }
</style>
