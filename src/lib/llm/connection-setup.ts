import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { TurnEventEnvelope, AgentDef } from './types';

/** Parse argv without a shell: quoting groups arguments; metacharacters stay literal. */
export function parseCommand(input: string): string[] {
  const args: string[] = []; let value = ''; let quote = ''; let escaped = false; let started = false;
  for (let index = 0; index < input.length; index++) {
    const c = input[index];
    if (escaped) { value += c; escaped = false; started = true; continue; }
    if (c === '\\' && quote !== "'" && (input[index + 1] === '"' || input[index + 1] === "'" || /\s/.test(input[index + 1] ?? ''))) { escaped = true; started = true; continue; }
    if (quote) { if (c === quote) quote = ''; else value += c; continue; }
    if (c === '"' || c === "'") { quote = c; started = true; continue; }
    if (/\s/.test(c)) { if (started) { args.push(value); value = ''; started = false; } }
    else { value += c; started = true; }
  }
  if (quote || escaped) throw new Error('Unclosed quote or trailing escape');
  if (started) args.push(value);
  return args;
}

/** A separate conversation, real runtime, one explicit short request, no invented success. */
export async function testConnection(agentId: string, signal: AbortSignal): Promise<void> {
  let requestId = ''; let finished = false; let said = false;
  const early: TurnEventEnvelope[] = [];
  let resolve!: () => void; let reject!: (e: Error) => void;
  const result = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
  // Attach rejection handling before invoking: cancellation may happen during IPC.
  void result.catch(() => {});
  const settle = (error?: string) => { if (finished) return; finished = true; error ? reject(new Error(error)) : resolve(); };
  const receive = (p: TurnEventEnvelope) => {
    if (!requestId) { early.push(p); return; }
    if (p.request_id !== requestId) return;
    if (p.event.type === 'text_delta' && p.event.text.trim()) said = true;
    if (p.event.type === 'error') settle(`${p.event.error.code}: ${p.event.error.message}`);
    if (p.event.type === 'finished') settle(p.event.reason === 'stop' && said ? undefined : `Test incomplete: ${p.event.reason}`);
  };
  const stop = await listen<TurnEventEnvelope>('llm://turn', e => receive(e.payload));
  const cancel = () => { if (requestId) void invoke('llm_turn_cancel', { requestId }).catch(() => {}); settle('Test cancelled'); };
  signal.addEventListener('abort', cancel);
  const timeout = setTimeout(cancel, 90_000);
  try {
    if (signal.aborted) throw new Error('Test cancelled');
    const started = invoke<{ request_id: string }>('llm_turn_start', {
      conversationId: `setup-${crypto.randomUUID()}`, agentId,
      input: 'Connection test. Reply only OK. Do not use tools or access files.',
    });
    // If cancellation wins while IPC is pending, cancel its eventual request too.
    void started.then(response => {
      if (finished && response?.request_id) void invoke('llm_turn_cancel', { requestId: response.request_id }).catch(() => {});
    }, () => {});
    const response = await Promise.race([started, result.then(() => { throw new Error('Test ended before start'); })]);
    requestId = response.request_id;
    if (!requestId) throw new Error('Missing request ID');
    if (signal.aborted || finished) cancel();
    for (const p of early) receive(p);
    await result;
  } finally { clearTimeout(timeout); stop(); signal.removeEventListener('abort', cancel); }
}

export interface ConnectionDraft {
  version: 1; mode: 'subscription' | 'api' | 'local'; step: number;
  cli: string; account: string; name: string; provider: string; models: string[];
  model: string; loginOpened: boolean; agentId: string; attempt: string; credentialId: string;
}
/** Whitelist fields: never restore secrets or a previous verification result. */
export function parseConnectionDraft(raw: unknown): ConnectionDraft | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  if (r.version !== 1 || !['subscription', 'api', 'local'].includes(String(r.mode)) ||
      !Number.isInteger(r.step) || Number(r.step) < 1 || Number(r.step) > 3 ||
      !['claude', 'codex'].includes(String(r.cli)) || typeof r.loginOpened !== 'boolean') return null;
  const strings = ['account', 'name', 'provider', 'model', 'agentId', 'attempt', 'credentialId'] as const;
  if (strings.some(k => typeof r[k] !== 'string' || (r[k] as string).length > 512)) return null;
  if (!/^[0-9a-f-]{36}$/.test(String(r.attempt))) return null;
  if (!Array.isArray(r.models) || r.models.length > 10000 || r.models.some(m => typeof m !== 'string' || m.length > 512)) return null;
  if (r.credentialId && !/^setup-[0-9a-f-]{36}$/.test(String(r.credentialId))) return null;
  if (r.agentId && !/^connection-[0-9a-f-]{36}$/.test(String(r.agentId))) return null;
  if (Number(r.step) === 3 && r.agentId !== `connection-${r.attempt}`) return null;
  if (Number(r.step) >= 2) {
    if (r.mode === 'subscription' && !r.account) return null;
    if (r.mode === 'api' && !r.credentialId) return null;
    if (r.mode !== 'subscription' && (!r.model || !r.models.includes(r.model))) return null;
  }
  return { version: 1, mode: r.mode as ConnectionDraft['mode'], step: Number(r.step), cli: String(r.cli),
    account: String(r.account), name: String(r.name), provider: String(r.provider), models: [...r.models] as string[],
    model: String(r.model), loginOpened: r.loginOpened, agentId: String(r.agentId), attempt: String(r.attempt), credentialId: String(r.credentialId) };
}


export const CONNECTION_TOKEN_LIMIT = 8192;
/** A resumed setup must not silently claim an externally edited agent as its own. */
export function connectionMatchesIntent(agent: AgentDef, expected: AgentDef): boolean {
  return agent.name === expected.name && JSON.stringify(agent.model) === JSON.stringify(expected.model)
    && JSON.stringify(agent.runtime) === JSON.stringify(expected.runtime)
    && (agent.tools?.length ?? 0) === 0 && (agent.skills?.length ?? 0) === 0
    && agent.budget?.tokens_per_turn === expected.budget?.tokens_per_turn
    && (agent.budget?.max_tool_calls_per_turn ?? 0) === (expected.budget?.max_tool_calls_per_turn ?? 0);
}

/** What went wrong with a connection, in terms a person can act on. */
export type ConnectionProblem =
  | 'missing_executable' | 'login_expired' | 'credential_invalid' | 'model_unavailable'
  | 'local_service_down' | 'rate_limited' | 'network' | 'cancelled' | 'no_answer' | 'unknown';

export interface ConnectionDiagnosis {
  problem: ConnectionProblem;
  /** i18n keys: what happened and what to do. */
  titleKey: string;
  actionKey: string;
  /** Offered buttons, in order. */
  actions: ('retry' | 'sign_in' | 'switch' | 'open_local' | 'install_cli' | 'change_model')[];
  /** The backend's message with anything that looks like a secret masked. */
  detail: string;
}

/** Masks API keys, bearer tokens and long opaque strings before an error is shown or kept. */
export function sanitizeError(text: string): string {
  return text
    .replace(/(bearer\s+)[A-Za-z0-9._~+/=-]{8,}/gi, '$1•••')
    .replace(/\b(sk|pk|rk|xai|gsk|key|tok|ghp|github_pat|AIza)[-_A-Za-z0-9]{12,}/g, '•••')
    .replace(/([?&](?:key|token|api_key|access_token)=)[^&\s]+/gi, '$1•••')
    .replace(/\b[A-Za-z0-9_-]{40,}\b/g, '•••')
    .slice(0, 600);
}

/**
 * Turns a failed connect/test into a concrete problem. `mode` is the wizard's
 * choice: a network error against a local model means the server is off.
 */
export function diagnoseConnectionError(raw: unknown, mode: 'subscription' | 'api' | 'local' | string): ConnectionDiagnosis {
  const text = String((raw as { message?: string })?.message ?? raw ?? '');
  const has = (...needles: string[]) => needles.some(n => text.includes(n));
  const lower = text.toLowerCase();
  let problem: ConnectionProblem = 'unknown';
  if (has('ERR_CLI_NOT_FOUND', 'ERR_CLI_SPAWN', 'ERR_LLM_LOCAL_BINARY') || /not found on (the )?path|no such file or directory|enoent/.test(lower)) problem = 'missing_executable';
  else if (has('ERR_CLI_AUTH', 'ERR_CLI_ACCOUNT') || /(not logged in|login (has )?expired|please (log|sign) ?in|session expired|oauth)/.test(lower)) problem = 'login_expired';
  else if (has('ERR_LLM_AUTH') || /\b(401|403)\b|invalid (api )?key|unauthori[sz]ed|forbidden/.test(lower)) problem = mode === 'subscription' ? 'login_expired' : 'credential_invalid';
  else if (has('ERR_LLM_MODEL', 'ERR_LLM_LOCAL_MODEL', 'ERR_LLM_NO_CANDIDATE') || /model .*(not found|does not exist|unavailable)|unknown model|no models/.test(lower)) problem = 'model_unavailable';
  else if (has('ERR_LLM_RATE', 'ERR_CLI_RATE') || /\b429\b|rate limit|quota/.test(lower)) problem = 'rate_limited';
  else if (has('ERR_LLM_NET') || /connection refused|econnrefused|timed? ?out|dns|network/.test(lower)) problem = mode === 'local' ? 'local_service_down' : 'network';
  else if (/test cancelled/i.test(text)) problem = 'cancelled';
  else if (/test incomplete|ended before start/i.test(text)) problem = 'no_answer';
  const actions: ConnectionDiagnosis['actions'] = {
    missing_executable: ['install_cli', 'retry', 'switch'],
    login_expired: ['sign_in', 'retry', 'switch'],
    credential_invalid: ['switch', 'retry'],
    model_unavailable: ['change_model', 'retry', 'switch'],
    local_service_down: ['open_local', 'retry', 'switch'],
    rate_limited: ['retry', 'switch'],
    network: ['retry', 'switch'],
    cancelled: ['retry'],
    no_answer: ['retry', 'change_model', 'switch'],
    unknown: ['retry', 'switch'],
  }[problem] as ConnectionDiagnosis['actions'];
  return {
    problem,
    titleKey: `assist.bots.connection.problem.${problem}`,
    actionKey: `assist.bots.connection.action.${problem}`,
    actions,
    detail: sanitizeError(text),
  };
}
