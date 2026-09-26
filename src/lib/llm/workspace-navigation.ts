/** Legacy URLs remain stable; only the active destination exposes its tools. */
export const LLM_DESTINATIONS = [
  { id: 'chat', href: '/llm', routes: ['/llm'], tools: [] },
  { id: 'agents', href: '/llm/roster', routes: ['/llm/roster', '/llm/memory', '/llm/reading'], tools: [
    { href: '/llm/roster', key: 'llm.workspace.agents' }, { href: '/llm/memory', key: 'assist.memory.nav' }, { href: '/llm/reading', key: 'assist.reading.nav' },
  ] },
  { id: 'activity', href: '/llm/missions', routes: ['/llm/missions', '/llm/jobs', '/llm/loops', '/llm/observatory'], tools: [
    { href: '/llm/missions', key: 'mission.nav' }, { href: '/llm/jobs', key: 'llm.workspace.tasks' }, { href: '/llm/loops', key: 'llm.workspace.routines' }, { href: '/llm/observatory', key: 'llm.workspace.performance' },
  ] },
  { id: 'configure', href: '/llm/accounts', routes: ['/llm/accounts', '/llm/models', '/llm/mcp', '/llm/skills', '/llm/local', '/llm/arena'], tools: [
    { href: '/llm/accounts', key: 'llm.tab.accounts' }, { href: '/llm/models', key: 'llm.tab.models' }, { href: '/llm/mcp', key: 'llm.tab.mcp' }, { href: '/llm/skills', key: 'llm.tab.skills' }, { href: '/llm/local', key: 'llm.tab.local' }, { href: '/llm/arena', key: 'llm.tab.arena' },
  ] },
] as const;
export function llmDestination(path: string) {
  const normalized = path.replace(/\/$/, '') || '/llm';
  return LLM_DESTINATIONS.find(d => (d.routes as readonly string[]).includes(normalized)) ?? LLM_DESTINATIONS[0];
}
