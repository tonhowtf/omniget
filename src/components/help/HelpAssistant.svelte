<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { testConnection } from '$lib/llm/connection-setup';
  import DownloadReceiptCard from './DownloadReceiptCard.svelte';
  import { parseHelpToolResult, mergeSources, type HelpSource, type DownloadReceipt } from '$lib/help/tool-results';
  import { locale } from '$lib/i18n';
  import { helpStrings } from '$lib/help/strings';
  import { resolveCitation } from '$lib/help/docs';
  import type { AgentDef, ToolAsk, TurnEventEnvelope } from '$lib/llm/types';
  let s=$derived(helpStrings($locale));
  let agents=$state<AgentDef[]>([]), selected=$state(''), draft=$state(''), error=$state(''), busy=$state(false), request=$state('');
  let session=$state(''), intent=$state(''), intentInput=$state('');
  let mounted=false, cancelWanted=false, submitted='', testController:AbortController|null=null;
  let pendingAsks:ToolAsk[]=[];
  let tested=$state(false), testing=$state(false);
  async function stop(){cancelWanted=true;if(request)try{await invoke('llm_turn_cancel',{requestId:request});}catch(e){if(mounted&&busy)error=String(e);}}
  function changeConnection(){plan=null;messages=[];session=`help-${crypto.randomUUID()}`;persist();}
  function modelLabel(agent:{model:AgentDef['model']}){return agent.model.policy==='fixed'?`${agent.model.model.provider} / ${agent.model.model.model}`:agent.model.chain.map(c=>c.model).join(' → ');}
  async function verify(){if(!saved||testing)return;testing=true;error='';testController=new AbortController();try{await testConnection(saved,testController.signal);tested=true;}catch(e){error=String(e);}finally{testing=false;testController=null;}}
  type Source=HelpSource;
  type Entry={role:string;text:string;sources:Source[];connection?:string;model?:string;tools?:string[];downloads?:number[];receipts?:Record<number,DownloadReceipt>};
  let messages=$state<Entry[]>([]), ask=$state<ToolAsk|null>(null);
  type ConnectionSummary={name:string;model:AgentDef['model'];connection:{runtime:string;cli?:string;accountId?:string;executable?:string}};
  const connectionLabel=(connection:ConnectionSummary['connection'])=>[connection.runtime,connection.cli,connection.accountId,connection.executable].filter(Boolean).join(' · ');
  let name=$state(''), plan=$state<{planId:string;revision:string;agent:AgentDef;diff?:{before?:ConnectionSummary;after?:ConnectionSummary;sourceName?:string;sourceAgentId?:string}}|null>(null), saved=$state('');
  let creating=$state(false), editing=$state(''), diagnostic=$state('');
  let source=$derived(agents.find(a=>a.id===selected));
  let connectionSelect=$state<HTMLSelectElement|undefined>();
  async function refreshRoster(){
    const preferred=selected;
    const next=await invoke<AgentDef[]>('llm_roster_list');
    agents=next;
    selected=preferred?(next.some(agent=>agent.id===preferred)?preferred:''):(next[0]?.id??'');
    await tick();
    if(connectionSelect)connectionSelect.value=selected;
    persist();
  }
  function persist(){try{localStorage.setItem('omniget-help-session',JSON.stringify({session,selected,draft,messages,intent,intentInput}));}catch{}}
  function consume({request_id,event}:TurnEventEnvelope){
    if(request_id!==request)return;
    const last=messages[messages.length-1];
    if(event.type==='text_delta' && last) last.text+=event.text;
    if(event.type==='tool_result' && last) {
      last.tools=[...(last.tools??[]),event.content];
      const result=parseHelpToolResult(event.content);last.sources=mergeSources(last.sources,result.sources);if(result.download){last.downloads=[...new Set([...(last.downloads??[]),result.download.id])];last.receipts={...(last.receipts??{}),[result.download.id]:result.download};}
    }
    if(event.type==='error'){error=event.error.message;busy=false;ask=null;}
    if(event.type==='finished'){busy=false;ask=null;if(event.reason==='stop'&&!error&&!cancelWanted&&draft===submitted){draft='';intent='';intentInput='';}persist();}
  }
  let buffer:TurnEventEnvelope[]=[];
  onMount(()=>{
    mounted=true;
    try{const data=JSON.parse(localStorage.getItem('omniget-help-session')??'null');if(data){session=data.session;selected=data.selected;draft=data.draft;intent=data.intent??'';intentInput=data.intentInput??'';messages=(data.messages??[]).map((message:Entry)=>({...message,sources:(message.sources??[]).filter((source)=>typeof source!=='string')}));}}catch{}
    session ||= `help-${crypto.randomUUID()}`;
    let destroyed=false;const stops:Array<()=>void>=[];
    void (async()=>{
      try{
        const stop=await listen<TurnEventEnvelope>('help://turn',({payload})=>{if(busy&&!request)buffer.push(payload);else consume(payload);});
        if(destroyed){stop();return;}stops.push(stop);
        const stopAsk=await listen<ToolAsk>('llm://tool-ask',({payload})=>{if(payload.request_id===request)ask=payload;else if(busy&&!request&&payload.agent===`help-${selected}`)pendingAsks.push(payload);});
        if(destroyed){stopAsk();return;}stops.push(stopAsk);
        await refreshRoster();
      }catch(e){error=String(e);}
    })();
    return()=>{destroyed=true;mounted=false;stops.forEach(f=>f());persist();testController?.abort();if(busy)void stop();};
  });
  async function send(){
    if(!draft.trim()||busy||!selected)return;
    if(!intent||intentInput!==draft){intent=crypto.randomUUID();intentInput=draft;}
    busy=true;request='';error='';buffer=[];pendingAsks=[];cancelWanted=false;submitted=draft;persist();
    if(messages.at(-1)?.role==='assistant'&&!messages.at(-1)?.text&&messages.at(-2)?.text===draft)messages=messages.slice(0,-2);
    messages.push({role:'user',text:draft,sources:[]},{role:'assistant',text:'',sources:[],connection:source?.name??selected,model:source?modelLabel(source):''});
    try{const result=await invoke<{request_id:string;sources:Array<{id:string;title:string;appVersion:string;locale:string;contentHash:string}>}>('help_turn_start',{conversationId:session,agentId:selected,input:draft,locale:$locale,intentId:intent});request=result.request_id;const reply=messages.at(-1);if(reply)reply.sources=mergeSources(reply.sources,result.sources.map(source=>({uri:`help://${source.id}#guide`,title:source.title,appVersion:source.appVersion,locale:source.locale,contentHash:source.contentHash})));if(cancelWanted||!mounted){await invoke('llm_turn_cancel',{requestId:request});busy=false;}else{buffer.forEach(consume);ask=busy?(pendingAsks.find(a=>a.request_id===request)??null):null;}buffer=[];pendingAsks=[];persist();}
    catch(e){error=String(e);busy=false;persist();}
  }
  async function answer(allow:boolean){if(!ask)return;try{await invoke('llm_tool_answer',{requestId:ask.request_id,toolCallId:ask.tool_call_id,allow,always:false});ask=null;}catch(e){error=String(e);}}
  async function preview(){creating=true;error='';try{plan=await invoke('help_tool_call',{name:'help_agent_plan',input:{sourceAgentId:selected,name,...(editing?{agentId:editing}:{})}});saved='';}catch(e){error=String(e);}finally{creating=false;}}
  async function apply(){if(!plan||creating)return;creating=true;error='';try{const result=await invoke<{agentId:string}>('help_tool_call',{name:'help_agent_apply',input:{planId:plan.planId,expectedRevision:plan.revision,idempotencyKey:plan.planId}});saved=result.agentId;tested=false;await refreshRoster();}catch(e){error=String(e);}finally{creating=false;}}
</script>
<section class="assistant">
  <header><h2>{s.assistant}</h2><button class="quiet" disabled={busy} onclick={()=>{messages=[];intent='';intentInput='';session=`help-${crypto.randomUUID()}`;persist();}}>{s.clear}</button></header>
  <p class="muted">{s.offline}</p>
  {#if agents.length}<label>{s.choose}<select bind:this={connectionSelect} bind:value={selected} disabled={busy||creating} onchange={changeConnection}>{#each agents as agent (agent.id)}<option value={agent.id}>{agent.name}</option>{/each}</select></label>{:else}<a class="action" href="/llm/accounts">{s.connect}</a>{/if}
  {#if source && source.runtime.kind!=='native'}<p class="muted">{s.toolSupport}</p>{/if}
  {#if selected}<button class="quiet" onclick={async()=>{try{diagnostic=JSON.stringify(await invoke('help_tool_call',{name:'help_connection_check',input:{connectionId:selected}}),null,2);}catch(e){error=String(e);}}}>{s.check}</button>{/if}
  {#if diagnostic}<details open><summary>{s.status}</summary><pre>{diagnostic}</pre></details>{/if}
  <div class="messages" aria-live="polite" aria-busy={busy}>
    {#each messages as message}
      <div class:human={message.role==='user'} class="message">
        <div class="body">{message.text}</div>
        {#each message.downloads??[] as id (id)}<DownloadReceiptCard {id} receipt={message.receipts?.[id]}/>{/each}
        {#if message.tools?.length}<details><summary>{s.status}</summary>{#each message.tools as result}<pre>{result}</pre>{/each}</details>{/if}
        {#if message.role==='assistant'&&message.text}<button class="quiet" onclick={async()=>{const {writeText}=await import('@tauri-apps/plugin-clipboard-manager');await writeText(message.text+'\n\n'+message.sources.map(source=>`${source.uri} (${source.appVersion}, ${source.locale}, ${source.contentHash})`).join('\n'));}}>{s.copyAnswer}</button>{/if}
        {#if message.model}<small>{s.configuredModel}: {message.connection} · {message.model}</small>{/if}
        {#if message.role==='assistant'&&message.sources.length}<small>{s.sources}</small><nav aria-label={s.sources}>{#each message.sources as source}{@const href=resolveCitation(source.uri,source.locale)}{#if href}<a href={href} title={`${s.version} ${source.appVersion} · ${source.locale} · ${source.contentHash}`}>{source.title}<small> {source.appVersion} · {source.locale}</small></a>{/if}{/each}</nav>{/if}
      </div>
    {/each}
  </div>
  {#if ask}<div class="permission" role="alert"><strong>{s.permission}: {ask.tool}</strong><pre>{ask.preview??''}</pre><button onclick={()=>answer(false)}>{s.deny}</button><button onclick={()=>answer(true)}>{s.allow}</button></div>{/if}
  {#if error}<p role="alert" class="error">{error}</p>{/if}
  <form onsubmit={(e)=>{e.preventDefault();void send();}}><label>{s.question}<textarea bind:value={draft} oninput={persist} rows="3" disabled={busy}></textarea></label><div class="send"><small>{s.draft}</small>{#if busy}<button type="button" onclick={stop}>{s.cancel}</button>{:else}<button class="primary" disabled={!selected||!draft.trim()}>{s.send}</button>{/if}</div></form>
  <details class="setup"><summary>{s.setup}</summary><p>{s.copy}</p><label>{s.edit}<select bind:value={editing} onchange={()=>{plan=null;saved='';name=agents.find(a=>a.id===editing)?.name??'';}}><option value="">{s.newAgent}</option>{#each agents as agent (agent.id)}<option value={agent.id}>{agent.name}</option>{/each}</select></label><label>{s.name}<input bind:value={name} oninput={()=>{plan=null;saved='';}} maxlength="120"/></label><button disabled={!name.trim()||!selected||creating} onclick={preview}>{s.plan}</button>
    {#if plan}<div class="review"><strong>{plan.agent.name}</strong>{#if plan.diff?.before}<p>{s.beforeChange}: {plan.diff.before.name} · {modelLabel(plan.diff.before)} · {connectionLabel(plan.diff.before.connection)}</p>{/if}<p>{s.afterChange}: {plan.agent.name} · {modelLabel(plan.agent)}{#if plan.diff?.after} · {connectionLabel(plan.diff.after.connection)}{/if}</p><p>{s.connection}: {plan.diff?.sourceName??source?.name} ({plan.diff?.sourceAgentId??selected})</p><p>{editing?s.keepPermissions:s.permissions}</p>{#if saved}<p role="status">{tested?s.ready:s.saved}</p><button disabled={testing} onclick={verify}>{s.test}</button><a href={`/llm?agent=${encodeURIComponent(saved)}`}>{s.chat}</a>{:else}<button class="primary" disabled={creating} onclick={apply}>{s.apply}</button>{/if}</div>{/if}
  </details>
</section>
<style>
  .assistant{padding:24px;border:1px solid var(--separator);border-radius:var(--radius-lg,16px);background:var(--surface);min-width:0}header{display:flex;align-items:center;justify-content:space-between;gap:12px}h2{font-size:1.2rem;margin:0}.muted,small{color:var(--text-muted)}label{display:flex;flex-direction:column;gap:7px;font-size:.85rem}select,input,textarea{font:inherit;color:var(--text);background:var(--bg);height:auto;min-height:40px;border:1px solid var(--separator);border-radius:8px;padding:10px;min-width:0;width:100%}textarea{resize:vertical}button,.action{border:1px solid var(--separator);border-radius:8px;padding:9px 13px;background:var(--surface);color:var(--text);font:inherit;font-size:.85rem;cursor:pointer}button:disabled{opacity:.5;cursor:default}.primary{background:var(--accent);color:var(--on-accent,#fff)}.quiet{border:0;background:transparent;font-size:.75rem}.send{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-top:10px}.messages{max-height:420px;overflow:auto;margin:16px 0}.message{padding:12px 0;border-bottom:1px solid var(--separator);font-size:.9rem;line-height:1.65}.human{font-weight:600}.body{white-space:pre-wrap;overflow-wrap:anywhere}nav{display:flex;gap:10px;flex-wrap:wrap;font-size:.75rem}a{color:var(--accent-hi,var(--accent))}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:180px;overflow:auto;font-size:.75rem}.permission,.review{padding:12px;background:var(--bg);border:1px solid var(--separator);border-radius:8px;margin:12px 0}.permission button{margin-right:8px}.error{color:var(--color-error,#c0392b);overflow-wrap:anywhere}.setup{border-top:1px solid var(--separator);margin-top:24px;padding-top:18px}.setup p{font-size:.85rem;color:var(--text-muted)}.setup button{margin-top:10px}summary{cursor:pointer}:is(button,a,input,select,textarea,summary):focus-visible{outline:2px solid var(--accent-hi,var(--accent));outline-offset:3px}
</style>
