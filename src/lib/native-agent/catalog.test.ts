import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import catalog from './catalog.json';
import { registerSemantic, readSemantics } from './semantic';
describe('native surface coverage', () => {
  it('classifies every application route exactly once', () => {
    const root=join(process.cwd(),'src/routes');
    function scan(dir:string): string[] { return readdirSync(dir,{withFileTypes:true}).flatMap(e => e.isDirectory() ? scan(join(dir,e.name)) : e.name==='+page.svelte' ? ['/'+relative(root,dir)] : []); }
    expect(catalog.map(s=>s.route).sort()).toEqual(scan(root).sort());
    expect(new Set(catalog.map(s=>s.screenId)).size).toBe(catalog.length);
    for (const route of ['/settings','/llm/accounts','/llm/mcp','/llm/models','/llm/roster','/about/debug']) expect(catalog.find(s=>s.route===route)?.sensitive).toBe(true);
  });
  it('classifies every native window constructor', () => {
    const root=join(process.cwd(),'src-tauri/src');
    function scan(dir:string):string[] {return readdirSync(dir,{withFileTypes:true}).flatMap(e=>e.isDirectory()?scan(join(dir,e.name)):e.name.endsWith('.rs')&&/WebviewWindowBuilder::(?:new|from_config)/.test(readFileSync(join(dir,e.name),'utf8'))?[relative(root,join(dir,e.name))]:[]);}
    expect(scan(root).sort()).toEqual(['commands/auth_webview.rs','commands/pet.rs','lib.rs','limits_strip/commands.rs','usage_tray/mod.rs']);
  });
  it('reads canvas state on demand and removes unmounted adapters', () => {
    let calls=0;const dispose=registerSemantic('test',()=>({tick:++calls}));
    expect(calls).toBe(0);expect(readSemantics().test).toEqual({tick:1});
    dispose();expect(readSemantics().test).toBeUndefined();expect(calls).toBe(1);
  });
});
