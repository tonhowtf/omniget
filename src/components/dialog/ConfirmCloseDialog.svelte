<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import DialogContainer from "./DialogContainer.svelte";
  import { t } from "$lib/i18n";

  let isOpen = $state(false);
  let activeCount = $state(0);

  onMount(() => {
    let unlisten: (() => void) | undefined;
    listen<number>("exit-confirm-required", (event) => {
      activeCount = typeof event.payload === "number" ? event.payload : 0;
      isOpen = true;
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  });

  function cancel() {
    isOpen = false;
  }

  async function confirm() {
    isOpen = false;
    await invoke("force_exit_app");
  }
</script>

<DialogContainer bind:isOpen titleId="confirm-close-title" onClose={cancel}>
  <h3 id="confirm-close-title" class="dialog-title">
    {$t("confirm_close.title")}
  </h3>
  <p class="dialog-body">
    {$t("confirm_close.message", { values: { count: activeCount } })}
  </p>
  <div class="dialog-actions">
    <button type="button" class="button button-secondary" onclick={cancel}>
      {$t("confirm_close.cancel")}
    </button>
    <button type="button" class="button button-danger" onclick={confirm}>
      {$t("confirm_close.confirm")}
    </button>
  </div>
</DialogContainer>

<style>
  .dialog-title {
    margin: 0;
    padding: calc(var(--padding) * 1.25) calc(var(--padding) * 1.5) calc(var(--padding) * 0.75);
    font-size: 15px;
    font-weight: 600;
    color: var(--secondary);
  }

  .dialog-body {
    margin: 0;
    padding: 0 calc(var(--padding) * 1.5) calc(var(--padding) * 1.25);
    font-size: 13px;
    line-height: 1.5;
    color: var(--secondary);
    opacity: 0.85;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: calc(var(--padding) * 0.5);
    padding: calc(var(--padding) * 0.75) calc(var(--padding) * 1.5) calc(var(--padding) * 1.25);
    border-top: 1px solid var(--content-border);
  }

  .button {
    padding: 8px 16px;
    font-size: 12.5px;
    font-weight: 500;
    border-radius: var(--border-radius);
    border: none;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }

  .button-secondary {
    background: transparent;
    color: var(--secondary);
    border: 1px solid var(--content-border);
  }

  .button-secondary:hover {
    background: var(--sidebar-highlight);
  }

  .button-danger {
    background: var(--red, #ed2236);
    color: var(--on-accent, #fff);
  }

  .button-danger:hover {
    filter: brightness(1.08);
  }

  .button:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
</style>
