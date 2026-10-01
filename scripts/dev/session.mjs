#!/usr/bin/env node
import {spawn, spawnSync} from 'node:child_process';
import {readFileSync,writeFileSync,mkdirSync,existsSync,watch,openSync,closeSync,unlinkSync,appendFileSync,renameSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {randomUUID,createHash} from 'node:crypto';
import {revisions,backendInputs} from './inputs.mjs';
import {copyChanged,fileDigest} from './artifacts.mjs';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const argv=process.argv.slice(2), command=argv[0]??'start';
const option=(name,fallback)=>argv.includes(name)?argv[argv.indexOf(name)+1]:fallback;
const directory=path.resolve(option('--session',path.join(root,'docs/dev-experience/session')));
const stateFile=path.join(directory,'state.json');
const read=()=>JSON.parse(readFileSync(stateFile,'utf8'));
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const alive=pid=>{if(!Number.isInteger(pid)||pid<=0)return false;try{process.kill(pid,0);return true;}catch{return false;}};
function emit(v){console.log(JSON.stringify(v));}
async function rpc(state,name,args={}) {
 const credentials=JSON.parse(readFileSync(path.join(directory,'credentials.json'),'utf8'));
 const settings=JSON.parse(readFileSync(path.join(directory,'profile/settings.json'),'utf8'));
 const response=await fetch(`http://127.0.0.1:${settings.app_settings.bridge.port}/mcp`,{method:'POST',headers:{Authorization:`Bearer ${credentials.token}`,'Content-Type':'application/json','MCP-Protocol-Version':'2025-06-18',Accept:'application/json, text/event-stream'},body:JSON.stringify({jsonrpc:'2.0',id:randomUUID(),method:'tools/call',params:{name:'omniget_app_'+name,arguments:args}}),signal:AbortSignal.timeout(10000)});
 if(!response.ok)throw Error('MCP_HTTP_'+response.status);
 const value=await response.json();
 if(value.error||value.result?.isError)throw Error(JSON.stringify(value.error??value.result.structuredContent));
 return value.result;
}
async function verified(state, revision) {
 if(!alive(state.supervisorPid)||state.phase!=='ready'||state.requestedRevision!==revision)throw Error('REVISION_NOT_READY');
 const identity=(await rpc(state,'status')).structuredContent.devIdentity;
 if(identity?.pid!==state.appPid||identity?.bootId!==state.bootId||identity?.sessionId!==state.sessionId||identity?.backendBuildId!==state.backendBuildId||identity?.workerBuildId!==state.workerBuildId)throw Error('INSTANCE_MISMATCH');
 const view=(await rpc(state,'inspect')).structuredContent;
 if(view.frontendRevision!==state.frontendRevision||view.readiness!=='ready')throw Error('FRONTEND_NOT_READY: '+JSON.stringify({revision:view.frontendRevision,readiness:view.readiness,details:view.readinessDetails}));
 if(read().requestedRevision!==revision||revisions(root).requestedRevision!==revision)throw Error('SOURCES_CHANGED');
 return {identity,view};
}
try {
if(command==='revision')emit(revisions(root));
else if(command==='prepare') {const state=read();if(!alive(state.supervisorPid))throw Error('SESSION_NOT_RUNNING');writeFileSync(path.join(directory,'prepare-request.json'),JSON.stringify({revision:revisions(root).requestedRevision,id:randomUUID()}));emit({requested:true});}
else if(command==='status'||command==='doctor') {
 const state=existsSync(stateFile)?read():{phase:'stopped'};
 let observation={readiness:'unverified',verifiedRevision:null};
 if(command==='status'&&state.phase==='ready') {
  try {const evidence=await verified(state,state.requestedRevision);observation={readiness:evidence.view.readiness,verifiedRevision:state.requestedRevision,uiEpoch:evidence.view.uiEpoch,stateRevision:evidence.view.stateRevision};}
  catch(error){observation={...observation,diagnostic:error.message};}
 }
 emit({...state,...observation,supervisorAlive:alive(state.supervisorPid??-999999),current:revisions(root),...(command==='doctor'?{node:process.version,rust:spawnSync('rustc',['-vV'],{cwd:root,encoding:'utf8'}).stdout,cargoTargetDir:process.env.CARGO_TARGET_DIR??path.join(root,'src-tauri/target'),configuration:'host implicit; dev; app+worker; incremental unchanged'}:{})});
} else if(command==='wait'||command==='capture') {
 const revision=option('--revision',null);if(!revision)throw Error('--revision is required (use revision after saving files)');
 const deadline=Date.now()+Number(option('--timeout','120000'));let last;
 while(Date.now()<deadline) {
  try {
   const state=read();
   if(state.phase==='failed'&&state.requestedRevision===revision)throw Object.assign(Error(state.error),{fatal:true});
   const evidence=await verified(state,revision);
   if(command==='capture') {
    const captureStarted=performance.now();const capture=await rpc(state,'capture');const captureRpcMs=performance.now()-captureStarted;
    await verified(read(),revision);
    const image=capture.content.find(c=>c.type==='image');
    if(!image)throw Error('NATIVE_IMAGE_MISSING');
    const out=path.resolve(option('--out',path.join(directory,'capture.png')));mkdirSync(path.dirname(out),{recursive:true});const artifact=capture.structuredContent.artifact;
    const settings=JSON.parse(readFileSync(path.join(directory,'profile/settings.json'),'utf8'));
    const grant=JSON.parse(readFileSync(path.join(directory,'credentials.json'),'utf8'));
    if(!artifact.downloadPath.startsWith('/mcp/artifacts/'))throw Error('INVALID_ARTIFACT_PATH');
    const download=await fetch(`http://127.0.0.1:${settings.app_settings.bridge.port}${artifact.downloadPath}`,{headers:{Authorization:`Bearer ${grant.token}`,'If-Match':`"${artifact.digest}"`},signal:AbortSignal.timeout(10000)});
    if(!download.ok)throw Error('ARTIFACT_DOWNLOAD_FAILED');
    const pixels=Buffer.from(await download.arrayBuffer());
    if(createHash('sha256').update(pixels).digest('hex')!==artifact.digest)throw Error('ARTIFACT_DIGEST_MISMATCH');
    writeFileSync(out,pixels);
    const result={...state,...evidence,capture:capture.structuredContent,captureRpcMs,captureWithDownloadMs:performance.now()-captureStarted,file:out};writeFileSync(out+'.json',JSON.stringify(result,null,2));emit(result);
   }else emit({...state,...evidence});
   process.exit(0);
  }catch(e){if(e.fatal)throw e;last=e.message;}
  await delay(150);
 }
 throw Error('DEADLINE_EXCEEDED: '+last);
} else if(command==='stop') {
 const state=read();if(state.phase!=='stopped'&&alive(state.supervisorPid))process.kill(state.supervisorPid,'SIGTERM');
} else if(command==='start') await start();
else throw Error('Expected start|status|doctor|revision|wait|capture|stop');
} catch(error) {
 const message=error instanceof Error?error.message:String(error);
 console.error(JSON.stringify({ok:false,error:{code:/^[A-Z_]+(?=:|$)/.exec(message)?.[0]??'DEV_COMMAND_FAILED',message}}));
 process.exitCode=1;
}
async function start() {
 mkdirSync(directory,{recursive:true,mode:0o700});
 const lock=path.join(directory,'supervisor.lock');
 if(existsSync(lock)){const pid=Number(readFileSync(lock,'utf8'));if(alive(pid))throw Error('SESSION_BUSY');unlinkSync(lock);}
 const fd=openSync(lock,'wx',0o600);writeFileSync(fd,String(process.pid));closeSync(fd);
 const port=Number(option('--port','1430'));
 const sessionId=randomUUID(),profile=path.join(directory,'profile');mkdirSync(profile,{recursive:true,mode:0o700});
 const env={...process.env,OMNIGET_DATA_DIR:profile,OMNIGET_DEV_SESSION:sessionId,OMNIGET_DEV_URL:`http://localhost:${port}`,OMNIGET_TEST_DRIVER:'1',TAURI_CONFIG:JSON.stringify({build:{devUrl:`http://localhost:${port}`}})};
 let state={schemaVersion:1,sessionId,supervisorRevision:fileDigest(fileURLToPath(import.meta.url)),supervisorPid:process.pid,root,profile,phase:'starting',...revisions(root)};
 let app,vite,buildChild,stopping=false,busy=false,again=false,timer;let backendGeneration=0;const watchers=[];
 function save(extra={}){const previousPhase=state.phase;Object.assign(state,extra,{updatedAt:new Date().toISOString()});writeFileSync(stateFile+'.tmp',JSON.stringify(state,null,2),{mode:0o600});renameSync(stateFile+'.tmp',stateFile);appendFileSync(path.join(directory,'events.jsonl'),JSON.stringify(state)+'\n');if(previousPhase!==state.phase)emit({phase:state.phase,requestedRevision:state.requestedRevision,cargoMs:state.cargoMs,bootId:state.bootId,...(state.error?{error:state.error}:{})});}
 function log(stream,data){appendFileSync(path.join(directory,stream+'.log'),data);}
 function signalChild(child,signal){try{if(process.platform==='win32')child.kill(signal);else process.kill(-child.pid,signal);}catch(e){if(e.code!=='ESRCH')throw e;}}
 async function stopChild(child){if(!child||(child.exitCode!==null||child.signalCode!==null))return;
  const exited=new Promise(r=>child.once('exit',r));signalChild(child,'SIGTERM');await Promise.race([exited,delay(5000)]);
  if(child.exitCode===null&&child.signalCode===null){signalChild(child,'SIGKILL');await exited;}
 }
 async function stop(exitCode=0){if(stopping)return;stopping=true;clearTimeout(timer);watchers.forEach(w=>w.close());await stopChild(buildChild);await stopChild(app);await stopChild(vite);save({phase:exitCode===0?'stopped':'failed',stale:true});unlinkSync(lock);process.exit(exitCode);}
 process.on('SIGINT',()=>void stop());process.on('SIGTERM',()=>void stop());
 async function provision(){
  const settings=JSON.parse(readFileSync(path.join(profile,'settings.json'),'utf8'));const {port,token}=settings.app_settings.bridge;
  async function debug(script){const r=await fetch(`http://127.0.0.1:${port}/v1/debug/eval`,{method:'POST',headers:{Authorization:`Bearer ${token}`,'Content-Type':'text/plain'},body:script,signal:AbortSignal.timeout(3000)});if(!r.ok)throw Error('DEBUG_NOT_READY');const v=await r.json();if(v.error)throw Error(v.error);return v;}
  if(!existsSync(path.join(directory,'fixture.json'))){
   await debug("return await (await import('/src/lib/stores/settings-store.svelte.ts')).updateSettings({onboarding_completed:true,legal_acknowledged:true,appearance:{language:'en'}});");
   writeFileSync(path.join(directory,'fixture.json'),JSON.stringify({schemaVersion:1,scenario:'empty isolated profile',route:'/downloads'}),{mode:0o600});
  }
  if(!existsSync(path.join(directory,'credentials.json'))){
   await debug("return await window.__TAURI_INTERNALS__.invoke('tool_mcp_set_enabled',{enabled:true});");
   const grant=await debug("return await window.__TAURI_INTERNALS__.invoke('tool_mcp_client_create',{name:'Isolated development session',scopes:['app_observe','app_control','app_capture','transfer']});");
   writeFileSync(path.join(directory,'credentials.json'),JSON.stringify(grant),{mode:0o600});
  }
  const grant=JSON.parse(readFileSync(path.join(directory,'credentials.json'),'utf8'));
  writeFileSync(path.join(directory,'endpoint.json'),JSON.stringify({url:`http://127.0.0.1:${port}/mcp`,token:grant.token,sessionId,bootId:state.bootId}),{mode:0o600});
 }
 async function build() {
  if(busy){if(revisions(root).backendRevision!==state.backendRevision)again=true;return;}busy=true;
  try {
   do {
    again=false;const generation=backendGeneration;const requested=revisions(root);save({...requested,phase:'building',error:null,stale:true});const started=performance.now();
    const args=['build','-p','omniget','-p','omniget-cli','--bin','omniget','--bin','omniget-worker','--bin','omniget-mcp','--message-format=json'];
    buildChild=spawn('cargo',args,{cwd:path.join(root,'src-tauri'),env,detached:process.platform!=='win32',stdio:['ignore','pipe','pipe']});let stdout='',stderr='',lockStarted=null,lockWaitMs=0;
    buildChild.stdout.on('data',b=>{if(lockStarted!==null){lockWaitMs=performance.now()-lockStarted;lockStarted=null;}stdout+=b;log('cargo',b);});buildChild.stderr.on('data',b=>{if(String(b).includes('Blocking waiting for file lock'))lockStarted=performance.now();stderr+=b;log('cargo',b);});
    const compiling=buildChild;let timedOut=false;
    const buildTimer=setTimeout(()=>{timedOut=true;signalChild(compiling,'SIGKILL');},Number(option('--build-timeout','900000')));
    let code;try{code=await new Promise((resolve,reject)=>{compiling.on('error',reject);compiling.on('close',resolve);});}finally{clearTimeout(buildTimer);}
    if(timedOut)throw Error('BUILD_DEADLINE_EXCEEDED');
    const cargoMs=performance.now()-started;save({cargoMs,lockWaitMs});
    if(stopping)return;
    if(backendGeneration!==generation||revisions(root).backendRevision!==requested.backendRevision){again=true;continue;}
    if(code!==0){const diagnostics=stdout.split('\n').flatMap(line=>{try{const v=JSON.parse(line);return v.reason==='compiler-message'?[v.message.rendered??v.message.message]:[];}catch{return [];}}).join('\n');save({phase:'failed',error:(stderr+diagnostics).slice(-12000),cargoMs,stale:true});break;}
    const artifacts=stdout.split('\n').flatMap(line=>{try{const v=JSON.parse(line);return v.reason==='compiler-artifact'&&v.executable?[v]:[];}catch{return [];}});
    const binary=name=>{const v=artifacts.find(a=>a.target.name===name);if(!v)throw Error('MISSING_ARTIFACT: '+name);return v.executable;};
    const backendBuildId=fileDigest(binary('omniget')),workerBuildId=fileDigest(binary('omniget-worker'));
    if(app&&app.exitCode===null&&app.signalCode===null&&state.backendBuildId===backendBuildId&&state.workerBuildId===workerBuildId){
      save({...revisions(root),phase:'ready',stale:false,cargoMs,workerCopied:false,reused:true});continue;
    }
    const run=path.join(directory,'artifacts',backendBuildId+'-'+workerBuildId);mkdirSync(run,{recursive:true});
    copyChanged(binary('omniget'),path.join(run,'omniget'));copyChanged(binary('omniget-mcp'),path.join(directory,'omniget-mcp'));const worker=copyChanged(binary('omniget-worker'),path.join(run,'omniget-worker'));
    let route='/downloads';try{const view=(await rpc(state,'inspect')).structuredContent;if(!view.sensitive)route=view.route;}catch{}
    save({phase:'restarting',cargoMs,backendBuildId,workerBuildId,workerCopied:worker.copied,stale:true});await stopChild(app);
    const bootId=randomUUID();save({bootId});
    app=spawn(path.join(run,'omniget'),[],{cwd:root,env:{...env,OMNIGET_BACKEND_BUILD_ID:backendBuildId,OMNIGET_WORKER_BUILD_ID:workerBuildId,OMNIGET_BOOT_ID:bootId},detached:process.platform!=='win32',stdio:['ignore','pipe','pipe']});
    const launched=app;app.on('exit',code=>{if(!stopping&&app===launched&&state.phase==='ready')save({phase:'failed',stale:true,error:'APP_EXITED: '+code});});
    app.stdout.on('data',b=>log('app',b));app.stderr.on('data',b=>log('app',b));save({appPid:app.pid,appExecutable:path.join(run,'omniget'),workerExecutable:path.join(run,'omniget-worker')});
    const launch=performance.now(),deadline=Date.now()+90000;let ready=false,last;
    while(Date.now()<deadline&&!stopping){try{
      if(app.exitCode!==null)throw Object.assign(Error('APP_EXITED: '+app.exitCode),{fatal:true});
      await provision();const identity=(await rpc(state,'status')).structuredContent.devIdentity;
      if(identity?.bootId!==bootId||identity?.sessionId!==sessionId)throw Error('INSTANCE_MISMATCH');
      await rpc(state,'navigate',{screenId:route,requestId:randomUUID()});ready=true;break;
     }catch(e){last=e.message;if(e.fatal)break;}await delay(200);}
    if(!ready)throw Error('LAUNCH_FAILED: '+last);
    save({...revisions(root),phase:'ready',stale:false,launchMs:performance.now()-launch,configuration:{args,profile:'dev',target:'implicit host',flags:Object.fromEntries(['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTC_WRAPPER'].map(key=>[key,env[key]??null])),targetDir:env.CARGO_TARGET_DIR??path.join(root,'src-tauri/target')}});
   }while(again&&!stopping);
  }catch(e){save({phase:'failed',error:e.message,stale:true});}
  finally{busy=false;}
 }
 try {
  vite=spawn(process.execPath,[path.join(root,'node_modules/vite/bin/vite.js'),'--port',String(port),'--strictPort'],{cwd:root,env,detached:process.platform!=='win32',stdio:['ignore','pipe','pipe']});
  vite.stdout.on('data',b=>log('vite',b));vite.stderr.on('data',b=>log('vite',b));
  vite.on('exit',code=>{if(!stopping){save({phase:'failed',error:'VITE_EXITED: '+code});void stop(1);}});
  const changed=()=>{clearTimeout(timer);timer=setTimeout(()=>{if(stopping)return;try{const next=revisions(root);if(next.backendRevision!==state.backendRevision)void build();else if(next.requestedRevision!==state.requestedRevision)save(next);}catch(error){log('watcher',String(error)+'\n');changed();}},200);};
  for(const dir of ['src-tauri','src','static','browser-extension','scripts/dev'])watchers.push(watch(path.join(root,dir),{recursive:true},(_event,file)=>{if(file&&!/(^|\/)(target|binaries)(\/|$)/.test(file)){if(env.CARGO_TARGET_DIR){const target=path.resolve(root,'src-tauri',env.CARGO_TARGET_DIR);const eventPath=path.resolve(root,dir,String(file));if(eventPath===target||eventPath.startsWith(target+path.sep))return;}const rel=path.posix.join(dir,String(file).split(path.sep).join('/'));if(backendInputs.some(input=>rel===input||rel.startsWith(input+'/')))backendGeneration++;changed();}}));
  watchers.push(watch(root,(_event,file)=>{if(file&&/^(vite.config.js|svelte.config.js|tsconfig.json|package.json|pnpm-lock.yaml|rust-toolchain.toml|LICENSE)$/.test(file)){if(backendInputs.includes(file))backendGeneration++;changed();}}));
  watchers.push(watch(directory,(_event,file)=>{if(file==='prepare-request.json')void build();}));
  save({vitePid:vite.pid});emit({session:directory,supervisorPid:process.pid});await build();
 }catch(e){save({phase:'failed',error:e.message});await stop(1);}
}
