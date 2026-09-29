/**
 * Legacy URLs remain stable; only the active destination exposes its tools.
 * Roadmap 3.9: "Work" gathers everything an agent is doing for you (the
 * unified list first, then each kind's own page), and the pages visited
 * less often sit under "More" instead of the primary tabs.
 */
export const LLM_DESTINATIONS = [
  { id: 'threads', href: '/llm/threads', routes: ['/llm/threads'], tools: [] },
  { id: 'work', href: '/llm/work', routes: ['/llm/work', '/llm/jobs', '/llm/loops', '/llm/missions', '/llm/arena'], tools: [
    { href: '/llm/work', key: 'llm.work.nav' }, { href: '/llm/jobs', key: 'llm.workspace.tasks' }, { href: '/llm/loops', key: 'llm.workspace.loops' },
    { href: '/llm/missions', key: 'mission.nav' }, { href: '/llm/arena', key: 'llm.tab.arena' },
  ] },
  { id: 'chat', href: '/llm', routes: ['/llm'], tools: [] },
  { id: 'agents', href: '/llm/roster', routes: ['/llm/roster', '/llm/memory'], tools: [
    { href: '/llm/roster', key: 'llm.workspace.agents' }, { href: '/llm/memory', key: 'assist.memory.nav' },
  ] },
  { id: 'catalog', href: '/llm/catalog', routes: ['/llm/catalog', '/llm/installed', '/llm/catalog/wizard'], tools: [
    { href: '/llm/catalog', key: 'llm.workspace.catalog' }, { href: '/llm/installed', key: 'llm.workspace.installed' }, { href: '/llm/catalog/wizard', key: 'llm.workspace.wizard' },
  ] },
  { id: 'tools', href: '/llm/tools', routes: ['/llm/tools'], tools: [] },
  { id: 'configure', href: '/llm/accounts', routes: ['/llm/accounts', '/llm/models', '/llm/mcp', '/llm/skills'], tools: [
    { href: '/llm/accounts', key: 'llm.tab.accounts' }, { href: '/llm/models', key: 'llm.tab.models' }, { href: '/llm/mcp', key: 'llm.tab.mcp' }, { href: '/llm/skills', key: 'llm.tab.skills' },
  ] },
  { id: 'more', href: '/llm/observatory', routes: ['/llm/observatory', '/llm/usage', '/llm/sessions', '/llm/retro', '/llm/reading', '/llm/remote'], tools: [
    { href: '/llm/observatory', key: 'llm.workspace.performance' }, { href: '/llm/usage', key: 'llm.workspace.usage' }, { href: '/llm/sessions', key: 'llm.workspace.sessions' },
    { href: '/llm/retro', key: 'llm.workspace.retro' }, { href: '/llm/reading', key: 'assist.reading.nav' }, { href: '/llm/remote', key: 'llm.workspace.remote' },
  ] },
] as const;

/** Old addresses that now land elsewhere (the page itself redirects too). */
export const LLM_REDIRECTS: Record<string, string> = { '/llm/local': '/llm/models' };

export function llmDestination(path: string) {
  const raw = path.replace(/\/$/, '') || '/llm';
  const normalized = LLM_REDIRECTS[raw] ?? raw;
  const exact = LLM_DESTINATIONS.find(d => (d.routes as readonly string[]).includes(normalized));
  if (exact) return exact;
  const nested = LLM_DESTINATIONS.find(d => (d.routes as readonly string[]).some(r => r !== '/llm' && normalized.startsWith(r + '/')));
  return nested ?? LLM_DESTINATIONS[2];
}
