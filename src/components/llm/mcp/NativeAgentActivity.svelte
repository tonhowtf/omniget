<script lang="ts">
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { invoke } from '@tauri-apps/api/core';
  import { t } from '$lib/i18n';
  let activity = $state<{id:string;name:string;windowId:string}|null>(null);
  let error = $state('');
  onMount(() => {
    let disposed=false;let stop: (()=>void)|undefined;
    listen<{id:string;name:string;windowId:string}>('native-agent-activity',({payload})=>{activity=payload;}).then(f=>{if(disposed)f();else stop=f;}).catch(() => {});
    return ()=>{disposed=true;stop?.();};
  });
  async function revoke() {
    if (!activity) return;
    try {await invoke('tool_mcp_client_revoke',{id:activity.id});activity=null;error='';}
    catch(e){error=String(e);}
  }
</script>
{#if activity}
  <aside aria-live="polite" class="native-agent-activity">
    <span>{$t('mcp_connections.native_active')}: {activity.name} · {activity.windowId}</span>
    <button type="button" onclick={revoke}>{$t('mcp_connections.native_revoke')}</button>
    {#if error}<span role="alert">{error}</span>{/if}
  </aside>
{/if}
<style>
  .native-agent-activity { position:fixed;bottom:8px;right:8px;z-index:70;display:flex;align-items:center;gap:8px;padding:6px 10px;border:1px solid var(--border-color);border-radius:8px;background:var(--bg-elevated);color:var(--text-primary);font-size:12px; }
  button { color:inherit;background:transparent;border:1px solid var(--border-color);border-radius:4px;padding:3px 6px;cursor:pointer; }
</style>
