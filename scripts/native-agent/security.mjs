import {readFile,writeFile,readdir} from 'node:fs/promises';
import path from 'node:path';import assert from 'node:assert/strict';import {randomUUID} from 'node:crypto';
const root=path.resolve(process.argv[2]);if(!root.endsWith('/docs/native-agent-access/evidence'))throw Error('isolated evidence directory required');
const dir=path.join(root,'integration');const grant=JSON.parse(await readFile(path.join(dir,'credentials.json'),'utf8'));const {base,artifacts}=JSON.parse(await readFile(path.join(dir,'summary.json'),'utf8'));const settings=JSON.parse(await readFile(path.join(root,'profile/settings.json'),'utf8'));
const results=[];const pause=ms=>new Promise(r=>setTimeout(r,ms));
async function debug(body){const r=await fetch(base+'/v1/debug/eval',{method:'POST',headers:{Authorization:`Bearer ${settings.app_settings.bridge.token}`,'Content-Type':'text/plain'},body});const v=await r.json();if(v?.error)throw Error(v.error);return v;}
async function call(token,name,args={}){const r=await fetch(base+'/mcp',{method:'POST',headers:{Authorization:`Bearer ${token}`,'Content-Type':'application/json',Accept:'application/json, text/event-stream'},body:JSON.stringify({jsonrpc:'2.0',id:randomUUID(),method:'tools/call',params:{name:'omniget_app_'+name,arguments:args}})});return {status:r.status,body:await r.json().catch(()=>null)};}
function code(v){return v.body?.result?.structuredContent?.error?.code;}
function data(v){assert(!v.body?.error&&!v.body?.result?.isError,JSON.stringify(v));return v.body.result.structuredContent;}
function record(name,ok){assert(ok,name);results.push({test:name,passed:true});}
const second=await debug(`return await window.__TAURI_INTERNALS__.invoke('tool_mcp_client_create',{name:'Native negative tests',scopes:['app_observe','app_control','app_capture','transfer']});`);
try {
 record('absent token',(await call('','status')).status===401);
 record('unknown window',code(await call(grant.token,'inspect',{windowId:'no-such-window'}))==='WINDOW_NOT_FOUND');
 record('unexpected argument',(await call(grant.token,'status',{eval:'arbitrary'})).body.error?.code===-32602);
 record('invalid pagination',!!(await call(grant.token,'inspect',{limit:1000})).body.error);
 const req={screenId:'/downloads',requestId:randomUUID()};data(await call(grant.token,'navigate',req));await pause(200);
 record('idempotency conflict',code(await call(grant.token,'navigate',{...req,screenId:'/llm'}))==='IDEMPOTENCY_CONFLICT');
 record('second controller busy',code(await call(second.token,'navigate',{screenId:'/llm',requestId:randomUUID()}))==='SESSION_BUSY');
 const state=data(await call(grant.token,'inspect'));const button=state.nodes.find(n=>n.role==='button'&&!n.actions.length);assert(button);
 record('ungranted UI handler',code(await call(grant.token,'interact',{ref:button.ref,stateRevision:state.stateRevision,action:'click',requestId:randomUUID()}))==='TARGET_NOT_ACTIONABLE');
 record('forged ref',code(await call(grant.token,'interact',{ref:'fake',stateRevision:state.stateRevision,action:'click',requestId:randomUUID()}))==='STALE_TARGET');
 await debug("const b=document.createElement('button');b.id='forged-native-action';b.dataset.agentActions='click';b.textContent='Forged action';document.body.append(b);return true;");
 const forged=data(await call(grant.token,'inspect'));const fake=forged.nodes.find(n=>n.name==='Forged action');record('HTML attributes confer no authority',!!fake&&!fake.actions.length);
 record('forged action refused',code(await call(grant.token,'interact',{ref:fake.ref,stateRevision:forged.stateRevision,action:'click',requestId:randomUUID()}))==='TARGET_NOT_ACTIONABLE');
 await debug("document.getElementById('forged-native-action').remove();return true;");
 const a=data(await call(grant.token,'capture')).artifact;
 const own=await fetch(base+a.downloadPath,{headers:{Authorization:`Bearer ${grant.token}`,'If-Match':`"${a.digest}"`}});record('owner can retrieve before negative tests',own.status===200);await own.arrayBuffer();
 const foreign=await fetch(base+a.downloadPath,{headers:{Authorization:`Bearer ${second.token}`,'If-Match':`"${a.digest}"`}});record('artifact owner isolation',foreign.status!==200);
 const wrong=await fetch(base+a.downloadPath,{headers:{Authorization:`Bearer ${grant.token}`,'If-Match':'"wrong"'}});record('artifact digest guard',wrong.status!==200);
 await debug("const e=document.createElement('input');e.type='password';e.id='native-secret-test';document.body.append(e);return true;");
 record('secret blocks pixels',code(await call(grant.token,'capture'))==='PERMISSION_REQUIRED');
 await debug("document.getElementById('native-secret-test').remove();return true;");
 const rec=data(await call(second.token,'record',{action:'start',requestId:randomUUID()}));await pause(500);data(await call(second.token,'record',{action:'cancel',jobId:rec.jobId}));
 let status;for(let i=0;i<30;i++){status=data(await call(second.token,'record',{action:'status',jobId:rec.jobId}));if(status.state.status==='cancelled')break;await pause(300);}record('record cancellation',status.state.status==='cancelled');
 const cap=data(await call(second.token,'capture'));
 data(await call(second.token,'record',{action:'start',requestId:randomUUID()}));await pause(500);
 await debug(`return await window.__TAURI_INTERNALS__.invoke('tool_mcp_client_revoke',{id:${JSON.stringify(second.principal.id)}});`);
 record('revoked tools denied',(await call(second.token,'status')).status===401);
 const revoked=await fetch(base+cap.artifact.downloadPath,{headers:{Authorization:`Bearer ${second.token}`,'If-Match':`"${cap.artifact.digest}"`}});record('revoked artifact denied',revoked.status===401);
 await pause(1500);
 const files=await readdir(path.join(root,'profile/native-agent-artifacts',second.principal.id));record('revoked recording partial removed',!files.some(f=>f.endsWith('-frames')||f.endsWith('.mp4')));
 const before=data(await call(grant.token,'inspect'));
 await debug("setTimeout(()=>location.reload(),50);return true;");await pause(1500);
 const after=data(await call(grant.token,'inspect'));record('reload creates new revision',before.stateRevision!==after.stateRevision);
 record('pre-reload refs stale',code(await call(grant.token,'interact',{ref:button.ref,stateRevision:before.stateRevision,action:'click',requestId:randomUUID()}))==='STALE_TARGET');
 await debug("return await window.__TAURI_INTERNALS__.invoke('native_agent_register',{epoch:crypto.randomUUID()});");
 record('missing listener times out',code(await call(grant.token,'inspect'))==='TIMEOUT');
 await debug("setTimeout(()=>location.reload(),50);return true;");await pause(1500);data(await call(grant.token,'inspect'));
} finally {
 await debug(`return await window.__TAURI_INTERNALS__.invoke('tool_mcp_client_revoke',{id:${JSON.stringify(second.principal.id)}});`).catch(()=>{});
 await writeFile(path.join(dir,'security-results.json'),JSON.stringify(results,null,2));
}
console.log(JSON.stringify({passed:results.length}));
