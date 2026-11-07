<script lang="ts">
  import type { Root } from "react-dom/client";
  import type { ExcalidrawImperativeAPI, ExcalidrawInitialDataState } from "@excalidraw/excalidraw/types";
  import type { Kit } from "$lib/board/excalidraw";

  let {
    kit,
    initial,
    theme,
    lang,
    label,
    onchange,
    onready,
  }: {
    kit: Kit;
    initial: ExcalidrawInitialDataState;
    theme: "light" | "dark";
    lang: string;
    label: string;
    onchange: () => void;
    onready: (api: ExcalidrawImperativeAPI | null) => void;
  } = $props();

  let host = $state<HTMLDivElement | null>(null);
  let root: Root | null = null;

  function render(currentTheme: "light" | "dark", currentLang: string) {
    if (!root) return;
    const { React, ex } = kit;
    root.render(
      React.createElement(ex.Excalidraw, {
        initialData: initial,
        theme: currentTheme,
        langCode: currentLang,
        autoFocus: true,
        handleKeyboardGlobally: false,
        excalidrawAPI: (api: ExcalidrawImperativeAPI) => onready(api),
        onChange: () => onchange(),
        UIOptions: {
          canvasActions: {
            loadScene: false,
            saveToActiveFile: false,
            saveAsImage: false,
            export: false,
            toggleTheme: false,
          },
          tools: { image: true },
        },
      }),
    );
  }

  $effect(() => {
    if (!host) return;
    root = kit.createRoot(host);
    return () => {
      onready(null);
      const r = root;
      root = null;
      queueMicrotask(() => r?.unmount());
    };
  });

  $effect(() => {
    if (host) render(theme, lang);
  });
</script>

<div class="xh" bind:this={host} role="application" aria-label={label}></div>

<style>
  .xh {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 0;
  }

  .xh :global(.excalidraw) {
    --ui-font: var(--font-body);
  }
</style>
