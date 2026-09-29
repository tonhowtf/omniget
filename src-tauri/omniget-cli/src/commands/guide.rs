//! `omniget guide [topic]`: short reference texts for people and for agents
//! that drive the CLI (the Claude Code skill `omniget-agents` points here, so
//! the skill stays small and the reference ships with the binary it
//! describes).

/// `(topic, one line, text)`.
const TOPICS: &[(&str, &str, &str)] = &[(
    "agents",
    "delegate a task to one of your OmniGet agents as a background job",
    AGENTS,
)];

const AGENTS: &str = r#"OmniGet agents: delegating a task as a background job
======================================================

The OmniGet app must be running. These commands talk to it over its local
bridge with the CLI token the app keeps for you (or OMNIGET_TOKEN).

Inside a job, do not delegate
  If OMNIGET_JOB_ID is set, this process is itself an OmniGet job. A job never
  starts another job (depth 1): `omniget agent run` refuses and so does the app.
  Do the task yourself.

Pick an agent
  omniget agent agents                     roster ids and names

Start a job and wait for its end
  omniget agent run --agent <id> "<task>"
      --title <text>        short name shown in lists
      --command-id <id>     idempotency key: the same id returns the same job
      --workspace <dir>     folder to work in (default: the current one)
      --isolate | --shared  own git worktree | the folder itself (default: a
                            worktree only when another job writes there)
      --base <ref>          start the worktree from this branch or commit
      --deadline-min <n>    stop after n minutes
      --detach              print the job id and return at once
  It waits on the app's long poll (no timer) and only accepts an end that is
  newer than the start. Exit 0: done, review or done_unreported; exit 1:
  failed, cancelled, or the agent said OUTCOME: failed.

Follow, stop, inspect
  omniget agent jobs [<id>] [--cancel]     list, follow one, or stop it
  omniget agent status                     what runs and what waits, and why
  omniget agent explain <id>               why a job is in its state
  omniget agent diff <id> [--patch]        files the job changed

Permission questions
  A job may stop at waiting_approval. Answer only when the person says so:
  omniget agent approvals --job <id>
  omniget agent approve <approval id> [--session | --deny | --cancel]

End states
  done              ended well, reported its OUTCOME
  review            ended well and changed files: look at the diff
  done_unreported   the process ended well, but the agent wrote no OUTCOME line
  failed / cancelled / interrupted

The agent's last line is `OUTCOME: succeeded|failed — <up to 3 sentences>`;
the app reads it into the job's `outcome`.

HTTP (scripts), with a token from LLM -> Jobs -> Access tokens
  POST /v1/commands  {"command_id","type":"job.start","agent_id","prompt",
                      "title","workspace","isolation","base","deadline_min",
                      "budget_usd","model","effort","run_mode","resume_of"}
       types: job.start | job.interrupt {job_id} |
              approval.respond {approval_id, decision}
       same command_id again -> same receipt (header Idempotent-Replayed);
       same id, other payload -> 409 ERR_COMMAND_CONFLICT
  GET  /v1/jobs/<id>?after_seq=N
  GET  /v1/jobs/<id>/wait?until=terminal&after_seq=N&timeout_ms=30000
       408 ERR_WAIT_TIMEOUT: call again

MCP (a connection with the agents_run grant): agent_run, job_get, job_wait,
job_cancel, jobs_state, job_diff, and approval_respond with its own grant.
"#;

pub fn execute(topic: Option<String>) -> anyhow::Result<()> {
    match topic.as_deref() {
        None => {
            println!("omniget guide <topic>\n");
            for (name, line, _) in TOPICS {
                println!("  {name:<10} {line}");
            }
            Ok(())
        }
        Some(t) => match TOPICS.iter().find(|(name, _, _)| *name == t) {
            Some((_, _, text)) => {
                print!("{text}");
                Ok(())
            }
            None => anyhow::bail!(
                "no guide `{t}`; topics: {}",
                TOPICS
                    .iter()
                    .map(|(n, _, _)| *n)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The skill `omniget-agents` points at this guide and at these flags.
    #[test]
    fn the_agents_guide_names_the_depth_rule_and_the_flags_it_documents() {
        for needle in [
            "OMNIGET_JOB_ID",
            "--command-id",
            "--detach",
            "done_unreported",
            "OUTCOME: succeeded|failed",
            "/v1/commands",
            "agent_run",
        ] {
            assert!(AGENTS.contains(needle), "{needle}");
        }
        let skill =
            include_str!("../../../../claude-plugin/omniget/skills/omniget-agents/SKILL.md");
        assert!(skill.contains("omniget guide agents"));
        assert!(skill.contains("OMNIGET_JOB_ID"));
    }
}
