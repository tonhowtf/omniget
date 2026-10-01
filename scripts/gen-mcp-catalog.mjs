#!/usr/bin/env node
// Regenerates src-tauri/omniget-cli/mcp-catalog.json, the offline tool list the
// omniget-mcp binary embeds, from the desktop's MCP tools. Run it after adding,
// removing or renaming an MCP tool. `--check` only fails when the file is stale.
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const tauriDir = fileURLToPath(new URL("../src-tauri", import.meta.url));
const check = process.argv.includes("--check");
const env = { ...process.env };
if (!check) env.OMNIGET_UPDATE_MCP_CATALOG = "1";
const r = spawnSync(
  "cargo",
  ["test", "-p", "omniget", "--lib", "adapter_catalog_matches_the_desktop", "--", "--exact", "mcp::tests::adapter_catalog_matches_the_desktop"],
  { cwd: tauriDir, env, stdio: "inherit" },
);
process.exit(r.status ?? 1);
