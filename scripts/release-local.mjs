#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const USAGE = `usage: node scripts/release-local.mjs [options]

Builds the signed desktop release of this machine into dist-release/v<version>/
and writes SHA256SUMS and latest.json there. Nothing is published.

  --target <triple>   Rust target (default: aarch64-apple-darwin)
  --out <dir>         output root (default: dist-release)
  --notes <file>      patch notes for latest.json (default: docs/releases/v<version>.md if it exists)
  --base <url>        download base for latest.json (default: https://getomniget.com/dl)
  --cli               also build and pack omniget-cli-<version>-<triple>.tar.gz (.zip on Windows)
  --bundles <list>    bundle formats passed to tauri build (default: all of tauri.conf.json)
  --skip-build        only collect bundles already built for the target
  --allow-dirty       do not stop on uncommitted changes
  --dry-run           check versions, tag and signing key, print the plan, build nothing
  --stage <name>      Windows only, for signing done outside this machine (SignPath):
                        compile  build without bundling and copy every executable to <sign-dir>/binaries
                        bundle   take the signed files from <sign-dir>/binaries-signed, build the
                                 installers and copy them to <sign-dir>/installers
                        collect  take the signed installers from <sign-dir>/installers-signed, redo
                                 the updater signature and write the release folder
  --sign-dir <dir>    folder shared by the stages (default: signpath)

The updater key is TAURI_SIGNING_PRIVATE_KEY or ~/.tauri/omniget-updater.key; its password is
TAURI_SIGNING_PRIVATE_KEY_PASSWORD or the Keychain item "omniget-updater".`;

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const args = process.argv.slice(2);
if (args.includes("--help") || args.includes("-h")) {
  console.log(USAGE);
  process.exit(0);
}
const flag = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const has = (name) => args.includes(name);

const target = flag("--target") ?? "aarch64-apple-darwin";
const dryRun = has("--dry-run");
const base = flag("--base") ?? "https://getomniget.com/dl";

function fail(message) {
  console.error(`release-local: ${message}`);
  process.exit(1);
}

const stage = flag("--stage") ?? "all";
if (!["all", "compile", "bundle", "collect"].includes(stage)) fail(`unknown stage ${stage}`);
if (stage !== "all" && !target.includes("windows")) fail("--stage only applies to the Windows targets");

function run(cmd, cmdArgs, opts = {}) {
  console.log(`$ ${cmd} ${cmdArgs.join(" ")}`);
  const shell = process.platform === "win32" && cmd === "pnpm";
  const r = spawnSync(cmd, cmdArgs, { cwd: root, stdio: "inherit", shell, ...opts });
  if (r.status !== 0) fail(`${cmd} exited with ${r.status}${r.error ? ` (${r.error.message})` : ""}`);
}

function capture(cmd, cmdArgs) {
  const r = spawnSync(cmd, cmdArgs, { cwd: root, encoding: "utf8" });
  return r.status === 0 ? r.stdout.trim() : null;
}

const read = (p) => readFileSync(join(root, p), "utf8");
const tomlVersion = (p) => read(p).match(/^version\s*=\s*"([^"]+)"/m)?.[1];

const version = JSON.parse(read("package.json")).version;
const versions = {
  "package.json": version,
  "src-tauri/tauri.conf.json": JSON.parse(read("src-tauri/tauri.conf.json")).version,
  "src-tauri/Cargo.toml": tomlVersion("src-tauri/Cargo.toml"),
  "src-tauri/omniget-cli/Cargo.toml": tomlVersion("src-tauri/omniget-cli/Cargo.toml"),
};
const mismatched = Object.entries(versions).filter(([, v]) => v !== version);
if (mismatched.length) {
  fail(`versions differ: ${Object.entries(versions).map(([f, v]) => `${f}=${v}`).join(", ")}; run node scripts/bump-version.js ${version}`);
}

const dirty = capture("git", ["status", "--porcelain", "--untracked-files=no"]);
if (dirty && !has("--allow-dirty")) fail("uncommitted changes; commit them or pass --allow-dirty");
const tag = `v${version}`;
const tagged = capture("git", ["rev-parse", "-q", "--verify", `refs/tags/${tag}`]);
const head = capture("git", ["rev-parse", "HEAD"]);
const tagCommit = tagged ? capture("git", ["rev-list", "-n", "1", tag]) : null;
if (!tagged) console.warn(`warning: no local tag ${tag} yet`);
else if (tagCommit !== head) console.warn(`warning: ${tag} points to ${tagCommit?.slice(0, 8)}, HEAD is ${head?.slice(0, 8)}`);

function signingKey() {
  if (process.env.TAURI_SIGNING_PRIVATE_KEY) return process.env.TAURI_SIGNING_PRIVATE_KEY;
  const file = join(homedir(), ".tauri", "omniget-updater.key");
  if (!existsSync(file)) fail(`no updater key: set TAURI_SIGNING_PRIVATE_KEY or create ${file}`);
  return readFileSync(file, "utf8").trim();
}

function signingPassword() {
  if (process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD !== undefined) return process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD;
  if (process.platform === "darwin") {
    const pw = capture("security", ["find-generic-password", "-s", "omniget-updater", "-w"]);
    if (pw !== null) return pw;
  }
  fail("no key password: set TAURI_SIGNING_PRIVATE_KEY_PASSWORD or add the Keychain item omniget-updater");
}

const key = signingKey();
const password = signingPassword();

const outDir = resolve(root, flag("--out") ?? "dist-release", tag);
const defaultNotes = join(root, "docs", "releases", `${tag}.md`);
const notes = flag("--notes") ?? (existsSync(defaultNotes) ? defaultNotes : undefined);
const arch = { "aarch64-apple-darwin": "aarch64", "x86_64-apple-darwin": "x64" }[target];
const isMac = target.endsWith("apple-darwin");
const cargoTarget = JSON.parse(capture("cargo", ["metadata", "--no-deps", "--format-version", "1", "--manifest-path", join(root, "src-tauri/Cargo.toml")]) ?? "{}").target_directory;
if (!cargoTarget) fail("cannot resolve the cargo target directory");
const bundleDir = join(cargoTarget, target, "release", "bundle");

console.log(`release ${tag} for ${target}`);
console.log(`  output   ${outDir}`);
console.log(`  bundles  ${bundleDir}`);
console.log(`  notes    ${notes ?? "(none)"}`);
console.log(`  key      ${key.length} bytes, password ${password ? "set" : "empty"}`);
console.log(`  cli      ${has("--cli") ? "yes" : "no"}`);
if (stage !== "all") console.log(`  stage    ${stage}`);
if (!isMac) console.warn("warning: only the macOS targets are tested; Windows and Linux bundles are collected best-effort");
if (dryRun) {
  console.log("dry run: nothing built");
  process.exit(0);
}

const env = { ...process.env, TAURI_SIGNING_PRIVATE_KEY: key, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: password };
for (const name of Object.keys(env)) if (name.startsWith("APPLE_") && !env[name]) delete env[name];
const windowsSign = target.includes("windows") ? process.env.WINDOWS_SIGN_COMMAND : "";
const releaseDir = join(cargoTarget, target, "release");
const binExt = target.includes("windows") ? ".exe" : "";
const cliBins = ["omniget-cli", "omniget-mcp", "omniget-gateway", "omniget-worker"].map((b) => `${b}${binExt}`);
const signDir = resolve(root, flag("--sign-dir") ?? "signpath");
const mainExe = join(releaseDir, "omniget.exe");
const sidecars = JSON.parse(read("src-tauri/tauri.conf.json")).bundle.externalBin.map((b) => `${basename(b)}-${target}.exe`);
const installerKinds = (flag("--bundles") ?? "nsis,msi").split(",").filter((k) => k === "nsis" || k === "msi");
const BUNDLE_TOKEN = Buffer.from("__TAURI_BUNDLE_TYPE_VAR_UNK");
const BUNDLE_TAG = { nsis: "NSS", msi: "MSI" };

function buildCli() {
  run("cargo", ["build", "-p", "omniget-cli", "--release", "--target", target], { cwd: join(root, "src-tauri") });
  for (const b of cliBins) if (!existsSync(join(releaseDir, b))) fail(`missing ${b} in ${releaseDir}`);
  if (!binExt || process.platform === "win32") run(join(releaseDir, `omniget-cli${binExt}`), ["--version"]);
}

function notaryCredentials() {
  if (env.APPLE_API_KEY && env.APPLE_API_ISSUER && env.APPLE_API_KEY_PATH) {
    return ["--key", env.APPLE_API_KEY_PATH, "--key-id", env.APPLE_API_KEY, "--issuer", env.APPLE_API_ISSUER];
  }
  if (env.APPLE_ID && env.APPLE_PASSWORD && env.APPLE_TEAM_ID) {
    return ["--apple-id", env.APPLE_ID, "--password", env.APPLE_PASSWORD, "--team-id", env.APPLE_TEAM_ID];
  }
  return null;
}

function signMacBinaries(files) {
  for (const f of files) run("codesign", ["--force", "--options", "runtime", "--timestamp", "--sign", env.APPLE_SIGNING_IDENTITY, f]);
  const credentials = notaryCredentials();
  if (!credentials) {
    console.warn("warning: no notarization credentials (APPLE_API_* or APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID); the CLI is signed but not notarized");
    return;
  }
  const zip = join(cargoTarget, `omniget-cli-${version}-notarize.zip`);
  rmSync(zip, { force: true });
  run("zip", ["-j", "-q", zip, ...files]);
  console.log(`$ xcrun notarytool submit ${basename(zip)} --wait`);
  const r = spawnSync("xcrun", ["notarytool", "submit", zip, "--wait", "--output-format", "json", ...credentials], { cwd: root, encoding: "utf8" });
  rmSync(zip, { force: true });
  let status;
  try {
    status = JSON.parse(r.stdout).status;
  } catch {
    status = undefined;
  }
  if (r.status !== 0 || status !== "Accepted") fail(`notarization of the CLI ended as ${status ?? "unknown"}: ${(r.stderr || r.stdout).trim().slice(0, 600)}`);
  console.log("  notarized: Accepted");
}

function withBundleType(exe, kind) {
  const at = exe.indexOf(BUNDLE_TOKEN);
  const out = Buffer.from(exe);
  if (at < 0) console.warn(`warning: no bundle type marker in omniget.exe; the ${kind} copy is left as built`);
  else out.write(`__TAURI_BUNDLE_TYPE_VAR_${BUNDLE_TAG[kind]}`, at, "latin1");
  return out;
}

function signedFile(dir, name, original) {
  const file = join(dir, name);
  if (!existsSync(file)) fail(`${name} did not come back signed in ${dir}`);
  if (original && readFileSync(file).equals(readFileSync(original))) fail(`${name} in ${dir} is identical to the unsigned file`);
  if (process.platform === "win32") {
    const status = capture("powershell", ["-NoProfile", "-Command", `(Get-AuthenticodeSignature -LiteralPath '${file.replaceAll("'", "''")}').Status`]);
    if (!status || status === "NotSigned") fail(`${name} in ${dir} carries no Authenticode signature`);
  }
  return file;
}

function compileForSigning() {
  run("rustup", ["target", "add", target]);
  run("pnpm", ["tauri", "build", "--target", target, "--no-bundle"], { env });
  if (!existsSync(mainExe)) fail(`missing ${mainExe}`);
  if (has("--cli")) buildCli();
  const dir = join(signDir, "binaries");
  rmSync(signDir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  const main = readFileSync(mainExe);
  writeFileSync(join(dir, "omniget.exe"), main);
  for (const kind of installerKinds) writeFileSync(join(dir, `omniget-${kind}.exe`), withBundleType(main, kind));
  for (const s of sidecars) copyFileSync(join(root, "src-tauri", "binaries", s), join(dir, s));
  if (has("--cli")) for (const b of cliBins) copyFileSync(join(releaseDir, b), join(dir, b));
  console.log(`\nto sign: ${readdirSync(dir).join(", ")} in ${dir}`);
}

function bundleSigned() {
  const unsigned = join(signDir, "binaries");
  const signed = join(signDir, "binaries-signed");
  const take = (name) => signedFile(signed, name, join(unsigned, name));
  for (const s of sidecars) {
    copyFileSync(take(s), join(root, "src-tauri", "binaries", s));
    const built = join(releaseDir, s.replace(`-${target}`, ""));
    if (existsSync(built)) copyFileSync(take(s), built);
  }
  if (has("--cli")) for (const b of cliBins) copyFileSync(take(b), join(releaseDir, b));
  for (const kind of installerKinds) {
    copyFileSync(take(`omniget-${kind}.exe`), mainExe);
    run("pnpm", ["tauri", "bundle", "--target", target, "--bundles", kind], { env });
  }
  copyFileSync(take("omniget.exe"), mainExe);
  const dir = join(signDir, "installers");
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  for (const [kind, match] of [["nsis", (f) => f.endsWith("-setup.exe")], ["msi", (f) => f.endsWith(".msi")]]) {
    if (!installerKinds.includes(kind)) continue;
    const found = readdirSync(join(bundleDir, kind)).filter(match);
    if (!found.length) fail(`no ${kind} installer in ${join(bundleDir, kind)}`);
    for (const f of found) copyFileSync(join(bundleDir, kind, f), join(dir, f));
  }
  console.log(`\nto sign: ${readdirSync(dir).join(", ")} in ${dir}`);
}

function installSignedInstallers() {
  const unsigned = join(signDir, "installers");
  const signed = join(signDir, "installers-signed");
  for (const f of readdirSync(unsigned)) {
    const dest = join(bundleDir, f.endsWith(".msi") ? "msi" : "nsis", f);
    copyFileSync(signedFile(signed, f, join(unsigned, f)), dest);
    if (existsSync(`${dest}.sig`)) {
      rmSync(`${dest}.sig`);
      run("pnpm", ["tauri", "signer", "sign", dest], { env });
      if (!existsSync(`${dest}.sig`)) fail(`no updater signature written for ${f}`);
    }
  }
}

if (stage === "compile") {
  compileForSigning();
  process.exit(0);
}
if (stage === "bundle") {
  bundleSigned();
  process.exit(0);
}
if (stage === "collect") installSignedInstallers();
if (stage === "all" && !has("--skip-build")) {
  run("rustup", ["target", "add", target]);
  const bundles = flag("--bundles");
  const signConfig = windowsSign ? ["--config", JSON.stringify({ bundle: { windows: { signCommand: windowsSign } } })] : [];
  run("pnpm", ["tauri", "build", "--target", target, ...signConfig, ...(bundles ? ["--bundles", bundles] : [])], { env });
}

mkdirSync(outDir, { recursive: true });
const copied = [];
function collect(dir, match, rename) {
  if (!existsSync(dir)) return;
  for (const f of readdirSync(dir)) {
    if (!match(f)) continue;
    const name = rename(f);
    copyFileSync(join(dir, f), join(outDir, name));
    copied.push(name);
  }
}

if (isMac) {
  collect(join(bundleDir, "dmg"), (f) => f.endsWith(".dmg"), (f) => f);
  collect(
    join(bundleDir, "macos"),
    (f) => f.endsWith(".app.tar.gz") || f.endsWith(".app.tar.gz.sig"),
    (f) => f.replace(/^.*\.app\.tar\.gz/, `omniget_${version}_${arch}.app.tar.gz`),
  );
} else {
  collect(join(bundleDir, "appimage"), (f) => /\.AppImage(\.sig)?$/.test(f), (f) => f);
  collect(join(bundleDir, "deb"), (f) => f.endsWith(".deb"), (f) => f);
  collect(join(bundleDir, "rpm"), (f) => f.endsWith(".rpm"), (f) => f);
  collect(join(bundleDir, "nsis"), (f) => /-setup\.exe(\.sig)?$/.test(f), (f) => f);
  collect(join(bundleDir, "msi"), (f) => f.endsWith(".msi"), (f) => f);
  if (target.includes("windows")) {
    const exe = join(cargoTarget, target, "release", "omniget.exe");
    const portable = `omniget_${version}_x64-portable.exe`;
    if (existsSync(exe)) {
      copyFileSync(exe, join(outDir, portable));
      copied.push(portable);
    }
  }
}
if (!copied.some((f) => f.endsWith(".sig"))) fail(`no signed updater bundle in ${bundleDir}`);

if (has("--cli")) {
  if (stage === "all") buildCli();
  const binDir = releaseDir;
  const bins = cliBins;
  if (target.includes("windows")) {
    if (windowsSign && stage === "all") for (const b of bins) { const signed = spawnSync(windowsSign.replaceAll("%1", `"${join(binDir, b)}"`), { shell: true, stdio: "inherit" }); if (signed.status !== 0) fail(`could not sign ${b}`); }
    const archive = `omniget-cli-${version}-${target}.zip`;
    run("zip", ["-j", "-q", join(outDir, archive), ...bins.map((b) => join(binDir, b))]);
    copied.push(archive);
  } else {
    if (isMac && env.APPLE_SIGNING_IDENTITY && stage === "all") signMacBinaries(bins.map((b) => join(binDir, b)));
    const tarball = `omniget-cli-${version}-${target}.tar.gz`;
    run("tar", ["-czf", join(outDir, tarball), "-C", binDir, ...bins]);
    copied.push(tarball);
  }
}

const sums = readdirSync(outDir)
  .filter((f) => f.includes(version) && !f.endsWith(".sig"))
  .sort()
  .map((f) => `${createHash("sha256").update(readFileSync(join(outDir, f))).digest("hex")}  ${f}`);
writeFileSync(join(outDir, "SHA256SUMS"), sums.join("\n") + "\n");

const latestArgs = [join(root, "scripts", "make-latest-json.mjs"), outDir, "--base", base];
if (notes) latestArgs.push("--notes", notes);
run(process.execPath, latestArgs);

console.log(`\n${basename(outDir)}: ${copied.join(", ")}, SHA256SUMS, latest.json`);
