#!/usr/bin/env node
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const SPEC_URL = "https://docs.scrapecreators.com/openapi.json";
const OUT = resolve(dirname(fileURLToPath(import.meta.url)), "../src-tauri/src/commands/research/endpoints.json");
const PAGING = ["cursor", "max_cursor", "min_time", "continuationToken", "after", "next_max_id", "max_id", "page", "paginationToken", "next_page_id", "pageToken"];
const METHODS = ["get", "post"];

function slug(text) {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

function clean(text, max) {
  const flat = String(text ?? "").replace(/\s+/g, " ").trim();
  return flat.length > max ? `${flat.slice(0, max - 1).trimEnd()}…` : flat;
}

function firstSentence(text) {
  const flat = String(text ?? "").replace(/\s+/g, " ").trim();
  const end = flat.search(/[.!?](\s|$)/);
  return end < 0 ? flat : flat.slice(0, end + 1);
}

function credits(description) {
  const flat = String(description ?? "").replace(/\s+/g, " ");
  const found = flat.split(/(?<=[.!?])\s+/).filter((sentence) => /credit/i.test(sentence));
  return found.length ? clean(found.join(" "), 400) : null;
}

function param(name, source, required, where) {
  const schema = source.schema ?? source;
  const out = { name, in: where, required: Boolean(required), type: Array.isArray(schema.type) ? schema.type[0] : (schema.type ?? "string") };
  if (Array.isArray(schema.enum) && schema.enum.length) out.enum = schema.enum.map(String);
  const description = clean(source.description ?? schema.description, 300);
  if (description) out.description = description;
  const example = source.example ?? schema.example;
  if (example !== undefined && typeof example !== "object") out.example = String(example).slice(0, 120);
  return out;
}

function parameters(op) {
  const out = (op.parameters ?? []).filter((p) => p.in === "query").map((p) => param(p.name, p, p.required, "query"));
  const body = op.requestBody?.content?.["application/json"]?.schema;
  if (body?.properties) {
    const required = new Set(body.required ?? []);
    for (const [name, schema] of Object.entries(body.properties)) out.push(param(name, schema, required.has(name), "body"));
  }
  return out;
}

function build(spec) {
  const operations = [];
  const seen = new Set();
  for (const [path, item] of Object.entries(spec.paths)) {
    for (const method of METHODS) {
      const op = item[method];
      if (!op) continue;
      const base = slug(path.replace(/^\/v(\d+)\//, "/")) || slug(path);
      let id = method === "get" ? base : `${base}_${method}`;
      if (seen.has(id)) id = `${slug(path)}${method === "get" ? "" : `_${method}`}`;
      if (seen.has(id)) throw new Error(`duplicate operation id ${id}`);
      seen.add(id);
      const params = parameters(op);
      const pagination = params.filter((p) => PAGING.includes(p.name)).map((p) => p.name);
      operations.push({
        id,
        platform: slug(op.tags?.[0] ?? path.split("/")[2]),
        group: op.tags?.[0] ?? "",
        method: method.toUpperCase(),
        path,
        summary: clean(`${op.summary ?? ""}: ${firstSentence(op.description)}`.replace(/^: /, ""), 280),
        params,
        pagination,
        credits: credits(op.description),
      });
    }
  }
  operations.sort((a, b) => a.platform.localeCompare(b.platform) || a.id.localeCompare(b.id));
  return { source: SPEC_URL, base: spec.servers?.[0]?.url ?? "https://api.scrapecreators.com", operations };
}

const local = process.argv[2];
const raw = local ? await readFile(local, "utf8") : await (await fetch(SPEC_URL, { headers: { "user-agent": "api-client" } })).text();
const catalog = build(JSON.parse(raw));
await writeFile(OUT, `${JSON.stringify(catalog, null, 1)}\n`);
const platforms = new Set(catalog.operations.map((op) => op.platform));
console.log(`${catalog.operations.length} operations, ${platforms.size} platforms -> ${OUT}`);
