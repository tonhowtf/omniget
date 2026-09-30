// Applies a node's role to the terminal it opens. The Rust side writes the
// role files and returns the per-agent recipe (commands::canvas::roles
// `canvas_roles_launch`, doc in core::canvas::roles):
//   claude   → project cwd + `--append-system-prompt <role>`
//   codex    → role folder cwd (AGENTS.md) + `--add-dir <project>`
//   gemini   → role folder cwd (AGENTS.md + .gemini/settings.json)
//   opencode → project cwd + OPENCODE_CONFIG with `instructions`
//   ollama   → the role text is sent as the first prompt
//   shell    → project cwd, OMNIGET_ROLE_* env only
//
// Integration (TerminalNode / spawn.ts, not owned here): before `termOpen`,
// `const { req, plan } = await withRole(req, node, workspaceId)`; after it,
// `afterOpen(plan, termId)`.

import type { CanvasNode, AgentKind } from '../model/types';
import type { TermOpen } from '../term/client';
import { termKey, termPaste } from '../term/client';
import { AGENT_COMMANDS } from '../spawn';
import { rolesLaunch, type RoleLaunch } from './client';

export function roleIdOf(node: CanvasNode): string | null {
  const r = node.data?.roleId;
  return typeof r === 'string' && r ? r : null;
}

function agentOf(node: CanvasNode, req: TermOpen): AgentKind {
  const a = (req.agent ?? node.data?.agent) as AgentKind | undefined;
  return a ?? 'shell';
}

/** Returns the request with the role applied (unchanged when the node has no role or the lookup fails). */
export async function withRole(
  req: TermOpen,
  node: CanvasNode,
  workspaceId: string,
): Promise<{ req: TermOpen; plan: RoleLaunch | null }> {
  const roleId = roleIdOf(node);
  if (!roleId || !workspaceId) return { req, plan: null };
  const agent = agentOf(node, req);
  let plan: RoleLaunch;
  try {
    plan = await rolesLaunch(workspaceId, roleId, agent, req.cwd ?? (node.data?.cwd as string | undefined) ?? null);
  } catch (e) {
    console.warn('[canvas.roles] launch plan failed', e);
    return { req, plan: null };
  }
  const base = req.command && req.command.length ? req.command : [...(AGENT_COMMANDS[agent] ?? [])];
  const command = base.length ? [...base, ...plan.args] : base;
  return {
    req: {
      ...req,
      cwd: plan.cwd ?? req.cwd,
      command,
      env: { ...(req.env ?? {}), ...plan.env },
    },
    plan,
  };
}

/** Sends the Ollama first prompt once the process had a moment to start. */
export function afterOpen(plan: RoleLaunch | null, termId: string, delayMs = 1500): void {
  const text = plan?.initialPrompt;
  if (!text) return;
  setTimeout(() => {
    void termPaste(termId, text)
      .then(() => new Promise((r) => setTimeout(r, 150)))
      .then(() => termKey(termId, { key: 'Enter', code: 'Enter' }))
      .catch(() => {});
  }, delayMs);
}
