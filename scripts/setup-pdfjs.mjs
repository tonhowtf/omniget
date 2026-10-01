import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dist = path.join(root, "node_modules", "pdfjs-dist");
const out = path.join(root, "static", "vendor", "pdfjs");

export function setupPdfjs() {
  if (!existsSync(dist)) return false;
  const version = JSON.parse(readFileSync(path.join(dist, "package.json"), "utf8")).version;
  const stamp = path.join(out, ".version");
  if (existsSync(stamp) && readFileSync(stamp, "utf8") === version) return true;
  mkdirSync(out, { recursive: true });
  cpSync(path.join(dist, "legacy", "build", "pdf.worker.min.mjs"), path.join(out, "pdf.worker.min.mjs"));
  for (const dir of ["wasm", "cmaps", "standard_fonts"]) {
    if (existsSync(path.join(dist, dir))) cpSync(path.join(dist, dir), path.join(out, dir), { recursive: true });
  }
  const css = readFileSync(path.join(dist, "web", "pdf_viewer.css"), "utf8");
  writeFileSync(path.join(out, "text_layer_builder.css"), css);
  writeFileSync(path.join(out, "annotation_layer_builder.css"), css);
  writeFileSync(stamp, version);
  return true;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  console.log(setupPdfjs() ? "pdf.js runtime ready in static/vendor/pdfjs" : "pdfjs-dist not installed");
}
