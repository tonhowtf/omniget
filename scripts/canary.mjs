#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const TARGETS = [
  { platform: "youtube", extractor: "youtube", url: "https://www.youtube.com/watch?v=jNQXAC9IVRw" },
  { platform: "tiktok", extractor: "tiktok", url: "https://www.tiktok.com/@scout2015/video/6718335390845095173" },
  { platform: "x", extractor: "twitter", url: "https://x.com/captainamerica/status/719944021058060289" },
  { platform: "reddit", extractor: "reddit", url: "https://www.reddit.com/r/videos/comments/6rrwyj/that_small_heart_attack/" },
  { platform: "bluesky", extractor: "bluesky", url: "https://bsky.app/profile/bsky.app/post/3l3vgf77uco2g" },
  { platform: "pinterest", extractor: "pinterest", url: "https://www.pinterest.com/pin/664281013778109217/" },
  { platform: "twitch", extractor: "twitch", url: "https://clips.twitch.tv/FaintLightGullWholeWheat" },
  { platform: "vimeo", extractor: "vimeo", url: "https://player.vimeo.com/video/76979871" },
  { platform: "bilibili", extractor: "bilibili", url: "https://www.bilibili.com/video/BV1bK411W797" },
  { platform: "instagram", extractor: "instagram", url: "https://www.instagram.com/p/BQ0eAlwhDrw/" },
  { platform: "threads", extractor: "threads", url: "https://www.threads.com/@zuck/post/DHV7vTivqWD" },
];

const argv = process.argv.slice(2);
const flag = (name) => argv.includes(name);
const option = (name) => {
  const i = argv.indexOf(name);
  if (i >= 0) return argv[i + 1];
  const eq = argv.find((a) => a.startsWith(`${name}=`));
  return eq ? eq.slice(name.length + 1) : undefined;
};
const asJson = flag("--json");
const realProfile = flag("--real-profile");
const timeoutMs = Number(option("--timeout") ?? 120) * 1000;
const only = option("--only")?.split(",").map((s) => s.trim().toLowerCase());
const log = (...a) => (asJson ? console.error(...a) : console.log(...a));

const targets = only ? TARGETS.filter((t) => only.includes(t.platform)) : TARGETS;
if (!targets.length) {
  console.error(`canary: nothing to run; platforms: ${TARGETS.map((t) => t.platform).join(", ")}`);
  process.exit(2);
}

function buildCli() {
  log("canary: building omniget-cli (cargo build -p omniget-cli)...");
  const r = spawnSync("cargo", ["build", "-p", "omniget-cli", "--bin", "omniget-cli", "--message-format=json-render-diagnostics"], {
    cwd: join(ROOT, "src-tauri"),
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
    maxBuffer: 256 * 1024 * 1024,
    shell: process.platform === "win32",
  });
  if (r.status !== 0) {
    console.error("canary: cargo build failed");
    process.exit(2);
  }
  let exe = null;
  for (const line of r.stdout.split("\n")) {
    if (!line.startsWith("{")) continue;
    try {
      const msg = JSON.parse(line);
      if (msg.reason === "compiler-artifact" && msg.target?.name === "omniget-cli" && msg.executable) exe = msg.executable;
    } catch {
    }
  }
  if (!exe) {
    console.error("canary: cargo did not report the omniget-cli executable");
    process.exit(2);
  }
  return exe;
}

const bin = option("--bin") ? resolve(option("--bin")) : buildCli();
if (!existsSync(bin)) {
  console.error(`canary: binary not found: ${bin}`);
  process.exit(2);
}

const keptProfile = option("--profile") ? resolve(option("--profile")) : null;
if (keptProfile) mkdirSync(keptProfile, { recursive: true });
const profile = realProfile ? null : (keptProfile ?? mkdtempSync(join(tmpdir(), "omniget-canary-")));
const FIRST_TIMEOUT_MS = Math.max(timeoutMs, 600_000);
const env = { ...process.env, RUST_LOG: process.env.RUST_LOG ?? "warn" };
if (profile) env.OMNIGET_DATA_DIR = profile;

function probe(t, limitMs) {
  return new Promise((done) => {
    const started = Date.now();
    const child = spawn(bin, ["--json", "info", t.url], { env, stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    child.stdout.on("data", (c) => (out += c));
    child.stderr.on("data", (c) => (err += c));
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
    }, limitMs);
    const finish = (code) => {
      clearTimeout(timer);
      const ms = Date.now() - started;
      const base = { platform: t.platform, url: t.url, expected: t.extractor, ms };
      if (timedOut) return done({ ...base, status: "FAIL", detail: `timed out after ${limitMs / 1000}s` });
      let info = null;
      for (const line of out.split("\n").reverse()) {
        if (!line.trim().startsWith("{")) continue;
        try {
          info = JSON.parse(line);
          break;
        } catch {
        }
      }
      if (code !== 0 || !info) {
        const lastErr = (err || out)
          .split("\n")
          .map((l) => l.trim())
          .filter(Boolean)
          .pop();
        return done({ ...base, status: "FAIL", detail: `exit ${code}: ${lastErr ?? "no output"}` });
      }
      const title = String(info.title ?? "").trim();
      if (info.platform !== t.extractor) {
        return done({ ...base, status: "FALLBACK", extractor: info.platform, title, detail: `native "${t.extractor}" extractor failed; "${info.platform}" answered` });
      }
      if (!title) return done({ ...base, status: "FAIL", extractor: info.platform, detail: "resolved with an empty title" });
      done({ ...base, status: "PASS", extractor: info.platform, title, formats: info.format_count, detail: title });
    };
    child.on("exit", finish);
    child.on("error", (e) => {
      clearTimeout(timer);
      done({ platform: t.platform, url: t.url, expected: t.extractor, ms: Date.now() - started, status: "FAIL", detail: e.message });
    });
  });
}

const results = [];
for (const [i, t] of targets.entries()) {
  log(`canary: ${t.platform} ...`);
  results.push(await probe(t, i === 0 && !realProfile ? FIRST_TIMEOUT_MS : timeoutMs));
}

if (profile && !keptProfile) {
  try {
    rmSync(profile, { recursive: true, force: true });
  } catch {
  }
}

const failed = results.filter((r) => r.status !== "PASS");
const clip = (s, n) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

if (asJson) {
  console.log(JSON.stringify({ ok: failed.length === 0, bin, results }, null, 2));
} else {
  const rows = results.map((r) => [r.platform, r.status, r.extractor ?? "-", r.formats === undefined ? "-" : String(r.formats), `${(r.ms / 1000).toFixed(1)}s`, clip(r.detail ?? "", 90)]);
  const head = ["platform", "status", "extractor", "formats", "time", "detail"];
  const widths = head.map((h, i) => Math.max(h.length, ...rows.map((r) => r[i].length)));
  const fmt = (r) => r.map((c, i) => (i === r.length - 1 ? c : c.padEnd(widths[i]))).join("  ");
  console.log(`\n${fmt(head)}\n${widths.map((w) => "-".repeat(w)).join("  ")}`);
  for (const r of rows) console.log(fmt(r));
  console.log(`\n${results.length - failed.length}/${results.length} platforms pass`);
}

if (process.env.GITHUB_STEP_SUMMARY) {
  const md = [
    "### Platform canary",
    "",
    "| platform | status | extractor | time | detail |",
    "|---|---|---|---|---|",
    ...results.map((r) => `| ${r.platform} | ${r.status === "PASS" ? "✅" : "❌"} ${r.status} | ${r.extractor ?? "-"} | ${(r.ms / 1000).toFixed(1)}s | ${clip(r.detail ?? "", 120).replace(/\|/g, "\\|")} |`),
    "",
  ].join("\n");
  try {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${md}\n`);
  } catch {
  }
}

process.exit(failed.length ? 1 : 0);
