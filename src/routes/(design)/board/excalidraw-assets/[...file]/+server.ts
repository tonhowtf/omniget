import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { error } from "@sveltejs/kit";
import type { EntryGenerator, RequestHandler } from "./$types";

export const prerender = true;

const ROOT = join(process.cwd(), "node_modules", "@excalidraw", "excalidraw", "dist", "prod");

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    return statSync(full).isDirectory() ? walk(full) : [full];
  });
}

function fontFiles(): Set<string> {
  try {
    return new Set(
      walk(join(ROOT, "fonts"))
        .filter((f) => f.endsWith(".woff2"))
        .map((f) => relative(ROOT, f).split(sep).join("/")),
    );
  } catch {
    return new Set();
  }
}

const FILES = fontFiles();

export const entries: EntryGenerator = () => [...FILES].map((file) => ({ file }));

export const GET: RequestHandler = ({ params }) => {
  if (!FILES.has(params.file)) error(404, "not found");
  const body = readFileSync(join(ROOT, ...params.file.split("/")));
  return new Response(body, {
    headers: { "content-type": "font/woff2", "cache-control": "public, max-age=31536000, immutable" },
  });
};
