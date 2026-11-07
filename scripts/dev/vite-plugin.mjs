import {readdirSync,readFileSync} from 'node:fs';
import path from 'node:path';
import {digest, frontendInputs} from './inputs.mjs';
const id='virtual:omniget-dev-revision', resolved='\0'+id;
/** @param {string} root @returns {import("vite").Plugin} */
export function devRevisionPlugin(root) {
  return {
    name:'omniget-dev-revision',
    resolveId(source) {if(source===id)return resolved;},
    load(source) {
      if(source!==resolved)return;
      const revision=process.env.OMNIGET_DEV_SESSION ? digest(root,frontendInputs) : null;
      return `export const revision=${JSON.stringify(revision)};
if (import.meta.hot) {
 const state=globalThis.__omnigetDev ??= {frontendRevision:revision};
 let pending=revision;
 const applied=()=>{state.frontendRevision=pending;};
 import.meta.hot.on('vite:afterUpdate',applied);
 import.meta.hot.accept(m=>{if(m)pending=m.revision;});
 import.meta.hot.dispose(()=>import.meta.hot.off('vite:afterUpdate',applied));
}`;
    },
    handleHotUpdate(ctx) {
      if(!process.env.OMNIGET_DEV_SESSION)return;
      const relative=path.relative(root,ctx.file).split(path.sep).join('/');
      if(!frontendInputs.some(entry=>relative===entry||relative.startsWith(entry+'/')))return;
      const mod=ctx.server.moduleGraph.getModuleById(resolved);
      if(mod){ctx.server.moduleGraph.invalidateModule(mod);return [...ctx.modules,mod];}
    }
  };
}

const INVOKE=/invoke<([^()]*?)>\(\s*["']([a-z0-9_]+)["']/g;
/** @param {string} type */
export function emptyValueFor(type) {
  const t=type.trim();
  if(t.startsWith('['))return undefined;
  if(/\[\]$/.test(t)||/^Array<.*>$/.test(t))return [];
  if(t==='void'||t==='null'||t==='undefined'||/\|\s*(null|undefined)\b/.test(t)||/^(null|undefined)\s*\|/.test(t))return null;
  if(t==='boolean')return false;
  if(t==='number')return 0;
  if(t==='string')return '';
  return undefined;
}
/** @param {string} dir @param {Record<string,unknown>} out */
function scanInvokes(dir,out) {
  for(const entry of readdirSync(dir,{withFileTypes:true})) {
    const file=path.join(dir,entry.name);
    if(entry.isDirectory()){scanInvokes(file,out);continue;}
    if(!/\.(ts|svelte)$/.test(entry.name))continue;
    for(const [,type,command] of readFileSync(file,'utf8').matchAll(INVOKE)) {
      const value=emptyValueFor(type);
      if(value!==undefined&&!(command in out))out[command]=value;
    }
  }
}
const previewId='virtual:omniget-preview-defaults', previewResolved='\0'+previewId;
/** @param {string} root @returns {import("vite").Plugin} */
export function previewDefaultsPlugin(root) {
  return {
    name:'omniget-preview-defaults',
    resolveId(source) {if(source===previewId)return previewResolved;},
    load(source) {
      if(source!==previewResolved)return;
      /** @type {Record<string,unknown>} */
      const out={};
      if(this.meta.watchMode)scanInvokes(path.join(root,'src'),out);
      return `export default ${JSON.stringify(out)};`;
    }
  };
}
