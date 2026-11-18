import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = new URL("../scripts/package.mjs", import.meta.url).pathname;

test("the store package drops permissions the store would reject", () => {
  const dir = mkdtempSync(join(tmpdir(), "omniget-store-"));
  try {
    const zip = join(dir, "store.zip");
    execFileSync(process.execPath, [script, "--store", "--output", zip], { stdio: "ignore" });
    const manifest = JSON.parse(execFileSync("unzip", ["-p", zip, "manifest.json"], { encoding: "utf8" }));
    assert.equal(manifest.permissions.includes("alarms"), false);
    assert.equal("key" in manifest, false);
    assert.equal(manifest.host_permissions.some((p) => p.includes("youtube")), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
