// Golden outputs for the Rust privacy.sexy compiler, made by the ORIGINAL
// TypeScript compiler. Run through `privacy_golden.sh`, which copies the
// privacy.sexy `src/` next to this file in a temporary folder (the `@/`
// imports resolve through its tsconfig) and installs `tsx` and `js-yaml`.
//
// Usage: tsx privacy_golden.ts <collection.yaml> <full.json> <golden.json> [count]
//   full.json   every script: { name: { code, revert } }
//   golden.json a deterministic sample of `count` scripts (default 30) that
//               covers every function called directly by a script first.
import { readFileSync, writeFileSync } from 'node:fs';
import { load as yamlLoad } from 'js-yaml';
import { createScriptCompiler } from '@/application/Application/Loader/Collections/Compiler/Executable/Script/Compiler/ScriptCompilerFactory';
import { ScriptLanguage } from '@/domain/ScriptMetadata/ScriptLanguage';

const [, , yamlPath, fullPath, goldenPath, countArg] = process.argv;
const count = Number(countArg ?? '30');
const data: any = yamlLoad(readFileSync(yamlPath, 'utf8'));
const language = data.scripting.language === 'batchfile' ? ScriptLanguage.batchfile : ScriptLanguage.shellscript;
const compiler = createScriptCompiler({ categoryContext: { functions: data.functions ?? [], language } });

const all: Record<string, { code: string; revert: string }> = {};
const firstCall: Record<string, string> = {};
function walk(node: any) {
  for (const c of node.children ?? []) {
    if (c.category) { walk(c); continue; }
    if (compiler.canCompile(c)) {
      const sc = compiler.compile(c);
      all[c.name] = { code: sc.execute, revert: sc.revert ?? '' };
      const calls = Array.isArray(c.call) ? c.call : [c.call];
      firstCall[c.name] = calls.map((x: any) => x.function).join('+');
    } else {
      all[c.name] = { code: c.code, revert: c.revertCode ?? '' };
      firstCall[c.name] = '<inline>';
    }
  }
}
for (const a of data.actions) walk(a);

// One script per distinct call shape first, then every n-th script.
const names = Object.keys(all);
const picked: string[] = [];
const seen = new Set<string>();
for (const n of names) {
  if (picked.length >= count) break;
  if (!seen.has(firstCall[n])) { seen.add(firstCall[n]); picked.push(n); }
}
const step = Math.max(1, Math.floor(names.length / count));
for (let i = 0; picked.length < count && i < names.length; i += step) {
  if (!picked.includes(names[i])) picked.push(names[i]);
}
const golden = Object.fromEntries(picked.map((n) => [n, all[n]]));
writeFileSync(fullPath, JSON.stringify(all, null, 1));
writeFileSync(goldenPath, `${JSON.stringify(golden, null, 1)}\n`);
console.log(`${yamlPath}: ${names.length} scripts, ${picked.length} in ${goldenPath}`);
