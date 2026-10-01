import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

const COMPANY_EVENT = 'company://changed';

export type Change = { company_id: string; entity: string; id: string };

export function rpc<T = unknown>(method: string, args: Record<string, unknown> = {}): Promise<T> {
  return invoke<T>('company_rpc', { method, args });
}

export function onCompanyChange(company: string | null, fn: (c: Change[]) => void): Promise<UnlistenFn> {
  let pending: Change[] = [];
  let scheduled = false;
  return listen<Change>(COMPANY_EVENT, (e) => {
    if (company && e.payload.company_id !== company) return;
    pending.push(e.payload);
    if (scheduled) return;
    scheduled = true;
    requestAnimationFrame(() => {
      const batch = pending;
      pending = [];
      scheduled = false;
      fn(batch);
    });
  });
}

export function errorText(e: unknown): string {
  const s = String(e);
  const i = s.indexOf(': ');
  return s.startsWith('ERR_') && i > 0 ? s.slice(i + 2) : s;
}

export type Company = {
  id: string; name: string; mission: string; description: string;
  status: 'active' | 'paused' | 'archived'; pause_reason: string | null;
  issue_prefix: string; issue_counter: number;
  require_hire_approval: boolean; require_strategy_approval: boolean;
  strategy_state: 'none' | 'pending' | 'approved'; workspace: string | null;
  max_parallel: number; template: string | null; created_ms: number; updated_ms: number;
};

export type Adapter =
  | { kind: 'roster' }
  | { kind: 'process'; command: string; args: string[]; timeout_sec?: number | null }
  | { kind: 'http'; url: string; timeout_sec?: number | null };

export type Staff = {
  id: string; company_id: string; agent_id: string; name: string; role: string; title: string;
  reports_to: string | null; capabilities: string; instructions: string; adapter: Adapter;
  status: 'pending_approval' | 'idle' | 'running' | 'paused' | 'error' | 'terminated';
  pause_reason: string | null; can_hire: boolean; heartbeat_cron: string | null; heartbeat_tz: string;
  heartbeat_next_ms: number | null; max_concurrent: number; workspace: string | null;
  last_heartbeat_ms: number | null; created_ms: number; updated_ms: number;
};

export type ChartNode = { staff: Staff; reports: ChartNode[] };

export type Goal = {
  id: string; company_id: string; level: 'company' | 'team' | 'agent' | 'task';
  parent_id: string | null; owner_staff_id: string | null; title: string; body: string;
  status: 'planned' | 'active' | 'achieved' | 'cancelled'; created_ms: number; updated_ms: number;
};

export type Project = {
  id: string; company_id: string; goal_id: string | null; lead_staff_id: string | null; name: string;
  description: string; status: 'backlog' | 'planned' | 'in_progress' | 'completed' | 'cancelled';
  workspace: string | null; isolation: 'shared' | 'worktree'; color: string | null;
  target_ms: number | null; created_ms: number; updated_ms: number;
};

export type IssueStatus = 'backlog' | 'todo' | 'in_progress' | 'in_review' | 'blocked' | 'done' | 'cancelled';
export type Priority = 'critical' | 'high' | 'medium' | 'low';

export type Issue = {
  id: string; company_id: string; number: number; identifier: string; project_id: string | null;
  goal_id: string | null; parent_id: string | null; title: string; body: string; status: IssueStatus;
  seq: number; priority: Priority; work_mode: 'standard' | 'ask' | 'planning';
  review_policy: 'none' | 'anyone' | 'not_creator' | 'human_only'; reviewer_staff_id: string | null;
  assignee_staff_id: string | null; assignee_user: boolean; created_by_staff_id: string | null;
  created_by_user: boolean; lease_run_id: string | null; lease_until_ms: number | null;
  blocked_reason: string | null; billing_code: string | null; request_depth: number; board_rank: number;
  origin_kind: string; origin_id: string | null; created_ms: number; updated_ms: number;
  started_ms: number | null; completed_ms: number | null; cancelled_ms: number | null;
  labels?: string[];
};

export type Comment = { id: string; issue_id: string; author_kind: 'user' | 'agent' | 'system'; author_staff_id: string | null; run_id: string | null; body: string; created_ms: number };
export type Document = { id: string; issue_id: string; key: string; rev: number; body: string; locked: boolean; updated_ms: number };
export type DocumentRev = { rev: number; body: string; summary: string | null; author: string; run_id: string | null; created_ms: number };
export type WorkProduct = { id: string; issue_id: string; kind: string; title: string; url: string | null; path: string | null; summary: string | null; run_id: string | null; review: 'none' | 'pending' | 'accepted' | 'rejected'; created_ms: number };
export type Label = { id: string; name: string; color: string };
export type IssueLink = { id: string; identifier: string; title: string; status: IssueStatus; kind: string };
export type IssueDetail = {
  issue: Issue; comments: Comment[]; documents: Document[]; work_products: WorkProduct[]; labels: Label[];
  blocked_by: IssueLink[]; blocking: IssueLink[]; related: IssueLink[]; children: IssueLink[]; assignee: Staff | null;
};

export const TRANSITIONS: Record<IssueStatus, IssueStatus[]> = {
  backlog: ['todo', 'cancelled'],
  todo: ['in_progress', 'blocked', 'cancelled', 'backlog'],
  in_progress: ['in_review', 'blocked', 'done', 'cancelled', 'todo'],
  in_review: ['in_progress', 'done', 'cancelled', 'todo'],
  blocked: ['todo', 'in_progress', 'cancelled'],
  done: ['todo'],
  cancelled: ['todo'],
};

export type Approval = {
  id: string; company_id: string; kind: 'hire_agent' | 'strategy' | 'budget_override' | 'board_request' | 'decision' | 'agent_change' | 'plan';
  title: string; body: string; requested_by_staff_id: string | null; issue_id: string | null; run_id: string | null;
  subject_hash: string; payload: Record<string, unknown>; options: string[] | null; chosen: string | null;
  state: 'pending' | 'revision_requested' | 'approved' | 'rejected' | 'cancelled' | 'expired' | 'applied' | 'apply_failed';
  note: string | null; decided_channel: string | null; decided_ms: number | null; effect_error: string | null;
  expires_ms: number | null; version: number; created_ms: number; updated_ms: number;
};

export type BudgetPolicy = { id: string; company_id: string; scope_type: 'company' | 'agent' | 'project'; scope_id: string; metric: 'usd' | 'tokens' | 'runs'; window: 'day' | 'month' | 'lifetime'; amount: number; warn_pct: number; hard_stop: boolean; enabled: boolean };
export type Incident = { id: string; company_id: string; policy_id: string; window_key: string; kind: 'soft' | 'hard'; observed: number; limit_value: number; state: 'open' | 'resolved' | 'dismissed'; approval_id: string | null; created_ms: number; resolved_ms: number | null };
export type PolicyStatus = { policy: BudgetPolicy; window_key: string; used: number; pct: number; open_incident: Incident | null };
export type CostRow = { key: string | null; name: string | null; cost_usd: number; tokens: number; events: number };
export type CostSummary = { window_key: string; cost_usd: number; tokens: number; runs: number; by_agent: CostRow[]; by_project: CostRow[]; by_model: CostRow[]; unpriced_events: number };

export type Run = {
  id: string; company_id: string; staff_id: string; issue_id: string | null; source: string; wake_count: number;
  job_id: string | null; state: 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled' | 'timed_out';
  failure_class: string | null; error: string | null; attempt: number; prompt_chars: number;
  input_tokens: number; cached_tokens: number; output_tokens: number; cost_micros: number;
  summary: string | null; liveness: string | null; created_ms: number; started_ms: number | null; finished_ms: number | null;
};

export type Wake = { id: string; company_id: string; staff_id: string; issue_id: string | null; source: string; reason: string; context: unknown; priority: number; state: 'queued' | 'coalesced' | 'dispatched' | 'skipped' | 'cancelled'; run_id: string | null; not_before_ms: number; skip_reason: string | null; created_ms: number };

export type Routine = {
  id: string; company_id: string; title: string; body: string; staff_id: string; project_id: string | null;
  priority: Priority; cron: string; tz: string; catch_up: 'skip' | 'once' | 'all_capped';
  concurrency: 'skip_if_active' | 'always'; enabled: boolean; next_run_ms: number | null;
  last_run_ms: number | null; last_issue_id: string | null; created_ms: number; updated_ms: number;
};

export type AuditRow = { seq: number; company_id: string; ts_ms: number; actor_kind: string; actor_id: string; channel: string; action: string; entity_kind: string; entity_id: string; run_id: string | null; details: unknown; hash: string };

export type TemplateInfo = { id: string; name: string; summary: string; roles: string[] };
export type ImportPlan = { company: string; issue_prefix: string; staff: string[]; goals: number; projects: number; routines: number; budgets: number; issues: number; secrets_to_fill: string[]; warnings: string[]; plan_hash: string };

export type Dashboard = {
  company: Company; staff: Staff[]; issue_counts: Partial<Record<IssueStatus, number>>; recent_runs: Run[];
  open_approvals: number; approvals: Approval[]; costs: CostSummary; budgets: PolicyStatus[]; goals: Goal[];
  projects: Project[]; running: number;
};

export type RuntimeTemplate = { id: string; name: string; runtime: unknown; model: string };
