import { emit } from "@tauri-apps/api/event";

type Args = Record<string, unknown>;

const SAMPLE_PDFS = ["/preview/Downloads/Course notes.pdf", "/preview/Downloads/Receipt.pdf"];
const SAMPLE_IMAGES = ["/preview/Pictures/Scan 1.jpg", "/preview/Pictures/Scan 2.jpg", "/preview/Pictures/Scan 10.png"];
const PAGES: Record<string, number> = { "Course notes.pdf": 12, "Receipt.pdf": 2 };
const cancelled = new Set<string>();

function nameOf(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

function pagesOf(path: string): number {
  return PAGES[nameOf(path)] ?? 3;
}

function samplePdf(title: string, pages: number): Uint8Array {
  const objects: string[] = [];
  const kids: string[] = [];
  const first = 4;
  for (let i = 0; i < pages; i++) {
    const page = first + i * 2;
    const text = `BT /F1 28 Tf 60 720 Td (${title.replace(/[()\\]/g, "")}) Tj 0 -60 Td /F1 96 Tf (${i + 1}) Tj ET`;
    objects[page] = `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >> >> /Contents ${page + 1} 0 R >>`;
    objects[page + 1] = `<< /Length ${text.length} >>\nstream\n${text}\nendstream`;
    kids.push(`${page} 0 R`);
  }
  objects[1] = "<< /Type /Catalog /Pages 2 0 R >>";
  objects[2] = `<< /Type /Pages /Kids [${kids.join(" ")}] /Count ${pages} >>`;
  objects[3] = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";
  let out = "%PDF-1.4\n";
  const offsets: number[] = [];
  for (let n = 1; n < objects.length; n++) {
    offsets[n] = out.length;
    out += `${n} 0 obj\n${objects[n]}\nendobj\n`;
  }
  const xref = out.length;
  out += `xref\n0 ${objects.length}\n0000000000 65535 f \n`;
  for (let n = 1; n < objects.length; n++) out += `${String(offsets[n]).padStart(10, "0")} 00000 n \n`;
  out += `trailer\n<< /Size ${objects.length} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return new TextEncoder().encode(out);
}

async function sampleImage(path: string): Promise<ArrayBuffer> {
  const canvas = document.createElement("canvas");
  canvas.width = 240;
  canvas.height = 320;
  const ctx = canvas.getContext("2d");
  if (ctx) {
    ctx.fillStyle = "#f4efe6";
    ctx.fillRect(0, 0, 240, 320);
    ctx.fillStyle = "#3b3b3b";
    ctx.font = "bold 28px system-ui";
    ctx.fillText(nameOf(path), 20, 160);
  }
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/jpeg"));
  return blob ? blob.arrayBuffer() : new ArrayBuffer(0);
}

async function progress(jobId: string, total: number): Promise<boolean> {
  for (let done = 1; done <= total; done++) {
    await new Promise((r) => setTimeout(r, 180));
    if (cancelled.has(jobId)) return false;
    await emit("pdf-progress", { job_id: jobId, done, total });
  }
  return true;
}

export const pdfFixtures: Record<string, (args: Args) => unknown> = {
  pdf_inspect: ({ paths }) =>
    ((paths as string[]) ?? []).map((path) => ({
      path,
      name: nameOf(path),
      size_bytes: 180_000 * pagesOf(path),
      pages: nameOf(path).startsWith("Locked") ? 0 : pagesOf(path),
      error: nameOf(path).startsWith("Locked") ? "password" : null,
    })),
  pdf_bytes: ({ path }) => samplePdf(nameOf(String(path)), pagesOf(String(path))).buffer,
  pdf_save: ({ outputs }) =>
    ((outputs as { path: string; pages: unknown[] }[]) ?? []).map((o) => ({ path: o.path, size_bytes: 95_000 * o.pages.length })),
  pdf_compress: async ({ jobId, path, outPath }) => {
    const id = String(jobId);
    const ok = await progress(id, 6);
    if (!ok) throw { code: "cancelled", message: "cancelled" };
    const before = 180_000 * pagesOf(String(path));
    if (nameOf(String(path)).startsWith("Receipt")) return { path: null, before_bytes: before, after_bytes: before, images: 0 };
    return { path: outPath, before_bytes: before, after_bytes: Math.round(before * 0.38), images: 6 };
  },
  pdf_expand_images: ({ paths }) => ((paths as string[]) ?? []).flatMap((p) => (/\.(jpe?g|png|webp)$/i.test(p) ? [p] : SAMPLE_IMAGES)),
  pdf_image_thumb: ({ path }) => sampleImage(String(path)),
  pdf_from_images: async ({ jobId, paths, outPath }) => {
    const list = (paths as string[]) ?? [];
    const ok = await progress(String(jobId), list.length);
    if (!ok) throw { code: "cancelled", message: "cancelled" };
    return { path: outPath, size_bytes: 240_000 * list.length };
  },
  pdf_cancel: ({ jobId }) => {
    cancelled.add(String(jobId));
    return null;
  },
  open_path_default: () => null,
};

export function pdfDialog(options: Record<string, unknown> | undefined): unknown {
  if (!location.pathname.startsWith("/pdf")) return undefined;
  if (options?.directory) return "/preview/Pictures";
  const filters = (options?.filters as { extensions: string[] }[] | undefined) ?? [];
  return filters.some((f) => f.extensions.includes("pdf")) ? SAMPLE_PDFS : SAMPLE_IMAGES.slice(0, 2);
}

export function pdfSaveDialog(options: Record<string, unknown> | undefined): unknown {
  if (!location.pathname.startsWith("/pdf")) return undefined;
  return String(options?.defaultPath ?? "/preview/Downloads/Edited.pdf");
}
