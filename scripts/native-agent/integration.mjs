#!/usr/bin/env node
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash,randomUUID} from 'node:crypto';
const profile=path.resolve(process.argv[2]??'');
if(!profile.includes('/docs/native-agent-access/evidence/profile')) throw Error('Use the isolated proof profile');
const out=path.join(path.dirname(profile),'integration');await mkdir(out,{recursive:true});
const settings=JSON.parse(await readFile(path.join(profile,'settings.json'),'utf8'));
const {port,token}=settings.app_settings.bridge;const base=`http://127.0.0.1:${port}`;
async function debug(script){const r=await fetch(base+'/v1/debug/eval',{method:'POST',headers:{Authorization:`Bearer ${token}`,'Content-Type':'text/plain'},body:script});const v=await r.json();if(v.error)throw Error(v.error);return v;}
await debug("return await window.__TAURI_INTERNALS__.invoke('tool_mcp_set_enabled',{enabled:true});");
let grant;
try{grant=JSON.parse(await readFile(path.join(out,'credentials.json'),'utf8'));}catch{
 grant=await debug(`return await window.__TAURI_INTERNALS__.invoke('tool_mcp_client_create',{name:'Native integration isolated',scopes:['app_observe','app_control','app_capture','transfer']});`);
 await writeFile(path.join(out,'credentials.json'),JSON.stringify(grant),{mode:0o600});
}
await writeFile(path.join(out,'claude-mcp.json'),JSON.stringify({mcpServers:{omniget:{type:'http',url:base+'/mcp',headers:{Authorization:`Bearer ${grant.token}`}}}}),{mode:0o600});
const events=[];
async function rpc(method,params={},credential=grant.token){const r=await fetch(base+'/mcp',{method:'POST',headers:{Authorization:`Bearer ${credential}`,'Content-Type':'application/json',Accept:'application/json, text/event-stream','MCP-Protocol-Version':'2025-06-18'},body:JSON.stringify({jsonrpc:'2.0',id:randomUUID(),method,params})});if(!r.ok)throw Error(`HTTP ${r.status}`);const v=await r.json();return v;}
async function call(name,args={},expectError=false){const v=await rpc('tools/call',{name:'omniget_app_'+name,arguments:args});events.push({name,args,result:v.result?.structuredContent??v.error});await writeFile(path.join(out,'events.json'),JSON.stringify(events,null,2));if(!expectError)assert(!v.error&&!v.result?.isError,JSON.stringify(v));return v.result;}
const pause=ms=>new Promise(r=>setTimeout(r,ms));
async function nav(screenId){await call('navigate',{screenId,requestId:randomUUID()});await pause(1000);return (await call('inspect')).structuredContent;}
async function download(a,name){const r=await fetch(base+a.downloadPath,{headers:{Authorization:`Bearer ${grant.token}`,'If-Match':`"${a.digest}"`}});assert.equal(r.status,200,'artifact download');const b=Buffer.from(await r.arrayBuffer());assert.equal(createHash('sha256').update(b).digest('hex'),a.digest);await writeFile(path.join(out,name),b);return b;}
const init=await rpc('initialize',{protocolVersion:'2025-06-18',capabilities:{},clientInfo:{name:'native-integration',version:'1'}});assert(init.result);
assert.equal((await rpc('tools/list')).result.tools.filter(t=>t.name.startsWith('omniget_app_')).length,8);
await call('status');const catalog=(await call('catalog')).structuredContent;assert(catalog.screens.length>=31);
let state=await nav('/llm');assert.equal(state.route,'/llm');
const input=state.nodes.find(n=>n.actions.includes('fill'));assert(input,'fill target');
const fill={stateRevision:state.stateRevision,ref:input.ref,action:'fill',value:'Native MCP integration — never sent',requestId:randomUUID()};
const filled=(await call('interact',fill)).structuredContent;assert(filled.nodes.some(n=>n.value===fill.value));
assert.deepEqual((await call('interact',fill)).structuredContent,filled);
const stale=await call('interact',{...fill,requestId:randomUUID(),value:'stale'},true);assert(stale.isError);assert.equal(stale.structuredContent.error.code,'STALE_TARGET');
state=(await call('inspect')).structuredContent;
const rail=state.nodes.find(n=>n.region==='conversation-messages');assert(rail,'scroll target');
const scrolled=(await call('interact',{stateRevision:state.stateRevision,ref:rail.ref,action:'scroll',value:rail.scroll.top>0?-400:400,requestId:randomUUID()})).structuredContent;assert.notEqual(scrolled.nodes.find(n=>n.ref===rail.ref).scroll.top,rail.scroll.top,'real conversation scroll offset must change');
await pause(300);
const full=await call('capture');await download(full.structuredContent.artifact,'conversation-full.png');
const captured=await call('capture',{region:{x:200,y:80,width:600,height:500}});assert(captured.content.some(c=>c.type==='image'));
await download(captured.structuredContent.artifact,'conversation-crop.png');
await nav('/settings');const blocked=await call('capture',{},true);assert(blocked.isError);
await nav('/world');await pause(2000);
await call('inspect');await pause(100);
const worldState=(await call('inspect')).structuredContent;
const canvasTab=worldState.nodes.find(n=>n.role==='tab'&&n.actions.includes('click'));assert(canvasTab,'World canvas tab');
await call('interact',{stateRevision:worldState.stateRevision,ref:canvasTab.ref,action:'click',requestId:randomUUID()});await pause(500);
const world=await call('capture');await download(world.structuredContent.artifact,'world.png');
const record=(await call('record',{action:'start',requestId:randomUUID()})).structuredContent;
await pause(1600);await nav('/downloads');await pause(1200);await call('record',{action:'stop',jobId:record.jobId});
let recording;
for(let i=0;i<60;i++){recording=(await call('record',{action:'status',jobId:record.jobId})).structuredContent;if(['complete','failed','cancelled'].includes(recording.state.status))break;await pause(500);}
assert.equal(recording.state.status,'complete',JSON.stringify(recording));await download(recording.state.artifact,'native-sequence.mp4');
const pet=await call('navigate',{screenId:'/pet',requestId:randomUUID()});await pause(1000);const auxiliary=await call('capture',{windowId:'pet'});await download(auxiliary.structuredContent.artifact,'pet.png');
const bundle=(await call('export',{artifactIds:[full.structuredContent.artifact.artifactId,captured.structuredContent.artifact.artifactId,world.structuredContent.artifact.artifactId,auxiliary.structuredContent.artifact.artifactId,recording.state.artifact.artifactId],storyboard:'# Omniget native proof\n\n1. Conversation: exact original text, 2 seconds.\n2. World: live native canvas, 3 seconds.\n3. Downloads: native sequence.\n4. Companion: auxiliary window.\n\nUse preserved PNG and MP4 pixels. Do not regenerate UI text. Demo profile; no provider request was made.'})).structuredContent;
await download(bundle.artifact,'production-bundle.zip');
await writeFile(path.join(out,'summary.json'),JSON.stringify({status:'passed',base,principalId:grant.principal.id,artifacts:[captured.structuredContent,world.structuredContent,auxiliary.structuredContent,recording,bundle]},null,2));
console.log(JSON.stringify({status:'passed',directory:out,events:events.length}));
