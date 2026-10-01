<script lang="ts">
  import { t } from "$lib/i18n";

  let {
    busy = false,
    errorKey = null,
    oninstalldir,
    oninstallzip,
    oninstallgit,
    onclose,
  }: {
    busy?: boolean;
    errorKey?: string | null;
    oninstalldir?: (path: string) => void;
    oninstallzip?: (path: string) => void;
    oninstallgit?: (url: string) => void;
    onclose?: () => void;
  } = $props();

  let gitUrl = $state("");
  let confirmingGit = $state(false);
  let pickerError = $state(false);

  let gitValid = $derived(/^(https?:\/\/|git@|ssh:\/\/)\S+$/.test(gitUrl.trim()));

  async function pick(kind: "dir" | "zip") {
    pickerError = false;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open(
        kind === "dir"
          ? { directory: true, multiple: false }
          : { multiple: false, filters: [{ name: "zip", extensions: ["zip"] }] },
      );
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (typeof path !== "string" || !path) return;
      if (kind === "dir") oninstalldir?.(path);
      else oninstallzip?.(path);
    } catch {
      pickerError = true;
    }
  }

  function submitGit() {
    if (!gitValid || busy) return;
    if (!confirmingGit) {
      confirmingGit = true;
      return;
    }
    confirmingGit = false;
    oninstallgit?.(gitUrl.trim());
  }
</script>

<div
  class="overlay"
  role="presentation"
  onclick={(e) => {
    if (e.target === e.currentTarget) onclose?.();
  }}
>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={$t("llm.skills.install_title")}>
    <header class="head">
      <h2>{$t("llm.skills.install_title")}</h2>
      <button type="button" class="close" onclick={() => onclose?.()} aria-label={$t("llm.skills.close")}>
        ×
      </button>
    </header>

    <div class="body">
      <p class="lede">{$t("llm.skills.install_lede")}</p>

      <section class="source">
        <div class="source-text">
          <h3>{$t("llm.skills.from_dir")}</h3>
          <p>{$t("llm.skills.from_dir_hint")}</p>
        </div>
        <button type="button" class="button" disabled={busy} onclick={() => pick("dir")}>
          {$t("llm.skills.choose_folder")}
        </button>
      </section>

      <section class="source">
        <div class="source-text">
          <h3>{$t("llm.skills.from_zip")}</h3>
          <p>{$t("llm.skills.from_zip_hint")}</p>
        </div>
        <button type="button" class="button" disabled={busy} onclick={() => pick("zip")}>
          {$t("llm.skills.choose_zip")}
        </button>
      </section>

      <section class="source column">
        <div class="source-text">
          <h3>{$t("llm.skills.from_git")}</h3>
          <p>{$t("llm.skills.from_git_hint")}</p>
        </div>
        <form
          class="git-row"
          onsubmit={(e) => {
            e.preventDefault();
            submitGit();
          }}
        >
          <input
            class="input"
            type="url"
            bind:value={gitUrl}
            oninput={() => (confirmingGit = false)}
            placeholder="https://github.com/OpenRouterTeam/skills"
            aria-label={$t("llm.skills.from_git")}
          />
          <button type="submit" class="button" disabled={!gitValid || busy}>
            {#if busy}
              {$t("llm.skills.installing")}
            {:else if confirmingGit}
              {$t("llm.skills.clone_confirm")}
            {:else}
              {$t("llm.skills.clone")}
            {/if}
          </button>
        </form>
        {#if confirmingGit}
          <p class="warn" role="status">{$t("llm.skills.clone_warning")}</p>
        {/if}
      </section>

      {#if pickerError}
        <p class="error" role="status">{$t("llm.skills.err_picker")}</p>
      {/if}
      {#if errorKey}
        <p class="error" role="alert">{$t(errorKey)}</p>
      {/if}
    </div>

    <footer class="foot">
      <button type="button" class="button" onclick={() => onclose?.()}>{$t("llm.skills.close")}</button>
    </footer>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: grid;
    place-items: center;
    z-index: 900;
  }

  .dialog {
    width: min(560px, 92vw);
    max-height: 86vh;
    overflow: auto;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3) var(--space-4);
  }

  .head h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
  }

  .close {
    background: transparent;
    border: 0;
    color: var(--text-dim);
    font-size: 20px;
    line-height: 1;
    cursor: pointer;
  }

  .body {
    padding: 0 var(--space-4) var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .lede {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .source {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    box-shadow: inset 0 0 0 var(--hairline) var(--separator);
  }

  .source.column {
    flex-direction: column;
    align-items: stretch;
  }

  .source-text h3 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
  }

  .source-text p {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .git-row {
    display: flex;
    gap: var(--space-2);
  }

  .git-row .input {
    flex: 1;
    min-width: 0;
  }

  .warn {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--warning, var(--text-muted));
  }

  .error {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--danger);
  }

  .foot {
    padding: var(--space-3) var(--space-4);
    display: flex;
    justify-content: flex-end;
  }
</style>
