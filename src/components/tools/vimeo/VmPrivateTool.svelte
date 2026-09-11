<script lang="ts">
  /**
   * Vimeo privado / unlisted: o link com hash já abre sozinho; o vídeo com
   * senha usa a senha que o dono passou para o usuário. Uma tentativa por
   * clique — senha errada volta como erro e para por aí.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openPath, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Result = {
    id: string;
    title: string;
    file: string | null;
    skipped: boolean;
    unlisted_hash: boolean;
    used_password: boolean;
    used_session: boolean;
    command: string;
  };

  let url = $state("");
  let dest = $state("");
  let password = $state("");
  let showPassword = $state(false);
  let audioOnly = $state(false);
  let writeInfo = $state(false);
  let skipExisting = $state(true);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const CODES = [
    "bad_url",
    "is_a_collection",
    "password_required",
    "wrong_password",
    "private",
    "not_found",
    "geo",
    "unavailable",
  ];

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "vm-private") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  function niceError(e: unknown): string {
    const raw = errText(e).trim();
    const code = raw.startsWith("vimeo:") ? raw.slice(6) : "";
    if (code && CODES.includes(code)) {
      const text = $t(`tools.vimeo.err_${code}`) as string;
      if (text) return text;
    }
    return raw;
  }

  async function run() {
    if (!url.trim() || busy) return;
    let out = dest;
    if (!out) {
      const d = await pickDir();
      if (!d) return;
      out = d;
      dest = d;
    }
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_vm_private", {
        opts: {
          url: url.trim(),
          dest: out,
          video_password: password.trim() || null,
          audio_only: audioOnly,
          write_info: writeInfo,
          skip_existing: skipExisting,
        },
      });
      showToast("success", result.skipped ? ($t("tools.vimeo.already_there") as string) : ($t("tools.common.done") as string));
    } catch (e) {
      showToast("error", niceError(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <input class="input" type="text" bind:value={url} placeholder={$t("tools.vimeo.link_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} />
          <div class="group-row-sub">{$t("tools.vimeo.link_hint")}</div>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.vimeo.password")} <span class="dim">({$t("tools.common.optional")})</span></div>
          <div class="group-row-sub">{$t("tools.vimeo.password_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if showPassword}
            <input class="input" type="text" bind:value={password} style:width="12em" autocomplete="off" />
          {:else}
            <input class="input" type="password" bind:value={password} style:width="12em" autocomplete="off" />
          {/if}
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => (showPassword = !showPassword)}>{showPassword ? $t("tools.vimeo.hide") : $t("tools.vimeo.show")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.output_folder")}</div>
          <div class="group-row-sub mono">{dest || "—"}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if dest}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (dest = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
    </div>
  </section>

  <section>
    <span class="group-label">{$t("tools.vimeo.options")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.vimeo.audio_only")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={audioOnly} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.vimeo.write_info")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={writeInfo} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.vimeo.skip_existing")}</div>
          <div class="group-row-sub">{$t("tools.vimeo.skip_existing_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={skipExisting} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub">{$t("tools.vimeo.downloading")} {pct(progress) ?? 0}%</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !url.trim()} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.vimeo.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title strong">{result.title || result.id}</div>
            <div class="group-row-sub">
              {#if result.skipped}<span class="tag tag-success">{$t("tools.vimeo.already_there")}</span>{:else}<span class="tag tag-success">{$t("tools.common.done")}</span>{/if}
              {#if result.unlisted_hash}<span class="tag">{$t("tools.vimeo.unlisted")}</span>{/if}
              {#if result.used_password}<span class="tag">{$t("tools.vimeo.with_password")}</span>{/if}
              <span class="tag">{result.used_session ? $t("tools.vimeo.session_on") : $t("tools.vimeo.session_off")}</span>
            </div>
            {#if result.file}<div class="group-row-sub mono">{result.file}</div>{/if}
          </div>
          <div class="group-row-trailing btn-row">
            {#if result.file}
              <button class="btn btn-secondary btn-sm" type="button" onclick={() => openPath(result!.file!)}>{$t("tools.common.open")}</button>
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result!.file!)}>{$t("tools.common.reveal")}</button>
            {/if}
          </div>
        </div>
        {#if result.command}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{$t("tools.vimeo.command")}</div>
              <div class="group-row-sub mono">{result.command}</div>
            </div>
          </div>
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .dim { color: var(--text-dim); font-weight: 400; }
  .strong { font-weight: 600; }
</style>
