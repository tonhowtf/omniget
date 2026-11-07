<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/state';
  import { locale } from '$lib/i18n';
  import { helpStrings } from '$lib/help/strings';
  import { articles, searchDocs, readArticle } from '$lib/help/docs';
  let query=$state(''), collection=$state('all');
  let favorites=$state<string[]>([]), recent=$state<string[]>([]), storageReady=$state(false);
  function readSaved(key:string):string[]{try{const value=JSON.parse(localStorage.getItem(key)??'[]');return Array.isArray(value)?value.filter((id):id is string=>typeof id==='string'):[];}catch{return [];}}
  function save(key:string,value:string[]){try{localStorage.setItem(key,JSON.stringify(value));}catch{}}
  onMount(()=>{favorites=readSaved('help-favorites');recent=readSaved('help-recent');storageReady=true;});
  $effect(()=>{if(active&&storageReady){const next=[active.id,...readSaved('help-recent').filter(id=>id!==active.id)].slice(0,8);recent=next;save('help-recent',next);}});
  function favorite(id:string){const next=favorites.includes(id)?favorites.filter(x=>x!==id):[...favorites,id];favorites=next;save('help-favorites',next);}
  function highlight(text:string){const term=query.trim();if(!term)return [{text,match:false}];const at=text.toLowerCase().indexOf(term.toLowerCase());return at<0?[{text,match:false}]:[{text:text.slice(0,at),match:false},{text:text.slice(at,at+term.length),match:true},{text:text.slice(at+term.length),match:false}];}

  let s=$derived(helpStrings($locale));
  let active=$derived(readArticle(page.url.searchParams.get('article')??'',$locale));
  let results=$derived((query.trim()?searchDocs(query,$locale,20):articles($locale)).filter(a=>collection==='all'||(collection==='favorites'?favorites:recent).includes(a.id)));
</script>
<svelte:head><title>Help · OmniGet</title></svelte:head>
<div class="help-page">
  <header class="hero"><h1>{s.title}</h1><p>{s.intro}</p></header>
  <div class="help-grid"><main>
    {#if active}<a class="back" href="/help">‹ {s.back}</a><article id="guide"><h2>{active.title}</h2><p class="summary">{active.summary}</p><button class="favorite" onclick={()=>favorite(active.id)}>{favorites.includes(active.id)?s.unsave:s.save}</button>{#if !$locale.startsWith('pt')&&!$locale.startsWith('en')&&!$locale.startsWith('ru')}<p class="notice">{s.fallback}</p>{/if}<small>{s.version} {active.appVersion} · {active.locale.toUpperCase()}</small><h3>{s.before}</h3><p>{active.prerequisites}</p><h3>{s.steps}</h3><ol>{#each active.steps as step}<li>{step}</li>{/each}</ol><h3>{s.expected}</h3><p>{active.expected}</p><h3>{s.recovery}</h3><p>{active.recovery}</p><a class="action" href={active.action}>{s.open}</a></article>
    {:else}<label class="search"><span>{s.search}</span><input type="search" bind:value={query} placeholder={s.search}/></label><nav class="collections" aria-label={s.guides}><button class:active={collection==='all'} onclick={()=>collection='all'}>{s.all}</button><button class:active={collection==='favorites'} onclick={()=>collection='favorites'}>{s.favorites}</button><button class:active={collection==='recent'} onclick={()=>collection='recent'}>{s.recent}</button></nav><h2 class="section-title">{s.guides}</h2><div class="guides">{#each results as guide}<a class="guide" href={`/help?article=${guide.id}`}><h3>{#each highlight(guide.title) as part}{#if part.match}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}</h3><p>{#each highlight(guide.summary) as part}{#if part.match}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}</p></a>{:else}<p>{s.noResults}</p>{/each}</div>{/if}
  </main></div>
</div>
<style>
.collections{display:flex;gap:6px;flex-wrap:wrap;margin-bottom:24px}.collections button,.favorite{font:inherit;font-size:.85rem;padding:9px 13px;border:1px solid var(--separator);border-radius:8px;background:var(--surface);color:var(--text);cursor:pointer}.collections button.active{background:var(--accent);color:var(--on-accent)}.collections{margin-top:20px;margin-bottom:0}mark{background:color-mix(in srgb,var(--accent) 25%,transparent);color:inherit}
.help-page{overflow:auto;padding:clamp(20px,4vw,48px);color:var(--text);background:var(--bg);height:100%;width:100%}.hero{max-width:760px;margin-bottom:32px}.hero h1{font:600 clamp(28px,3vw,40px)/1.15 var(--font-display,var(--font-sans));letter-spacing:-.035em;margin:0 0 12px}.hero p{color:var(--text-muted);font-size:1rem}.help-grid{display:grid;grid-template-columns:minmax(0,1fr);gap:32px;align-items:start;max-width:1040px}main{min-width:0}.search{display:flex;flex-direction:column;gap:8px;font-size:.85rem}.search input{padding:13px 16px;border:1px solid var(--separator);border-radius:10px;background:var(--surface);color:var(--text);font:inherit;width:100%}.section-title{font-size:.85rem;font-weight:500;color:var(--text-muted);margin:26px 0 8px}.guides{display:grid;grid-template-columns:1fr 1fr;column-gap:24px}.guide{padding:18px 0;border-bottom:1px solid var(--separator);text-decoration:none;color:inherit}.guide:hover h3{color:var(--accent-hi,var(--accent))}.guide h3{font-size:.95rem;font-weight:600;margin:0 0 7px}.guide p{font-size:.8rem;line-height:1.5;color:var(--text-muted);margin:0}.back{font-size:.85rem;color:var(--accent-hi,var(--accent))}article h2{font-size:1.7rem;margin:22px 0 10px}article h3{font-size:.9rem;margin:26px 0 8px}article p,li{font-size:.95rem;line-height:1.7;max-width:65ch}li{padding-left:6px;margin:10px 0}small,.summary{color:var(--text-muted)}.action{display:inline-block;margin:20px 0;padding:12px 18px;background:var(--accent);color:var(--on-accent,#fff);border-radius:9px;text-decoration:none}:is(a,input):focus-visible{outline:2px solid var(--accent-hi,var(--accent));outline-offset:3px}@media(max-width:1050px){.help-grid{grid-template-columns:1fr}.guides{grid-template-columns:1fr 1fr}}@media(max-width:560px){.guides{grid-template-columns:1fr}.help-page{padding:20px}}
</style>
