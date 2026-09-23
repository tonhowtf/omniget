/** Legacy URLs remain stable; only the active destination exposes its tools. */
export const LLM_DESTINATIONS = [
  { id: 'threads', href: '/llm/threads', routes: ['/llm/threads'], tools: [] },
  { id: 'chat', href: '/llm', routes: ['/llm'], tools: [] },
  { id: 'agents', href: '/llm/roster', routes: ['/llm/roster'], tools: [] },
  { id: 'activity', href: '/llm/jobs', routes: ['/llm/jobs', '/llm/loops', '/llm/observatory', '/llm/usage', '/llm/sessions', '/llm/retro'], tools: [
    { href: '/llm/jobs', key: 'llm.workspace.tasks' }, { href: '/llm/loops', key: 'llm.workspace.routines' }, { href: '/llm/observatory', key: 'llm.workspace.performance' },
    { href: '/llm/usage', key: 'llm.workspace.usage' }, { href: '/llm/sessions', key: 'llm.workspace.sessions' }, { href: '/llm/retro', key: 'llm.workspace.retro' },
  ] },
  { id: 'catalog', href: '/llm/catalog', routes: ['/llm/catalog', '/llm/installed', '/llm/catalog/wizard'], tools: [
    { href: '/llm/catalog', key: 'llm.workspace.catalog' }, { href: '/llm/installed', key: 'llm.workspace.installed' }, { href: '/llm/catalog/wizard', key: 'llm.workspace.wizard' },
  ] },
  { id: 'tools', href: '/llm/tools', routes: ['/llm/tools'], tools: [] },
  { id: 'configure', href: '/llm/accounts', routes: ['/llm/accounts', '/llm/models', '/llm/mcp', '/llm/skills', '/llm/local', '/llm/arena', '/llm/remote'], tools: [
    { href: '/llm/accounts', key: 'llm.tab.accounts' }, { href: '/llm/models', key: 'llm.tab.models' }, { href: '/llm/mcp', key: 'llm.tab.mcp' }, { href: '/llm/skills', key: 'llm.tab.skills' }, { href: '/llm/local', key: 'llm.tab.local' }, { href: '/llm/arena', key: 'llm.tab.arena' }, { href: '/llm/remote', key: 'llm.workspace.remote' },
  ] },
] as const;
export function llmDestination(path: string) {
  const normalized = path.replace(/\/$/, '') || '/llm';
  const exact = LLM_DESTINATIONS.find(d => (d.routes as readonly string[]).includes(normalized));
  if (exact) return exact;
  const nested = LLM_DESTINATIONS.find(d => (d.routes as readonly string[]).some(r => r !== '/llm' && normalized.startsWith(r + '/')));
  return nested ?? LLM_DESTINATIONS[1];
}
