// Native P0 harness. Requires a debug build in an explicitly isolated profile.
// Uses the app's existing authenticated test driver, never Chromium/mocked IPC.
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
const [profile, operation = 'inspect', output] = process.argv.slice(2);
if (!profile || !path.isAbsolute(profile)) throw new Error('Pass the absolute isolated OMNIGET_DATA_DIR');
if (!['inspect', 'capture'].includes(operation)) throw new Error('Expected inspect or capture');
const settings = JSON.parse(await readFile(path.join(profile, 'settings.json'), 'utf8'));
const { port, token } = settings.app_settings.bridge;
if (!Number.isInteger(port) || !token) throw new Error('Native app bridge not ready');
const script = operation === 'capture'
  ? "return await window.__TAURI_INTERNALS__.invoke('debug_native_snapshot');"
  : `return {
      source: 'native-tauri-webview', href: location.href, title: document.title,
      viewport: {width: innerWidth, height: innerHeight, dpr: devicePixelRatio},
      backend: await window.__TAURI_INTERNALS__.invoke('llm_conversation_get', {conversationId:'native-proof'}),
      scrollContainers: [...document.querySelectorAll('*')].filter(e=>e.clientHeight>0 && e.scrollHeight>e.clientHeight+2).slice(0,40).map(e=>({tag:e.tagName,className:e.className,top:e.scrollTop,height:e.clientHeight,scrollHeight:e.scrollHeight})),
      controls: [...document.querySelectorAll('button,a,input,textarea,select,[role]')].slice(0,200).map(e => ({
        tag: e.tagName, role:e.getAttribute('role'), name:e.getAttribute('aria-label') || e.textContent?.trim().slice(0,160),
        type:e.getAttribute('type'), disabled:e.disabled || false,
        bounds: (()=>{const r=e.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height}})()
      }))
    };`;
const response = await fetch(`http://127.0.0.1:${port}/v1/debug/eval`, {
  method:'POST', headers:{Authorization:`Bearer ${token}`, 'Content-Type':'text/plain'}, body:script,
  signal:AbortSignal.timeout(30000),
});
if (!response.ok) throw new Error(`Native driver HTTP ${response.status}`);
const result = await response.json();
if (result?.error) throw new Error(result.error);
if (operation === 'capture') {
  if (!output) throw new Error('Capture requires an output PNG path');
  const png=Buffer.from(result,'base64');
  if (!png.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10]))) throw new Error('Not a PNG');
  await mkdir(path.dirname(path.resolve(output)), {recursive:true});
  await writeFile(output,png);
  console.log(JSON.stringify({file:path.resolve(output),bytes:png.length,width:png.readUInt32BE(16),height:png.readUInt32BE(20)}));
} else {
  if (output) await writeFile(output,JSON.stringify(result,null,2));
  console.log(JSON.stringify({source:result.source,href:result.href,viewport:result.viewport,controls:result.controls.length,backendMessages:result.backend.messages.length}));
}
