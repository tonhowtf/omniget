import type { PDFDocumentLoadingTask, PDFDocumentProxy } from "pdfjs-dist";
import * as api from "./api";
import type { Size } from "./pages";

type PdfJs = typeof import("pdfjs-dist");

const ASSETS = "/vendor/pdfjs/";
const PARALLEL = 2;

let lib: Promise<PdfJs> | null = null;

function pdfjs(): Promise<PdfJs> {
  lib ??= import("pdfjs-dist/legacy/build/pdf.min.mjs").then(() => {
    const loaded = (globalThis as unknown as { pdfjsLib: PdfJs }).pdfjsLib;
    const worker = loaded.GlobalWorkerOptions.workerSrc;
    if (!worker || worker === "./pdf.worker.mjs") loaded.GlobalWorkerOptions.workerSrc = `${ASSETS}pdf.worker.min.mjs`;
    return loaded;
  });
  return lib;
}

export class Thumbs {
  private tasks = new Map<string, Promise<PDFDocumentLoadingTask>>();
  private urls = new Map<string, Promise<string>>();
  private sizes = new Map<string, Size>();
  private active = 0;
  private waiting: (() => void)[] = [];

  private doc(path: string): Promise<PDFDocumentProxy> {
    let task = this.tasks.get(path);
    if (!task) {
      task = Promise.all([pdfjs(), api.bytes(path)]).then(([lib, data]) =>
        lib.getDocument({
          data,
          cMapUrl: `${ASSETS}cmaps/`,
          cMapPacked: true,
          standardFontDataUrl: `${ASSETS}standard_fonts/`,
          wasmUrl: `${ASSETS}wasm/`,
        }),
      );
      task.catch(() => this.tasks.delete(path));
      this.tasks.set(path, task);
    }
    return task.then((t) => t.promise);
  }

  private async slot<T>(work: () => Promise<T>): Promise<T> {
    if (this.active >= PARALLEL) await new Promise<void>((resolve) => this.waiting.push(resolve));
    this.active += 1;
    try {
      return await work();
    } finally {
      this.active -= 1;
      this.waiting.shift()?.();
    }
  }

  size(path: string, page: number): Size | null {
    return this.sizes.get(`${path}#${page}`) ?? null;
  }

  render(path: string, page: number, width: number): Promise<string> {
    const key = `${path}#${page}`;
    let url = this.urls.get(key);
    if (!url) {
      url = this.slot(async () => {
        const doc = await this.doc(path);
        const proxy = await doc.getPage(page + 1);
        try {
          const base = proxy.getViewport({ scale: 1 });
          this.sizes.set(key, { width: base.width, height: base.height });
          const scale = (width * Math.min(2, globalThis.devicePixelRatio || 1)) / base.width;
          const viewport = proxy.getViewport({ scale });
          const canvas = document.createElement("canvas");
          canvas.width = Math.ceil(viewport.width);
          canvas.height = Math.ceil(viewport.height);
          await proxy.render({ canvas, viewport }).promise;
          const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/jpeg", 0.82));
          if (!blob) throw new Error("thumbnail");
          return URL.createObjectURL(blob);
        } finally {
          proxy.cleanup();
        }
      });
      url.catch(() => this.urls.delete(key));
      this.urls.set(key, url);
    }
    return url;
  }

  release(path: string) {
    for (const [key, url] of this.urls) {
      if (key.startsWith(`${path}#`)) {
        void url.then((u) => URL.revokeObjectURL(u)).catch(() => {});
        this.urls.delete(key);
      }
    }
    const task = this.tasks.get(path);
    this.tasks.delete(path);
    void task?.then((t) => t.destroy()).catch(() => {});
  }

  clear() {
    for (const path of [...this.tasks.keys()]) this.release(path);
    for (const url of this.urls.values()) void url.then((u) => URL.revokeObjectURL(u)).catch(() => {});
    this.urls.clear();
    this.sizes.clear();
  }
}
