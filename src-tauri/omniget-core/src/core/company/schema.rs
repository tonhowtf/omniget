use crate::core::assist::db::Migration;

pub const MODULE: &str = "company";

pub const MIGRATIONS: &[Migration] = &[Migration {
    module: MODULE,
    version: 1,
    sql: V1,
}];

const V1: &str = r#"
CREATE TABLE companies (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  mission TEXT NOT NULL DEFAULT '',
  description TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','paused','archived')),
  pause_reason TEXT,
  issue_prefix TEXT NOT NULL UNIQUE COLLATE NOCASE,
  issue_counter INTEGER NOT NULL DEFAULT 0,
  require_hire_approval INTEGER NOT NULL DEFAULT 1,
  require_strategy_approval INTEGER NOT NULL DEFAULT 0,
  strategy_state TEXT NOT NULL DEFAULT 'none' CHECK (strategy_state IN ('none','pending','approved')),
  workspace TEXT,
  max_parallel INTEGER NOT NULL DEFAULT 4,
  template TEXT,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL
);

CREATE TABLE staff (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  agent_id TEXT NOT NULL,
  name TEXT NOT NULL,
  role TEXT NOT NULL,
  title TEXT NOT NULL DEFAULT '',
  reports_to TEXT,
  capabilities TEXT NOT NULL DEFAULT '',
  instructions TEXT NOT NULL DEFAULT '',
  adapter TEXT NOT NULL DEFAULT '{"kind":"roster"}',
  status TEXT NOT NULL DEFAULT 'idle'
    CHECK (status IN ('pending_approval','idle','running','paused','error','terminated')),
  pause_reason TEXT,
  can_hire INTEGER NOT NULL DEFAULT 0,
  heartbeat_cron TEXT,
  heartbeat_tz TEXT NOT NULL DEFAULT 'local',
  heartbeat_next_ms INTEGER,
  max_concurrent INTEGER NOT NULL DEFAULT 1,
  workspace TEXT,
  last_heartbeat_ms INTEGER,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL,
  UNIQUE (company_id, id),
  UNIQUE (company_id, agent_id),
  FOREIGN KEY (company_id, reports_to) REFERENCES staff(company_id, id)
);
CREATE INDEX staff_tree ON staff(company_id, reports_to);

CREATE TABLE goals (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  level TEXT NOT NULL CHECK (level IN ('company','team','agent','task')),
  parent_id TEXT,
  owner_staff_id TEXT,
  title TEXT NOT NULL,
  body TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('planned','active','achieved','cancelled')),
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL,
  UNIQUE (company_id, id),
  FOREIGN KEY (company_id, parent_id) REFERENCES goals(company_id, id),
  FOREIGN KEY (company_id, owner_staff_id) REFERENCES staff(company_id, id)
);
CREATE UNIQUE INDEX goals_one_root ON goals(company_id) WHERE level = 'company' AND parent_id IS NULL;

CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  goal_id TEXT,
  lead_staff_id TEXT,
  name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'in_progress'
    CHECK (status IN ('backlog','planned','in_progress','completed','cancelled')),
  workspace TEXT,
  isolation TEXT NOT NULL DEFAULT 'shared' CHECK (isolation IN ('shared','worktree')),
  color TEXT,
  target_ms INTEGER,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL,
  UNIQUE (company_id, id),
  FOREIGN KEY (company_id, goal_id) REFERENCES goals(company_id, id),
  FOREIGN KEY (company_id, lead_staff_id) REFERENCES staff(company_id, id)
);

CREATE TABLE issues (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  number INTEGER NOT NULL,
  identifier TEXT NOT NULL,
  project_id TEXT,
  goal_id TEXT,
  parent_id TEXT,
  title TEXT NOT NULL,
  body TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'todo'
    CHECK (status IN ('backlog','todo','in_progress','in_review','blocked','done','cancelled')),
  seq INTEGER NOT NULL DEFAULT 0,
  priority TEXT NOT NULL DEFAULT 'medium' CHECK (priority IN ('critical','high','medium','low')),
  work_mode TEXT NOT NULL DEFAULT 'standard' CHECK (work_mode IN ('standard','ask','planning')),
  review_policy TEXT NOT NULL DEFAULT 'none'
    CHECK (review_policy IN ('none','anyone','not_creator','human_only')),
  reviewer_staff_id TEXT,
  assignee_staff_id TEXT,
  assignee_user INTEGER NOT NULL DEFAULT 0,
  created_by_staff_id TEXT,
  created_by_user INTEGER NOT NULL DEFAULT 0,
  lease_run_id TEXT,
  lease_until_ms INTEGER,
  blocked_reason TEXT,
  billing_code TEXT,
  request_depth INTEGER NOT NULL DEFAULT 0,
  board_rank REAL NOT NULL DEFAULT 0,
  origin_kind TEXT NOT NULL DEFAULT 'manual',
  origin_id TEXT,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL,
  started_ms INTEGER,
  completed_ms INTEGER,
  cancelled_ms INTEGER,
  UNIQUE (company_id, id),
  UNIQUE (company_id, number),
  CHECK (NOT (assignee_staff_id IS NOT NULL AND assignee_user = 1)),
  CHECK (status <> 'in_progress' OR assignee_staff_id IS NOT NULL OR assignee_user = 1),
  FOREIGN KEY (company_id, parent_id) REFERENCES issues(company_id, id),
  FOREIGN KEY (company_id, project_id) REFERENCES projects(company_id, id),
  FOREIGN KEY (company_id, goal_id) REFERENCES goals(company_id, id),
  FOREIGN KEY (company_id, assignee_staff_id) REFERENCES staff(company_id, id),
  FOREIGN KEY (company_id, reviewer_staff_id) REFERENCES staff(company_id, id)
);
CREATE INDEX issues_queue ON issues(company_id, assignee_staff_id, status);
CREATE INDEX issues_board ON issues(company_id, status, board_rank);
CREATE INDEX issues_parent ON issues(company_id, parent_id);
CREATE INDEX issues_project ON issues(company_id, project_id);

CREATE VIRTUAL TABLE issues_fts USING fts5(
  identifier, title, body, content='issues', content_rowid='rowid', tokenize='trigram'
);
CREATE TRIGGER issues_ai AFTER INSERT ON issues BEGIN
  INSERT INTO issues_fts(rowid, identifier, title, body) VALUES (new.rowid, new.identifier, new.title, new.body);
END;
CREATE TRIGGER issues_ad AFTER DELETE ON issues BEGIN
  INSERT INTO issues_fts(issues_fts, rowid, identifier, title, body)
    VALUES ('delete', old.rowid, old.identifier, old.title, old.body);
END;
CREATE TRIGGER issues_au AFTER UPDATE OF identifier, title, body ON issues BEGIN
  INSERT INTO issues_fts(issues_fts, rowid, identifier, title, body)
    VALUES ('delete', old.rowid, old.identifier, old.title, old.body);
  INSERT INTO issues_fts(rowid, identifier, title, body) VALUES (new.rowid, new.identifier, new.title, new.body);
END;

CREATE TABLE relations (
  company_id TEXT NOT NULL,
  from_id TEXT NOT NULL,
  to_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('blocks','related','duplicate')),
  created_ms INTEGER NOT NULL,
  PRIMARY KEY (company_id, from_id, to_id, kind),
  CHECK (from_id <> to_id),
  FOREIGN KEY (company_id, from_id) REFERENCES issues(company_id, id) ON DELETE CASCADE,
  FOREIGN KEY (company_id, to_id) REFERENCES issues(company_id, id) ON DELETE CASCADE
);
CREATE INDEX relations_to ON relations(company_id, to_id, kind);

CREATE TABLE comments (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL,
  issue_id TEXT NOT NULL,
  author_kind TEXT NOT NULL CHECK (author_kind IN ('user','agent','system')),
  author_staff_id TEXT,
  run_id TEXT,
  body TEXT NOT NULL,
  client_req TEXT,
  created_ms INTEGER NOT NULL,
  UNIQUE (issue_id, client_req),
  FOREIGN KEY (company_id, issue_id) REFERENCES issues(company_id, id) ON DELETE CASCADE
);
CREATE INDEX comments_issue ON comments(issue_id, created_ms);

CREATE TABLE documents (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL,
  issue_id TEXT NOT NULL,
  key TEXT NOT NULL,
  rev INTEGER NOT NULL,
  body TEXT NOT NULL,
  locked INTEGER NOT NULL DEFAULT 0,
  updated_ms INTEGER NOT NULL,
  UNIQUE (issue_id, key),
  FOREIGN KEY (company_id, issue_id) REFERENCES issues(company_id, id) ON DELETE CASCADE
);
CREATE TABLE document_revs (
  doc_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  rev INTEGER NOT NULL,
  body TEXT NOT NULL,
  summary TEXT,
  author TEXT NOT NULL,
  run_id TEXT,
  created_ms INTEGER NOT NULL,
  PRIMARY KEY (doc_id, rev)
);

CREATE TABLE work_products (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL,
  issue_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  title TEXT NOT NULL DEFAULT '',
  url TEXT,
  path TEXT,
  summary TEXT,
  run_id TEXT,
  review TEXT NOT NULL DEFAULT 'none' CHECK (review IN ('none','pending','accepted','rejected')),
  created_ms INTEGER NOT NULL,
  FOREIGN KEY (company_id, issue_id) REFERENCES issues(company_id, id) ON DELETE CASCADE
);

CREATE TABLE labels (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  color TEXT NOT NULL DEFAULT '#888888',
  UNIQUE (company_id, name COLLATE NOCASE)
);
CREATE TABLE issue_labels (
  issue_id TEXT NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
  label_id TEXT NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
  PRIMARY KEY (issue_id, label_id)
);

CREATE TABLE inbox_state (
  issue_id TEXT PRIMARY KEY REFERENCES issues(id) ON DELETE CASCADE,
  read_ms INTEGER,
  archived_ms INTEGER,
  snooze_until_ms INTEGER
);

-- Governance: one inbox for everything that needs the owner.
CREATE TABLE approvals (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('hire_agent','strategy','budget_override','board_request','decision','agent_change','plan')),
  title TEXT NOT NULL,
  body TEXT NOT NULL DEFAULT '',
  requested_by_staff_id TEXT,
  issue_id TEXT,
  run_id TEXT,
  subject_hash TEXT NOT NULL,
  payload TEXT NOT NULL DEFAULT '{}',
  options TEXT,
  chosen TEXT,
  state TEXT NOT NULL DEFAULT 'pending'
    CHECK (state IN ('pending','revision_requested','approved','rejected','cancelled','expired','applied','apply_failed')),
  note TEXT,
  decided_channel TEXT,
  decided_ms INTEGER,
  effect_error TEXT,
  expires_ms INTEGER,
  version INTEGER NOT NULL DEFAULT 1,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL
);
CREATE INDEX approvals_open ON approvals(company_id, state, kind);

CREATE TABLE budget_policies (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  scope_type TEXT NOT NULL CHECK (scope_type IN ('company','agent','project')),
  scope_id TEXT NOT NULL,
  metric TEXT NOT NULL CHECK (metric IN ('usd','tokens','runs')),
  window TEXT NOT NULL CHECK (window IN ('day','month','lifetime')),
  amount REAL NOT NULL,
  warn_pct INTEGER NOT NULL DEFAULT 80,
  hard_stop INTEGER NOT NULL DEFAULT 1,
  enabled INTEGER NOT NULL DEFAULT 1,
  created_ms INTEGER NOT NULL,
  UNIQUE (company_id, scope_type, scope_id, metric, window)
);

CREATE TABLE budget_incidents (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  policy_id TEXT NOT NULL REFERENCES budget_policies(id) ON DELETE CASCADE,
  window_key TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('soft','hard')),
  observed REAL NOT NULL,
  limit_value REAL NOT NULL,
  state TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open','resolved','dismissed')),
  approval_id TEXT,
  created_ms INTEGER NOT NULL,
  resolved_ms INTEGER,
  UNIQUE (policy_id, window_key, kind)
);

-- One row per model request, never per run (area C §5.1).
CREATE TABLE cost_events (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  staff_id TEXT,
  project_id TEXT,
  issue_id TEXT,
  run_id TEXT,
  billing_code TEXT,
  provider TEXT NOT NULL DEFAULT '',
  model TEXT NOT NULL DEFAULT '',
  billing_type TEXT NOT NULL DEFAULT 'unknown',
  cost_status TEXT NOT NULL DEFAULT 'reported' CHECK (cost_status IN ('reported','estimated','unpriced')),
  input_tokens INTEGER NOT NULL DEFAULT 0,
  cached_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  cost_micros INTEGER NOT NULL DEFAULT 0,
  occurred_ms INTEGER NOT NULL
);
CREATE INDEX cost_company_time ON cost_events(company_id, occurred_ms);
CREATE INDEX cost_staff_time ON cost_events(staff_id, occurred_ms);

-- Running totals updated in the same transaction as the event, so a budget
-- check is one keyed read, never a SUM over history.
CREATE TABLE spend_rollup (
  company_id TEXT NOT NULL,
  scope_key TEXT NOT NULL,
  window_key TEXT NOT NULL,
  cost_micros INTEGER NOT NULL DEFAULT 0,
  tokens INTEGER NOT NULL DEFAULT 0,
  runs INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (scope_key, window_key)
);

-- Heartbeats: every reason to wake an agent is a durable row.
CREATE TABLE wakeups (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  staff_id TEXT NOT NULL,
  issue_id TEXT,
  source TEXT NOT NULL,
  reason TEXT NOT NULL DEFAULT '',
  context TEXT NOT NULL DEFAULT '{}',
  priority INTEGER NOT NULL DEFAULT 50,
  state TEXT NOT NULL DEFAULT 'queued'
    CHECK (state IN ('queued','coalesced','dispatched','skipped','cancelled')),
  run_id TEXT,
  not_before_ms INTEGER NOT NULL DEFAULT 0,
  skip_reason TEXT,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL,
  FOREIGN KEY (company_id, staff_id) REFERENCES staff(company_id, id) ON DELETE CASCADE
);
CREATE INDEX wakeups_queue ON wakeups(state, not_before_ms, priority);
CREATE INDEX wakeups_staff ON wakeups(staff_id, state);

CREATE TABLE runs (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  staff_id TEXT NOT NULL,
  issue_id TEXT,
  source TEXT NOT NULL,
  wake_count INTEGER NOT NULL DEFAULT 1,
  job_id TEXT,
  state TEXT NOT NULL DEFAULT 'queued'
    CHECK (state IN ('queued','running','succeeded','failed','cancelled','timed_out')),
  failure_class TEXT,
  error TEXT,
  attempt INTEGER NOT NULL DEFAULT 1,
  prompt_chars INTEGER NOT NULL DEFAULT 0,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  cached_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  cost_micros INTEGER NOT NULL DEFAULT 0,
  summary TEXT,
  liveness TEXT,
  created_ms INTEGER NOT NULL,
  started_ms INTEGER,
  finished_ms INTEGER
);
CREATE INDEX runs_staff ON runs(staff_id, created_ms DESC);
CREATE INDEX runs_state ON runs(state);
CREATE INDEX runs_issue ON runs(issue_id, created_ms DESC);

CREATE TABLE routines (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  body TEXT NOT NULL DEFAULT '',
  staff_id TEXT NOT NULL,
  project_id TEXT,
  priority TEXT NOT NULL DEFAULT 'medium',
  cron TEXT NOT NULL,
  tz TEXT NOT NULL DEFAULT 'local',
  catch_up TEXT NOT NULL DEFAULT 'once' CHECK (catch_up IN ('skip','once','all_capped')),
  concurrency TEXT NOT NULL DEFAULT 'skip_if_active' CHECK (concurrency IN ('skip_if_active','always')),
  enabled INTEGER NOT NULL DEFAULT 1,
  next_run_ms INTEGER,
  last_run_ms INTEGER,
  last_issue_id TEXT,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL,
  FOREIGN KEY (company_id, staff_id) REFERENCES staff(company_id, id) ON DELETE CASCADE
);

CREATE TABLE secret_bindings (
  company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  staff_id TEXT NOT NULL,
  env_key TEXT NOT NULL,
  secret_name TEXT NOT NULL,
  PRIMARY KEY (company_id, staff_id, env_key)
);

-- Append-only, hash-chained. Deleting an agent never deletes its history.
CREATE TABLE audit (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  company_id TEXT NOT NULL,
  ts_ms INTEGER NOT NULL,
  actor_kind TEXT NOT NULL,
  actor_id TEXT NOT NULL DEFAULT '',
  channel TEXT NOT NULL DEFAULT 'app',
  action TEXT NOT NULL,
  entity_kind TEXT NOT NULL,
  entity_id TEXT NOT NULL DEFAULT '',
  run_id TEXT,
  details TEXT,
  prev_hash TEXT NOT NULL,
  hash TEXT NOT NULL
);
CREATE INDEX audit_company ON audit(company_id, seq DESC);
CREATE INDEX audit_entity ON audit(company_id, entity_kind, entity_id, seq);
"#;
