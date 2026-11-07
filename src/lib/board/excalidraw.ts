import type * as ExcalidrawModule from "@excalidraw/excalidraw";
import type * as ReactModule from "react";
import type * as ReactDomClient from "react-dom/client";
import type { AppState, BinaryFiles, ExcalidrawInitialDataState } from "@excalidraw/excalidraw/types";
import type { ExcalidrawElement } from "@excalidraw/excalidraw/element/types";
import type { ExportOptions } from "./types";

export const ASSET_PATH = "/board/excalidraw-assets/";
export const THUMB_MAX_PX = 480;

export type Kit = {
  React: typeof ReactModule;
  createRoot: typeof ReactDomClient.createRoot;
  ex: typeof ExcalidrawModule;
};

export type Scene = {
  elements: readonly ExcalidrawElement[];
  appState: AppState;
  files: BinaryFiles;
};

let loading: Promise<Kit> | null = null;

export function loadKit(): Promise<Kit> {
  if (!loading) {
    (window as unknown as { EXCALIDRAW_ASSET_PATH?: string[] }).EXCALIDRAW_ASSET_PATH = [ASSET_PATH];
    loading = Promise.all([
      import("react"),
      import("react-dom/client"),
      import("@excalidraw/excalidraw"),
      import("@excalidraw/excalidraw/index.css"),
    ])
      .then(([React, dom, ex]) => ({ React, createRoot: dom.createRoot, ex }))
      .catch((error) => {
        loading = null;
        throw error;
      });
  }
  return loading;
}

export function parseScene(contents: string): ExcalidrawInitialDataState {
  const data = JSON.parse(contents) as { elements?: unknown; appState?: Record<string, unknown>; files?: unknown };
  if (!data || typeof data !== "object" || !Array.isArray(data.elements)) throw new Error("BOARD_NOT_SCENE");
  return {
    elements: data.elements as ExcalidrawInitialDataState["elements"],
    appState: (data.appState ?? {}) as ExcalidrawInitialDataState["appState"],
    files: (data.files ?? {}) as BinaryFiles,
    scrollToContent: true,
  };
}

export function signature(kit: Kit, scene: Scene): string {
  return `${kit.ex.hashElementsVersion(scene.elements)}|${scene.appState.viewBackgroundColor}|${Object.keys(scene.files).length}`;
}

export function serialize(kit: Kit, scene: Scene): string {
  return kit.ex.serializeAsJSON(scene.elements, scene.appState, scene.files, "local");
}

function visible(kit: Kit, scene: Scene) {
  return kit.ex.getNonDeletedElements(scene.elements);
}

export async function thumbnail(kit: Kit, scene: Scene): Promise<Blob> {
  return kit.ex.exportToBlob({
    elements: visible(kit, scene),
    appState: { ...scene.appState, exportBackground: true, exportWithDarkMode: false },
    files: scene.files,
    mimeType: "image/png",
    maxWidthOrHeight: THUMB_MAX_PX,
    exportPadding: 16,
  });
}

export async function toPng(kit: Kit, scene: Scene, opts: ExportOptions): Promise<Blob> {
  return kit.ex.exportToBlob({
    elements: visible(kit, scene),
    appState: { ...scene.appState, exportBackground: opts.background, exportWithDarkMode: opts.dark, exportScale: opts.scale },
    files: scene.files,
    mimeType: "image/png",
    getDimensions: (width: number, height: number) => ({ width: width * opts.scale, height: height * opts.scale, scale: opts.scale }),
  });
}

export async function toSvg(kit: Kit, scene: Scene, opts: ExportOptions): Promise<string> {
  const svg = await kit.ex.exportToSvg({
    elements: visible(kit, scene),
    appState: { ...scene.appState, exportBackground: opts.background, exportWithDarkMode: opts.dark },
    files: scene.files,
  });
  return new XMLSerializer().serializeToString(svg);
}

export function isEmpty(kit: Kit, scene: Scene): boolean {
  return visible(kit, scene).length === 0;
}
