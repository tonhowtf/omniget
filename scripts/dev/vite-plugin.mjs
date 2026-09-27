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
