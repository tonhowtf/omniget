use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{
    audit, changed, conflict, forbidden, invalid, new_id, not_found, now_ms, sql, Actor,
    CompanyStore,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Company {
    pub id: String,
    pub name: String,
    pub mission: String,
    pub description: String,
    pub status: String,
    pub pause_reason: Option<String>,
    pub issue_prefix: String,
    pub issue_counter: i64,
    pub require_hire_approval: bool,
    pub require_strategy_approval: bool,
    pub strategy_state: String,
    pub workspace: Option<String>,
    pub max_parallel: i64,
    pub template: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewCompany {
    pub name: String,
    #[serde(default)]
    pub mission: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub issue_prefix: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub require_hire_approval: Option<bool>,
    #[serde(default)]
    pub require_strategy_approval: Option<bool>,
    #[serde(default)]
    pub max_parallel: Option<i64>,
    #[serde(default)]
    pub template: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompanyPatch {
    pub name: Option<String>,
    pub mission: Option<String>,
    pub description: Option<String>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub workspace: Option<Option<String>>,
    pub require_hire_approval: Option<bool>,
    pub require_strategy_approval: Option<bool>,
    pub max_parallel: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Adapter {
    #[default]
    Roster,
    Process {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        timeout_sec: Option<u64>,
    },
    Http {
        url: String,
        #[serde(default)]
        timeout_sec: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Staff {
    pub id: String,
    pub company_id: String,
    pub agent_id: String,
    pub name: String,
    pub role: String,
    pub title: String,
    pub reports_to: Option<String>,
    pub capabilities: String,
    pub instructions: String,
    pub adapter: Adapter,
    pub status: String,
    pub pause_reason: Option<String>,
    pub can_hire: bool,
    pub heartbeat_cron: Option<String>,
    pub heartbeat_tz: String,
    pub heartbeat_next_ms: Option<i64>,
    pub max_concurrent: i64,
    pub workspace: Option<String>,
    pub last_heartbeat_ms: Option<i64>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewStaff {
    pub agent_id: String,
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub reports_to: Option<String>,
    #[serde(default)]
    pub capabilities: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub adapter: Adapter,
    #[serde(default)]
    pub can_hire: bool,
    #[serde(default)]
    pub heartbeat_cron: Option<String>,
    #[serde(default)]
    pub max_concurrent: Option<i64>,
    #[serde(default)]
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StaffPatch {
    pub name: Option<String>,
    pub role: Option<String>,
    pub title: Option<String>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub reports_to: Option<Option<String>>,
    pub capabilities: Option<String>,
    pub instructions: Option<String>,
    pub adapter: Option<Adapter>,
    pub can_hire: Option<bool>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub heartbeat_cron: Option<Option<String>>,
    pub max_concurrent: Option<i64>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub workspace: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChartNode {
    pub staff: Staff,
    pub reports: Vec<ChartNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub company_id: String,
    pub level: String,
    pub parent_id: Option<String>,
    pub owner_staff_id: Option<String>,
    pub title: String,
    pub body: String,
    pub status: String,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewGoal {
    pub level: String,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub owner_staff_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoalPatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub status: Option<String>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub owner_staff_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub parent_id: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub company_id: String,
    pub goal_id: Option<String>,
    pub lead_staff_id: Option<String>,
    pub name: String,
    pub description: String,
    pub status: String,
    pub workspace: Option<String>,
    pub isolation: String,
    pub color: Option<String>,
    pub target_ms: Option<i64>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewProject {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub goal_id: Option<String>,
    #[serde(default)]
    pub lead_staff_id: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub isolation: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub target_ms: Option<i64>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub goal_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub lead_staff_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub workspace: Option<Option<String>>,
    pub isolation: Option<String>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub color: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::double_option")]
    pub target_ms: Option<Option<i64>>,
    pub status: Option<String>,
}

pub(crate) const COMPANY_COLS: &str =
    "id, name, mission, description, status, pause_reason, issue_prefix,
  issue_counter, require_hire_approval, require_strategy_approval, strategy_state, workspace,
  max_parallel, template, created_ms, updated_ms";

pub(crate) fn company_row(r: &Row) -> rusqlite::Result<Company> {
    Ok(Company {
        id: r.get(0)?,
        name: r.get(1)?,
        mission: r.get(2)?,
        description: r.get(3)?,
        status: r.get(4)?,
        pause_reason: r.get(5)?,
        issue_prefix: r.get(6)?,
        issue_counter: r.get(7)?,
        require_hire_approval: r.get::<_, i64>(8)? != 0,
        require_strategy_approval: r.get::<_, i64>(9)? != 0,
        strategy_state: r.get(10)?,
        workspace: r.get(11)?,
        max_parallel: r.get(12)?,
        template: r.get(13)?,
        created_ms: r.get(14)?,
        updated_ms: r.get(15)?,
    })
}

pub(crate) const STAFF_COLS: &str = "id, company_id, agent_id, name, role, title, reports_to,
  capabilities, instructions, adapter, status, pause_reason, can_hire, heartbeat_cron, heartbeat_tz,
  heartbeat_next_ms, max_concurrent, workspace, last_heartbeat_ms, created_ms, updated_ms";

pub(crate) fn staff_row(r: &Row) -> rusqlite::Result<Staff> {
    let adapter: String = r.get(9)?;
    Ok(Staff {
        id: r.get(0)?,
        company_id: r.get(1)?,
        agent_id: r.get(2)?,
        name: r.get(3)?,
        role: r.get(4)?,
        title: r.get(5)?,
        reports_to: r.get(6)?,
        capabilities: r.get(7)?,
        instructions: r.get(8)?,
        adapter: serde_json::from_str(&adapter).unwrap_or_default(),
        status: r.get(10)?,
        pause_reason: r.get(11)?,
        can_hire: r.get::<_, i64>(12)? != 0,
        heartbeat_cron: r.get(13)?,
        heartbeat_tz: r.get(14)?,
        heartbeat_next_ms: r.get(15)?,
        max_concurrent: r.get(16)?,
        workspace: r.get(17)?,
        last_heartbeat_ms: r.get(18)?,
        created_ms: r.get(19)?,
        updated_ms: r.get(20)?,
    })
}

const GOAL_COLS: &str =
    "id, company_id, level, parent_id, owner_staff_id, title, body, status, created_ms, updated_ms";

fn goal_row(r: &Row) -> rusqlite::Result<Goal> {
    Ok(Goal {
        id: r.get(0)?,
        company_id: r.get(1)?,
        level: r.get(2)?,
        parent_id: r.get(3)?,
        owner_staff_id: r.get(4)?,
        title: r.get(5)?,
        body: r.get(6)?,
        status: r.get(7)?,
        created_ms: r.get(8)?,
        updated_ms: r.get(9)?,
    })
}

const PROJECT_COLS: &str = "id, company_id, goal_id, lead_staff_id, name, description, status,
  workspace, isolation, color, target_ms, created_ms, updated_ms";

fn project_row(r: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        company_id: r.get(1)?,
        goal_id: r.get(2)?,
        lead_staff_id: r.get(3)?,
        name: r.get(4)?,
        description: r.get(5)?,
        status: r.get(6)?,
        workspace: r.get(7)?,
        isolation: r.get(8)?,
        color: r.get(9)?,
        target_ms: r.get(10)?,
        created_ms: r.get(11)?,
        updated_ms: r.get(12)?,
    })
}

pub(crate) fn get_company(tx: &rusqlite::Connection, id: &str) -> Result<Company, String> {
    tx.query_row(
        &format!("SELECT {COMPANY_COLS} FROM companies WHERE id = ?1"),
        params![id],
        company_row,
    )
    .optional()
    .map_err(sql)?
    .ok_or_else(|| not_found("company", id))
}

pub(crate) fn get_staff(
    tx: &rusqlite::Connection,
    company: &str,
    id: &str,
) -> Result<Staff, String> {
    tx.query_row(
        &format!("SELECT {STAFF_COLS} FROM staff WHERE company_id = ?1 AND id = ?2"),
        params![company, id],
        staff_row,
    )
    .optional()
    .map_err(sql)?
    .ok_or_else(|| not_found("staff", id))
}

fn level_rank(level: &str) -> Option<u8> {
    match level {
        "company" => Some(0),
        "team" => Some(1),
        "agent" => Some(2),
        "task" => Some(3),
        _ => None,
    }
}

pub fn derive_prefix(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            w.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect();
    let p: String = if words.len() >= 2 {
        words
            .iter()
            .take(4)
            .filter_map(|w| w.chars().next())
            .collect()
    } else {
        words
            .first()
            .map(|w| w.chars().take(3).collect())
            .unwrap_or_default()
    };
    let p = p.to_ascii_uppercase();
    if p.is_empty() {
        "CO".into()
    } else {
        p
    }
}

fn valid_prefix(p: &str) -> bool {
    (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric())
}

fn reports_cycle(
    tx: &rusqlite::Connection,
    company: &str,
    staff: &str,
    manager: &str,
) -> Result<bool, String> {
    if staff == manager {
        return Ok(true);
    }
    tx.query_row(
        "WITH RECURSIVE up(id, depth) AS (
           SELECT reports_to, 1 FROM staff WHERE company_id = ?1 AND id = ?2
           UNION
           SELECT s.reports_to, up.depth + 1 FROM staff s JOIN up ON s.id = up.id
           WHERE s.company_id = ?1 AND up.depth < 256
         )
         SELECT EXISTS (SELECT 1 FROM up WHERE id = ?3)",
        params![company, manager, staff],
        |r| r.get::<_, bool>(0),
    )
    .map_err(sql)
}

fn goal_cycle(
    tx: &rusqlite::Connection,
    company: &str,
    goal: &str,
    parent: &str,
) -> Result<bool, String> {
    if goal == parent {
        return Ok(true);
    }
    tx.query_row(
        "WITH RECURSIVE up(id, depth) AS (
           SELECT parent_id, 1 FROM goals WHERE company_id = ?1 AND id = ?2
           UNION
           SELECT g.parent_id, up.depth + 1 FROM goals g JOIN up ON g.id = up.id
           WHERE g.company_id = ?1 AND up.depth < 256
         )
         SELECT EXISTS (SELECT 1 FROM up WHERE id = ?3)",
        params![company, parent, goal],
        |r| r.get::<_, bool>(0),
    )
    .map_err(sql)
}

impl CompanyStore {
    pub fn company_create(&self, new: NewCompany, actor: &Actor) -> Result<Company, String> {
        let name = new.name.trim().to_string();
        if name.is_empty() {
            return Err(invalid("a company needs a name"));
        }
        let prefix = new
            .issue_prefix
            .map(|p| p.trim().to_ascii_uppercase())
            .unwrap_or_else(|| derive_prefix(&name));
        if !valid_prefix(&prefix) {
            return Err(invalid("the issue prefix is 1 to 8 letters or digits"));
        }
        self.write(|tx, ch| {
            let mut chosen = prefix.clone();
            let mut n = 1;
            while tx
                .query_row(
                    "SELECT 1 FROM companies WHERE issue_prefix = ?1",
                    params![chosen],
                    |_| Ok(()),
                )
                .optional()
                .map_err(sql)?
                .is_some()
            {
                n += 1;
                chosen = format!("{}{n}", &prefix[..prefix.len().min(7)]);
            }
            let id = new_id();
            let now = now_ms();
            tx.execute(
                "INSERT INTO companies (id, name, mission, description, issue_prefix, workspace,
                   require_hire_approval, require_strategy_approval, max_parallel, template,
                   created_ms, updated_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11)",
                params![
                    id,
                    name,
                    new.mission.trim(),
                    new.description,
                    chosen,
                    new.workspace,
                    new.require_hire_approval.unwrap_or(true) as i64,
                    new.require_strategy_approval.unwrap_or(false) as i64,
                    new.max_parallel.unwrap_or(4).clamp(1, 64),
                    new.template,
                    now
                ],
            )
            .map_err(sql)?;
            let mission = if new.mission.trim().is_empty() {
                name.clone()
            } else {
                new.mission.trim().to_string()
            };
            tx.execute(
                "INSERT INTO goals (id, company_id, level, title, status, created_ms, updated_ms)
                 VALUES (?1, ?2, 'company', ?3, 'active', ?4, ?4)",
                params![new_id(), id, mission, now],
            )
            .map_err(sql)?;
            audit::record(
                tx,
                &id,
                actor,
                "company.created",
                "company",
                &id,
                Some(json!({"name": name})),
            )?;
            changed(ch, &id, "company", &id);
            get_company(tx, &id)
        })
    }

    pub fn company_list(&self) -> Result<Vec<Company>, String> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {COMPANY_COLS} FROM companies ORDER BY status = 'archived', created_ms"
            ))?;
            let rows = st.query_map([], company_row)?;
            rows.collect()
        })
    }

    pub fn company_get(&self, id: &str) -> Result<Company, String> {
        self.db_read_company(id)
    }

    fn db_read_company(&self, id: &str) -> Result<Company, String> {
        self.read(|c| {
            c.query_row(
                &format!("SELECT {COMPANY_COLS} FROM companies WHERE id = ?1"),
                params![id],
                company_row,
            )
            .optional()
        })?
        .ok_or_else(|| not_found("company", id))
    }

    pub fn company_update(
        &self,
        id: &str,
        patch: CompanyPatch,
        actor: &Actor,
    ) -> Result<Company, String> {
        if !actor.is_user() {
            return Err(forbidden("only the owner changes company settings"));
        }
        self.write(|tx, ch| {
            let mut c = get_company(tx, id)?;
            if let Some(v) = patch.name {
                if v.trim().is_empty() {
                    return Err(invalid("a company needs a name"));
                }
                c.name = v.trim().into();
            }
            if let Some(v) = patch.mission {
                c.mission = v.trim().into();
                tx.execute(
                    "UPDATE goals SET title = ?2, updated_ms = ?3
                     WHERE company_id = ?1 AND level = 'company' AND parent_id IS NULL",
                    params![id, if c.mission.is_empty() { &c.name } else { &c.mission }, now_ms()],
                )
                .map_err(sql)?;
            }
            if let Some(v) = patch.description {
                c.description = v;
            }
            if let Some(v) = patch.workspace {
                c.workspace = v;
            }
            if let Some(v) = patch.require_hire_approval {
                c.require_hire_approval = v;
            }
            if let Some(v) = patch.require_strategy_approval {
                c.require_strategy_approval = v;
            }
            if let Some(v) = patch.max_parallel {
                c.max_parallel = v.clamp(1, 64);
            }
            tx.execute(
                "UPDATE companies SET name=?2, mission=?3, description=?4, workspace=?5,
                   require_hire_approval=?6, require_strategy_approval=?7, max_parallel=?8, updated_ms=?9
                 WHERE id = ?1",
                params![
                    id,
                    c.name,
                    c.mission,
                    c.description,
                    c.workspace,
                    c.require_hire_approval as i64,
                    c.require_strategy_approval as i64,
                    c.max_parallel,
                    now_ms()
                ],
            )
            .map_err(sql)?;
            audit::record(tx, id, actor, "company.updated", "company", id, None)?;
            changed(ch, id, "company", id);
            get_company(tx, id)
        })
    }

    pub fn company_set_status(
        &self,
        id: &str,
        status: &str,
        reason: Option<&str>,
        actor: &Actor,
    ) -> Result<Company, String> {
        if !matches!(status, "active" | "paused" | "archived") {
            return Err(invalid(format!("unknown company status `{status}`")));
        }
        self.write(|tx, ch| {
            let c = get_company(tx, id)?;
            if c.status == "archived" && status != "archived" && !actor.is_user() {
                return Err(forbidden("only the owner restores an archived company"));
            }
            tx.execute(
                "UPDATE companies SET status = ?2, pause_reason = ?3, updated_ms = ?4 WHERE id = ?1",
                params![id, status, if status == "paused" { reason } else { None }, now_ms()],
            )
            .map_err(sql)?;
            if status != "active" {
                tx.execute(
                    "UPDATE wakeups SET state = 'cancelled', skip_reason = 'company_paused', updated_ms = ?2
                     WHERE company_id = ?1 AND state = 'queued'",
                    params![id, now_ms()],
                )
                .map_err(sql)?;
            }
            audit::record(
                tx,
                id,
                actor,
                &format!("company.{status}"),
                "company",
                id,
                reason.map(|r| json!({"reason": r})),
            )?;
            changed(ch, id, "company", id);
            get_company(tx, id)
        })
    }

    pub fn company_delete(&self, id: &str, actor: &Actor) -> Result<(), String> {
        if !actor.is_user() {
            return Err(forbidden("only the owner deletes a company"));
        }
        self.write(|tx, ch| {
            get_company(tx, id)?;
            audit::record(tx, id, actor, "company.deleted", "company", id, None)?;
            tx.execute("DELETE FROM issue_labels WHERE issue_id IN (SELECT id FROM issues WHERE company_id = ?1)", params![id]).map_err(sql)?;
            tx.execute("DELETE FROM inbox_state WHERE issue_id IN (SELECT id FROM issues WHERE company_id = ?1)", params![id]).map_err(sql)?;
            tx.execute("DELETE FROM relations WHERE company_id = ?1", params![id]).map_err(sql)?;
            tx.execute("DELETE FROM comments WHERE company_id = ?1", params![id]).map_err(sql)?;
            tx.execute("DELETE FROM documents WHERE company_id = ?1", params![id]).map_err(sql)?;
            tx.execute("DELETE FROM work_products WHERE company_id = ?1", params![id]).map_err(sql)?;
            tx.execute("UPDATE issues SET parent_id = NULL WHERE company_id = ?1", params![id]).map_err(sql)?;
            tx.execute("DELETE FROM issues WHERE company_id = ?1", params![id]).map_err(sql)?;
            tx.execute("DELETE FROM projects WHERE company_id = ?1", params![id]).map_err(sql)?;
            tx.execute("UPDATE goals SET parent_id = NULL, owner_staff_id = NULL WHERE company_id = ?1", params![id]).map_err(sql)?;
            tx.execute("DELETE FROM goals WHERE company_id = ?1", params![id]).map_err(sql)?;
            tx.execute("DELETE FROM runs WHERE company_id = ?1", params![id]).map_err(sql)?;
            tx.execute("DELETE FROM spend_rollup WHERE company_id = ?1", params![id]).map_err(sql)?;
            tx.execute("UPDATE staff SET reports_to = NULL WHERE company_id = ?1", params![id]).map_err(sql)?;
            tx.execute("DELETE FROM companies WHERE id = ?1", params![id]).map_err(sql)?;
            changed(ch, id, "company", id);
            Ok(())
        })
    }

    pub fn staff_hire(&self, company: &str, new: NewStaff, actor: &Actor) -> Result<Staff, String> {
        if new.agent_id.trim().is_empty()
            || new.name.trim().is_empty()
            || new.role.trim().is_empty()
        {
            return Err(invalid("a hire needs an agent, a name and a role"));
        }
        if let Some(cron) = &new.heartbeat_cron {
            crate::core::company::routines::validate_cron(cron)?;
        }
        self.write(|tx, ch| {
            let c = get_company(tx, company)?;
            if c.status == "archived" {
                return Err(conflict("the company is archived"));
            }
            let mut pending = false;
            let mut reports_to = new.reports_to.clone();
            if let Some(hirer) = actor.staff_id() {
                let h = get_staff(tx, company, hirer)?;
                if !h.can_hire {
                    return Err(forbidden(format!("{} cannot hire", h.name)));
                }
                let manager = reports_to.get_or_insert_with(|| hirer.to_string()).clone();
                if manager != hirer && !reports_cycle(tx, company, &manager, hirer)? {
                    return Err(forbidden("an agent hires only into its own branch of the org chart"));
                }
                pending = c.require_hire_approval;
            }
            if let Some(m) = &reports_to {
                let mgr = get_staff(tx, company, m)?;
                if mgr.status == "terminated" {
                    return Err(invalid("the manager was terminated"));
                }
            }
            let id = new_id();
            let now = now_ms();
            let adapter = serde_json::to_string(&new.adapter).map_err(invalid)?;
            let status = if pending { "pending_approval" } else { "idle" };
            tx.execute(
                "INSERT INTO staff (id, company_id, agent_id, name, role, title, reports_to, capabilities,
                   instructions, adapter, status, can_hire, heartbeat_cron, max_concurrent, workspace,
                   created_ms, updated_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?16)",
                params![
                    id,
                    company,
                    new.agent_id.trim(),
                    new.name.trim(),
                    new.role.trim(),
                    new.title,
                    reports_to,
                    new.capabilities,
                    new.instructions,
                    adapter,
                    status,
                    new.can_hire as i64,
                    new.heartbeat_cron,
                    new.max_concurrent.unwrap_or(1).clamp(1, 20),
                    new.workspace,
                    now
                ],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(f, _)
                    if f.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    conflict("that agent already works for this company")
                }
                e => sql(e),
            })?;
            if pending {
                let payload = json!({"staff_id": id, "agent_id": new.agent_id, "name": new.name,
                    "role": new.role, "title": new.title, "reports_to": reports_to,
                    "capabilities": new.capabilities});
                super::gov::open_approval(
                    tx,
                    company,
                    super::gov::NewApproval {
                        kind: "hire_agent".into(),
                        title: format!("Hire {} as {}", new.name.trim(), new.role.trim()),
                        body: new.capabilities.clone(),
                        issue_id: None,
                        payload,
                        options: None,
                        expires_ms: None,
                    },
                    actor,
                    ch,
                )?;
            }
            audit::record(
                tx,
                company,
                actor,
                if pending { "staff.hire_requested" } else { "staff.hired" },
                "staff",
                &id,
                Some(json!({"name": new.name, "role": new.role, "agent_id": new.agent_id})),
            )?;
            super::routines::schedule_staff_heartbeat(tx, company, &id, now)?;
            changed(ch, company, "staff", &id);
            get_staff(tx, company, &id)
        })
    }

    pub fn staff_list(&self, company: &str) -> Result<Vec<Staff>, String> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {STAFF_COLS} FROM staff WHERE company_id = ?1 ORDER BY status = 'terminated', created_ms"
            ))?;
            let rows = st.query_map(params![company], staff_row)?;
            rows.collect()
        })
    }

    pub fn staff_get(&self, company: &str, id: &str) -> Result<Staff, String> {
        self.read(|c| {
            c.query_row(
                &format!("SELECT {STAFF_COLS} FROM staff WHERE company_id = ?1 AND id = ?2"),
                params![company, id],
                staff_row,
            )
            .optional()
        })?
        .ok_or_else(|| not_found("staff", id))
    }

    pub fn staff_by_agent(&self, agent_id: &str) -> Result<Vec<Staff>, String> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {STAFF_COLS} FROM staff WHERE agent_id = ?1 AND status <> 'terminated'"
            ))?;
            let rows = st.query_map(params![agent_id], staff_row)?;
            rows.collect()
        })
    }

    pub fn staff_find(&self, id: &str) -> Result<Staff, String> {
        self.read(|c| {
            c.query_row(
                &format!("SELECT {STAFF_COLS} FROM staff WHERE id = ?1"),
                params![id],
                staff_row,
            )
            .optional()
        })?
        .ok_or_else(|| not_found("staff", id))
    }

    pub fn staff_update(
        &self,
        company: &str,
        id: &str,
        patch: StaffPatch,
        actor: &Actor,
    ) -> Result<Staff, String> {
        if let Some(Some(cron)) = &patch.heartbeat_cron {
            crate::core::company::routines::validate_cron(cron)?;
        }
        self.write(|tx, ch| {
            let mut s = get_staff(tx, company, id)?;
            if s.status == "terminated" {
                return Err(conflict("a terminated agent cannot be changed"));
            }
            if let Some(me) = actor.staff_id() {
                let own_branch = me == id || reports_cycle(tx, company, id, me)?;
                if !own_branch {
                    return Err(forbidden("an agent changes only its own branch"));
                }
                if patch.can_hire.is_some() || patch.adapter.is_some() {
                    return Err(forbidden("permissions and runtime are changed by the owner"));
                }
            }
            if let Some(v) = patch.name {
                s.name = v;
            }
            if let Some(v) = patch.role {
                s.role = v;
            }
            if let Some(v) = patch.title {
                s.title = v;
            }
            if let Some(m) = patch.reports_to {
                if let Some(m) = &m {
                    get_staff(tx, company, m)?;
                    if reports_cycle(tx, company, id, m)? {
                        return Err(invalid("that would make a loop in the org chart"));
                    }
                }
                s.reports_to = m;
            }
            if let Some(v) = patch.capabilities {
                s.capabilities = v;
            }
            if let Some(v) = patch.instructions {
                s.instructions = v;
            }
            if let Some(v) = patch.adapter {
                s.adapter = v;
            }
            if let Some(v) = patch.can_hire {
                s.can_hire = v;
            }
            let cron_changed = patch.heartbeat_cron.is_some();
            if let Some(v) = patch.heartbeat_cron {
                s.heartbeat_cron = v;
            }
            if let Some(v) = patch.max_concurrent {
                s.max_concurrent = v.clamp(1, 20);
            }
            if let Some(v) = patch.workspace {
                s.workspace = v;
            }
            tx.execute(
                "UPDATE staff SET name=?3, role=?4, title=?5, reports_to=?6, capabilities=?7,
                   instructions=?8, adapter=?9, can_hire=?10, heartbeat_cron=?11, max_concurrent=?12,
                   workspace=?13, updated_ms=?14
                 WHERE company_id = ?1 AND id = ?2",
                params![
                    company,
                    id,
                    s.name,
                    s.role,
                    s.title,
                    s.reports_to,
                    s.capabilities,
                    s.instructions,
                    serde_json::to_string(&s.adapter).map_err(invalid)?,
                    s.can_hire as i64,
                    s.heartbeat_cron,
                    s.max_concurrent,
                    s.workspace,
                    now_ms()
                ],
            )
            .map_err(sql)?;
            if cron_changed {
                super::routines::schedule_staff_heartbeat(tx, company, id, now_ms())?;
            }
            audit::record(tx, company, actor, "staff.updated", "staff", id, None)?;
            changed(ch, company, "staff", id);
            get_staff(tx, company, id)
        })
    }

    pub fn staff_set_status(
        &self,
        company: &str,
        id: &str,
        action: &str,
        reason: Option<&str>,
        actor: &Actor,
    ) -> Result<Staff, String> {
        self.write(|tx, ch| {
            let s = get_staff(tx, company, id)?;
            if s.status == "terminated" {
                return Err(conflict("terminated is final"));
            }
            if let Some(me) = actor.staff_id() {
                if me == id || !reports_cycle(tx, company, id, me)? {
                    return Err(forbidden("an agent pauses only its reports"));
                }
                if action == "terminate" {
                    return Err(forbidden("only the owner terminates an agent"));
                }
            }
            let now = now_ms();
            match action {
                "pause" => {
                    tx.execute(
                        "UPDATE staff SET status='paused', pause_reason=?3, updated_ms=?4 WHERE company_id=?1 AND id=?2",
                        params![company, id, reason, now],
                    )
                    .map_err(sql)?;
                    tx.execute(
                        "UPDATE wakeups SET state='cancelled', skip_reason='agent_paused', updated_ms=?2
                         WHERE staff_id=?1 AND state='queued'",
                        params![id, now],
                    )
                    .map_err(sql)?;
                }
                "resume" => {
                    if s.status == "pending_approval" {
                        return Err(conflict("the hire is still waiting for approval"));
                    }
                    tx.execute(
                        "UPDATE staff SET status='idle', pause_reason=NULL, updated_ms=?3 WHERE company_id=?1 AND id=?2",
                        params![company, id, now],
                    )
                    .map_err(sql)?;
                }
                "terminate" => {
                    tx.execute(
                        "UPDATE staff SET status='terminated', pause_reason=?3, heartbeat_next_ms=NULL, updated_ms=?4
                         WHERE company_id=?1 AND id=?2",
                        params![company, id, reason, now],
                    )
                    .map_err(sql)?;
                    tx.execute(
                        "UPDATE staff SET reports_to = ?3, updated_ms = ?4 WHERE company_id = ?1 AND reports_to = ?2",
                        params![company, id, s.reports_to, now],
                    )
                    .map_err(sql)?;
                    let to_user = s.reports_to.is_none() as i64;
                    tx.execute(
                        "UPDATE issues SET assignee_staff_id = ?3, assignee_user = ?4,
                           status = CASE WHEN status = 'in_progress' THEN 'todo' ELSE status END,
                           lease_run_id = NULL, lease_until_ms = NULL, seq = seq + 1, updated_ms = ?5
                         WHERE company_id = ?1 AND assignee_staff_id = ?2 AND status NOT IN ('done','cancelled')",
                        params![company, id, s.reports_to, to_user, now],
                    )
                    .map_err(sql)?;
                    tx.execute(
                        "UPDATE wakeups SET state='cancelled', skip_reason='agent_terminated', updated_ms=?2
                         WHERE staff_id=?1 AND state='queued'",
                        params![id, now],
                    )
                    .map_err(sql)?;
                    tx.execute(
                        "UPDATE routines SET staff_id = COALESCE(?3, staff_id), enabled = CASE WHEN ?3 IS NULL THEN 0 ELSE enabled END
                         WHERE company_id = ?1 AND staff_id = ?2",
                        params![company, id, s.reports_to],
                    )
                    .map_err(sql)?;
                }
                _ => return Err(invalid(format!("unknown action `{action}`"))),
            }
            audit::record(
                tx,
                company,
                actor,
                &format!("staff.{action}"),
                "staff",
                id,
                reason.map(|r| json!({"reason": r})),
            )?;
            changed(ch, company, "staff", id);
            get_staff(tx, company, id)
        })
    }

    pub fn org_chart(&self, company: &str) -> Result<Vec<ChartNode>, String> {
        let all = self.staff_list(company)?;
        let live: Vec<Staff> = all
            .into_iter()
            .filter(|s| s.status != "terminated")
            .collect();
        fn build(parent: Option<&str>, all: &[Staff], depth: usize) -> Vec<ChartNode> {
            if depth > 64 {
                return Vec::new();
            }
            all.iter()
                .filter(|s| s.reports_to.as_deref() == parent)
                .map(|s| ChartNode {
                    staff: s.clone(),
                    reports: build(Some(&s.id), all, depth + 1),
                })
                .collect()
        }
        let ids: std::collections::HashSet<&str> = live.iter().map(|s| s.id.as_str()).collect();
        let mut roots: Vec<ChartNode> = live
            .iter()
            .filter(|s| s.reports_to.as_deref().is_none_or(|m| !ids.contains(m)))
            .map(|s| ChartNode {
                staff: s.clone(),
                reports: build(Some(&s.id), &live, 1),
            })
            .collect();
        roots.sort_by_key(|n| n.staff.created_ms);
        Ok(roots)
    }

    pub fn chain_of_command(&self, company: &str, staff: &str) -> Result<Vec<Staff>, String> {
        let mut out = Vec::new();
        let mut cur = self.staff_get(company, staff)?.reports_to;
        while let Some(id) = cur {
            if out.len() > 64 || out.iter().any(|s: &Staff| s.id == id) {
                break;
            }
            let s = self.staff_get(company, &id)?;
            cur = s.reports_to.clone();
            out.push(s);
        }
        Ok(out)
    }

    pub fn staff_reports(&self, company: &str, staff: &str) -> Result<Vec<Staff>, String> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {STAFF_COLS} FROM staff WHERE company_id = ?1 AND reports_to = ?2 AND status <> 'terminated'"
            ))?;
            let rows = st.query_map(params![company, staff], staff_row)?;
            rows.collect()
        })
    }

    pub fn goal_create(&self, company: &str, new: NewGoal, actor: &Actor) -> Result<Goal, String> {
        let rank = level_rank(&new.level)
            .ok_or_else(|| invalid("goal level is company, team, agent or task"))?;
        if new.title.trim().is_empty() {
            return Err(invalid("a goal needs a title"));
        }
        let status = new.status.clone().unwrap_or_else(|| "active".into());
        if !matches!(
            status.as_str(),
            "planned" | "active" | "achieved" | "cancelled"
        ) {
            return Err(invalid("unknown goal status"));
        }
        self.write(|tx, ch| {
            get_company(tx, company)?;
            let parent = match &new.parent_id {
                Some(p) => Some(p.clone()),
                None if rank == 0 => return Err(conflict("a company has one mission; edit it instead")),
                None => tx
                    .query_row(
                        "SELECT id FROM goals WHERE company_id = ?1 AND level = 'company' AND parent_id IS NULL",
                        params![company],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(sql)?,
            };
            if let Some(p) = &parent {
                let pl: String = tx
                    .query_row(
                        "SELECT level FROM goals WHERE company_id = ?1 AND id = ?2",
                        params![company, p],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(sql)?
                    .ok_or_else(|| not_found("goal", p))?;
                if level_rank(&pl).unwrap_or(0) > rank {
                    return Err(invalid(format!("a {} goal cannot sit under a {pl} goal", new.level)));
                }
            }
            if let Some(o) = &new.owner_staff_id {
                get_staff(tx, company, o)?;
            }
            let id = new_id();
            let now = now_ms();
            tx.execute(
                "INSERT INTO goals (id, company_id, level, parent_id, owner_staff_id, title, body, status, created_ms, updated_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
                params![id, company, new.level, parent, new.owner_staff_id, new.title.trim(), new.body, status, now],
            )
            .map_err(sql)?;
            audit::record(tx, company, actor, "goal.created", "goal", &id, Some(json!({"title": new.title})))?;
            changed(ch, company, "goal", &id);
            tx.query_row(&format!("SELECT {GOAL_COLS} FROM goals WHERE id = ?1"), params![id], goal_row)
                .map_err(sql)
        })
    }

    pub fn goal_list(&self, company: &str) -> Result<Vec<Goal>, String> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {GOAL_COLS} FROM goals WHERE company_id = ?1
                 ORDER BY CASE level WHEN 'company' THEN 0 WHEN 'team' THEN 1 WHEN 'agent' THEN 2 ELSE 3 END, created_ms"
            ))?;
            let rows = st.query_map(params![company], goal_row)?;
            rows.collect()
        })
    }

    pub fn goal_update(
        &self,
        company: &str,
        id: &str,
        patch: GoalPatch,
        actor: &Actor,
    ) -> Result<Goal, String> {
        self.write(|tx, ch| {
            let mut g = tx
                .query_row(
                    &format!("SELECT {GOAL_COLS} FROM goals WHERE company_id = ?1 AND id = ?2"),
                    params![company, id],
                    goal_row,
                )
                .optional()
                .map_err(sql)?
                .ok_or_else(|| not_found("goal", id))?;
            if let Some(v) = patch.title {
                g.title = v;
            }
            if let Some(v) = patch.body {
                g.body = v;
            }
            if let Some(v) = patch.status {
                if !matches!(v.as_str(), "planned" | "active" | "achieved" | "cancelled") {
                    return Err(invalid("unknown goal status"));
                }
                g.status = v;
            }
            if let Some(v) = patch.owner_staff_id {
                if let Some(o) = &v {
                    get_staff(tx, company, o)?;
                }
                g.owner_staff_id = v;
            }
            if let Some(p) = patch.parent_id {
                if g.level == "company" {
                    return Err(invalid("the mission has no parent"));
                }
                let Some(p) = p else {
                    return Err(invalid("a goal under the mission needs a parent"));
                };
                if goal_cycle(tx, company, id, &p)? {
                    return Err(invalid("that would make a loop in the goals"));
                }
                g.parent_id = Some(p);
            }
            tx.execute(
                "UPDATE goals SET title=?3, body=?4, status=?5, owner_staff_id=?6, parent_id=?7, updated_ms=?8
                 WHERE company_id=?1 AND id=?2",
                params![company, id, g.title, g.body, g.status, g.owner_staff_id, g.parent_id, now_ms()],
            )
            .map_err(sql)?;
            if g.level == "company" {
                tx.execute(
                    "UPDATE companies SET mission = ?2 WHERE id = ?1",
                    params![company, g.title],
                )
                .map_err(sql)?;
            }
            audit::record(tx, company, actor, "goal.updated", "goal", id, None)?;
            changed(ch, company, "goal", id);
            Ok(g)
        })
    }

    pub fn goal_delete(&self, company: &str, id: &str, actor: &Actor) -> Result<(), String> {
        self.write(|tx, ch| {
            let level: String = tx
                .query_row(
                    "SELECT level FROM goals WHERE company_id = ?1 AND id = ?2",
                    params![company, id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql)?
                .ok_or_else(|| not_found("goal", id))?;
            if level == "company" {
                return Err(conflict("the mission cannot be deleted"));
            }
            let kids: i64 = tx
                .query_row(
                    "SELECT (SELECT COUNT(*) FROM goals WHERE company_id=?1 AND parent_id=?2)
                          + (SELECT COUNT(*) FROM issues WHERE company_id=?1 AND goal_id=?2)
                          + (SELECT COUNT(*) FROM projects WHERE company_id=?1 AND goal_id=?2)",
                    params![company, id],
                    |r| r.get(0),
                )
                .map_err(sql)?;
            if kids > 0 {
                return Err(conflict(format!(
                    "{kids} goals, projects or issues still point here"
                )));
            }
            tx.execute("DELETE FROM goals WHERE id = ?1", params![id])
                .map_err(sql)?;
            audit::record(tx, company, actor, "goal.deleted", "goal", id, None)?;
            changed(ch, company, "goal", id);
            Ok(())
        })
    }

    pub fn project_create(
        &self,
        company: &str,
        new: NewProject,
        actor: &Actor,
    ) -> Result<Project, String> {
        if new.name.trim().is_empty() {
            return Err(invalid("a project needs a name"));
        }
        let isolation = new.isolation.clone().unwrap_or_else(|| "shared".into());
        if !matches!(isolation.as_str(), "shared" | "worktree") {
            return Err(invalid("isolation is shared or worktree"));
        }
        let status = new.status.clone().unwrap_or_else(|| "in_progress".into());
        self.write(|tx, ch| {
            get_company(tx, company)?;
            let id = new_id();
            let now = now_ms();
            tx.execute(
                "INSERT INTO projects (id, company_id, goal_id, lead_staff_id, name, description, status,
                   workspace, isolation, color, target_ms, created_ms, updated_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?12)",
                params![id, company, new.goal_id, new.lead_staff_id, new.name.trim(), new.description,
                    status, new.workspace, isolation, new.color, new.target_ms, now],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
                    invalid("the goal or the lead is not in this company, or the status is unknown")
                }
                e => sql(e),
            })?;
            audit::record(tx, company, actor, "project.created", "project", &id, Some(json!({"name": new.name})))?;
            changed(ch, company, "project", &id);
            tx.query_row(&format!("SELECT {PROJECT_COLS} FROM projects WHERE id = ?1"), params![id], project_row)
                .map_err(sql)
        })
    }

    pub fn project_list(&self, company: &str) -> Result<Vec<Project>, String> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {PROJECT_COLS} FROM projects WHERE company_id = ?1 ORDER BY status IN ('completed','cancelled'), created_ms"
            ))?;
            let rows = st.query_map(params![company], project_row)?;
            rows.collect()
        })
    }

    pub fn project_get(&self, company: &str, id: &str) -> Result<Project, String> {
        self.read(|c| {
            c.query_row(
                &format!("SELECT {PROJECT_COLS} FROM projects WHERE company_id = ?1 AND id = ?2"),
                params![company, id],
                project_row,
            )
            .optional()
        })?
        .ok_or_else(|| not_found("project", id))
    }

    pub fn project_update(
        &self,
        company: &str,
        id: &str,
        patch: ProjectPatch,
        actor: &Actor,
    ) -> Result<Project, String> {
        self.write(|tx, ch| {
            let mut p = tx
                .query_row(
                    &format!("SELECT {PROJECT_COLS} FROM projects WHERE company_id = ?1 AND id = ?2"),
                    params![company, id],
                    project_row,
                )
                .optional()
                .map_err(sql)?
                .ok_or_else(|| not_found("project", id))?;
            if let Some(v) = patch.name {
                p.name = v;
            }
            if let Some(v) = patch.description {
                p.description = v;
            }
            if let Some(v) = patch.goal_id {
                p.goal_id = v;
            }
            if let Some(v) = patch.lead_staff_id {
                p.lead_staff_id = v;
            }
            if let Some(v) = patch.workspace {
                p.workspace = v;
            }
            if let Some(v) = patch.isolation {
                if !matches!(v.as_str(), "shared" | "worktree") {
                    return Err(invalid("isolation is shared or worktree"));
                }
                p.isolation = v;
            }
            if let Some(v) = patch.color {
                p.color = v;
            }
            if let Some(v) = patch.target_ms {
                p.target_ms = v;
            }
            if let Some(v) = patch.status {
                p.status = v;
            }
            tx.execute(
                "UPDATE projects SET goal_id=?3, lead_staff_id=?4, name=?5, description=?6, status=?7,
                   workspace=?8, isolation=?9, color=?10, target_ms=?11, updated_ms=?12
                 WHERE company_id=?1 AND id=?2",
                params![company, id, p.goal_id, p.lead_staff_id, p.name, p.description, p.status,
                    p.workspace, p.isolation, p.color, p.target_ms, now_ms()],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
                    invalid("the goal or the lead is not in this company, or the status is unknown")
                }
                e => sql(e),
            })?;
            audit::record(tx, company, actor, "project.updated", "project", id, None)?;
            changed(ch, company, "project", id);
            Ok(p)
        })
    }

    pub fn project_delete(&self, company: &str, id: &str, actor: &Actor) -> Result<(), String> {
        self.write(|tx, ch| {
            tx.execute(
                "UPDATE issues SET project_id = NULL WHERE company_id = ?1 AND project_id = ?2",
                params![company, id],
            )
            .map_err(sql)?;
            tx.execute(
                "UPDATE routines SET project_id = NULL WHERE company_id = ?1 AND project_id = ?2",
                params![company, id],
            )
            .map_err(sql)?;
            let n = tx
                .execute(
                    "DELETE FROM projects WHERE company_id = ?1 AND id = ?2",
                    params![company, id],
                )
                .map_err(sql)?;
            if n == 0 {
                return Err(not_found("project", id));
            }
            audit::record(tx, company, actor, "project.deleted", "project", id, None)?;
            changed(ch, company, "project", id);
            Ok(())
        })
    }
}

pub(crate) fn invokable(tx: &rusqlite::Connection, s: &Staff) -> Result<(), String> {
    let c = get_company(tx, &s.company_id)?;
    if c.status != "active" {
        return Err(format!("company_{}", c.status));
    }
    match s.status.as_str() {
        "idle" | "running" | "error" => Ok(()),
        other => Err(format!("agent_{other}")),
    }
}
