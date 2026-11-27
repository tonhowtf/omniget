import {createHash} from 'node:crypto';
import {readdirSync, readFileSync, existsSync, statSync} from 'node:fs';
import path from 'node:path';
const ignored = new Set(['target','node_modules','.git','.svelte-kit','binaries']);
/** @param {string} root @param {string[]} entries */
export function digest(root, entries) {
  const hash=createHash('sha256');
  /** @param {string} rel */
  function walk(rel) {
    const full=path.join(root,rel);
    const configured=process.env.CARGO_TARGET_DIR;
    if(configured&&path.resolve(full)===path.resolve(root,'src-tauri',configured))return;
    if(!existsSync(full)) return;
    if(statSync(full).isDirectory()) {
      for(const name of readdirSync(full).sort()) if(!ignored.has(name)) walk(path.join(rel,name));
    } else { hash.update(rel); hash.update('\0'); hash.update(readFileSync(full)); }
  }
  for(const entry of entries) walk(entry);
  return hash.digest('hex');
}
export const frontendInputs=['src','static','vite.config.js','tsconfig.json','package.json','pnpm-lock.yaml','scripts/dev/vite-plugin.mjs','scripts/dev/inputs.mjs'];
export const backendInputs=['src-tauri','rust-toolchain.toml','browser-extension','src/lib/native-agent/catalog.json','src/lib/style/workspace-design.css','static/favicon.png','LICENSE'];
/** @param {string} root */
export function revisions(root) {
  const frontendRevision=digest(root,frontendInputs), backendRevision=digest(root,backendInputs);
  return {frontendRevision,backendRevision,requestedRevision:createHash('sha256').update(frontendRevision+backendRevision).digest('hex')};
}
