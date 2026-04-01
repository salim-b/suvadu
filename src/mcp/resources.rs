use std::fmt::Write;

use serde_json::{json, Value};

use crate::models::SearchField;
use crate::repository::{QueryFilter, Repository};
use crate::util;

use chrono::TimeZone;

// ── Resource catalog ────────────────────────────────────────

/// Return the `resources/list` response.
pub fn list_resources(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "resources": [
                {
                    "uri": "suvadu://history/recent",
                    "name": "Recent Commands",
                    "description": "Last 20 commands with exit codes, directories, and executors",
                    "mimeType": "text/plain"
                },
                {
                    "uri": "suvadu://failures/recent",
                    "name": "Recent Failures",
                    "description": "Commands that failed in the last 24 hours, grouped by prompt",
                    "mimeType": "text/plain"
                },
                {
                    "uri": "suvadu://stats/today",
                    "name": "Today's Stats",
                    "description": "Command count, success rate, top commands, and top directories for today",
                    "mimeType": "text/plain"
                },
                {
                    "uri": "suvadu://risk/summary",
                    "name": "Risk Summary",
                    "description": "Risk assessment summary of recent agent commands",
                    "mimeType": "text/plain"
                },
                {
                    "uri": "suvadu://agents/activity",
                    "name": "Agent Activity",
                    "description": "Overview of AI agent activity: which agents, how many commands, success rates",
                    "mimeType": "text/plain"
                },
                {
                    "uri": "suvadu://agents/sessions",
                    "name": "Recent Agent Sessions",
                    "description": "Summary of the 5 most recent AI agent sessions, with prompts and command counts",
                    "mimeType": "text/plain"
                }
            ]
        }
    })
}

/// Return the `resources/templates/list` response.
pub fn list_resource_templates(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "resourceTemplates": [
                {
                    "uriTemplate": "suvadu://history/session/{session_id}",
                    "name": "Session History",
                    "description": "Full command history for a specific session",
                    "mimeType": "text/plain"
                }
            ]
        }
    })
}

// ── Resource reader ─────────────────────────────────────────

/// Read a resource by URI. Returns the text content or an error.
pub fn read_resource(repo: &Repository, uri: &str) -> Result<Value, String> {
    let content = match uri {
        "suvadu://history/recent" => read_recent_history(repo)?,
        "suvadu://failures/recent" => read_recent_failures(repo)?,
        "suvadu://stats/today" => read_today_stats(repo)?,
        "suvadu://risk/summary" => read_risk_summary(repo)?,
        "suvadu://agents/activity" => read_agent_activity(repo)?,
        "suvadu://agents/sessions" => read_agent_sessions(repo)?,
        _ if uri.starts_with("suvadu://history/session/") => {
            let session_id = uri.strip_prefix("suvadu://history/session/").unwrap_or("");
            read_session_history(repo, session_id)?
        }
        _ => return Err(format!("Unknown resource: {uri}")),
    };

    Ok(json!({
        "contents": [{
            "uri": uri,
            "mimeType": "text/plain",
            "text": content
        }]
    }))
}

// ── Resource handlers ───────────────────────────────────────

fn format_time(ms: i64) -> String {
    let ms_val = util::normalize_display_ms(ms);
    chrono::Local
        .timestamp_millis_opt(ms_val)
        .single()
        .map_or_else(
            || "unknown".to_string(),
            |dt| dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        )
}

fn read_recent_history(repo: &Repository) -> Result<String, String> {
    let entries = repo
        .get_recent_entries(20, 0, None, false, None)
        .map_err(|e| format!("query failed: {e}"))?;

    if entries.is_empty() {
        return Ok("No recent commands found.".to_string());
    }

    let mut out = String::new();
    let _ = writeln!(out, "Recent commands ({}):\n", entries.len());
    for entry in &entries {
        let exit = match entry.exit_code {
            Some(0) => "ok",
            Some(_) => "FAIL",
            None => "?",
        };
        let executor = entry.executor.as_deref().unwrap_or("terminal");
        let _ = writeln!(
            out,
            "  [{}] {} | {} | {} | {}",
            exit,
            entry.command,
            entry.cwd,
            executor,
            format_time(entry.started_at),
        );
    }
    Ok(out)
}

fn read_recent_failures(repo: &Repository) -> Result<String, String> {
    let now = chrono::Utc::now().timestamp_millis();
    let day_ago = now - 24 * 60 * 60 * 1000;

    let qf = QueryFilter {
        after: Some(day_ago),
        before: None,
        tag_id: None,
        exit_code: None,
        query: None,
        prefix_match: false,
        executor: None,
        cwd: None,
        field: SearchField::Command,
    };

    let entries = repo
        .get_entries_filtered(200, 0, &qf)
        .map_err(|e| format!("query failed: {e}"))?;

    let failures: Vec<_> = entries
        .iter()
        .filter(|e| e.exit_code.is_some_and(|c| c != 0))
        .collect();

    if failures.is_empty() {
        return Ok("No failures in the last 24 hours.".to_string());
    }

    let mut out = String::new();
    let _ = writeln!(out, "{} failures in the last 24 hours:\n", failures.len());

    for entry in failures.iter().take(20) {
        let code = entry.exit_code.unwrap_or(-1);
        let prompt = entry
            .context
            .as_ref()
            .and_then(|ctx| ctx.get("agent_prompt"))
            .map_or("(no prompt)", String::as_str);
        let executor = entry.executor.as_deref().unwrap_or("terminal");
        let _ = writeln!(
            out,
            "  exit {} | {} | {} | {}",
            code,
            entry.command,
            executor,
            format_time(entry.started_at),
        );
        if prompt != "(no prompt)" {
            let _ = writeln!(out, "    prompt: \"{prompt}\"");
        }
    }
    Ok(out)
}

fn read_today_stats(repo: &Repository) -> Result<String, String> {
    let now = chrono::Utc::now().timestamp_millis();
    let today_start = now - (now % (24 * 60 * 60 * 1000));

    let qf = QueryFilter {
        after: Some(today_start),
        before: None,
        tag_id: None,
        exit_code: None,
        query: None,
        prefix_match: false,
        executor: None,
        cwd: None,
        field: SearchField::Command,
    };

    let total = repo
        .count_filtered(&qf)
        .map_err(|e| format!("query failed: {e}"))?;

    let success_qf = QueryFilter {
        exit_code: Some(0),
        ..qf
    };
    let successes = repo
        .count_filtered(&success_qf)
        .map_err(|e| format!("query failed: {e}"))?;

    let rate = if total > 0 {
        successes * 100 / total
    } else {
        0
    };

    let entries = repo
        .get_entries_filtered(100, 0, &qf)
        .map_err(|e| format!("query failed: {e}"))?;

    let mut cmd_counts: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    let mut dir_counts: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for e in &entries {
        let program = e.command.split_whitespace().next().unwrap_or(&e.command);
        *cmd_counts.entry(program).or_default() += 1;
        *dir_counts.entry(e.cwd.as_str()).or_default() += 1;
    }

    let mut top_cmds: Vec<_> = cmd_counts.into_iter().collect();
    top_cmds.sort_by(|a, b| b.1.cmp(&a.1));
    top_cmds.truncate(5);

    let mut top_dirs: Vec<_> = dir_counts.into_iter().collect();
    top_dirs.sort_by(|a, b| b.1.cmp(&a.1));
    top_dirs.truncate(3);

    let mut out = String::new();
    let _ = writeln!(out, "Today's stats:\n");
    let _ = writeln!(out, "  Total commands: {total}");
    let _ = writeln!(out, "  Success rate: {rate}%\n");

    if !top_cmds.is_empty() {
        let _ = writeln!(out, "  Top commands:");
        for (cmd, count) in &top_cmds {
            let _ = writeln!(out, "    {count:>4}x  {cmd}");
        }
    }

    if !top_dirs.is_empty() {
        let _ = writeln!(out, "\n  Top directories:");
        for (dir, count) in &top_dirs {
            let _ = writeln!(out, "    {count:>4}x  {dir}");
        }
    }

    Ok(out)
}

fn read_risk_summary(repo: &Repository) -> Result<String, String> {
    use crate::risk;

    let now = chrono::Utc::now().timestamp_millis();
    let day_ago = now - 24 * 60 * 60 * 1000;

    let qf = QueryFilter {
        after: Some(day_ago),
        before: None,
        tag_id: None,
        exit_code: None,
        query: None,
        prefix_match: false,
        executor: None,
        cwd: None,
        field: SearchField::Command,
    };

    let entries = repo
        .get_entries_filtered(500, 0, &qf)
        .map_err(|e| format!("query failed: {e}"))?;

    let risk_summary = risk::session_risk(&entries);

    let mut out = String::new();
    let _ = writeln!(
        out,
        "Risk summary (last 24 hours, {} commands):\n",
        entries.len()
    );
    let _ = writeln!(out, "  Critical: {}", risk_summary.critical_count);
    let _ = writeln!(out, "  High:     {}", risk_summary.high_count);
    let _ = writeln!(out, "  Medium:   {}", risk_summary.medium_count);
    let _ = writeln!(out, "  Low:      {}", risk_summary.low_count);
    let _ = writeln!(out, "  Safe:     {}", risk_summary.safe_count);

    if !risk_summary.packages_installed.is_empty() {
        let _ = writeln!(out, "\n  Packages installed:");
        for pkg in &risk_summary.packages_installed {
            let _ = writeln!(out, "    {} ({})", pkg.packages.join(", "), pkg.manager);
        }
    }

    if !risk_summary.failed_commands.is_empty() {
        let _ = writeln!(out, "\n  Failed commands:");
        for fail in risk_summary.failed_commands.iter().take(10) {
            let _ = writeln!(
                out,
                "    exit {} | {} | {}",
                fail.exit_code,
                fail.command,
                format_time(fail.timestamp),
            );
        }
    }

    Ok(out)
}

fn read_agent_activity(repo: &Repository) -> Result<String, String> {
    let executors = repo
        .get_distinct_executors()
        .map_err(|e| format!("query failed: {e}"))?;

    let agents: Vec<&str> = executors
        .iter()
        .filter(|e| e.starts_with("agent:") && !e.ends_with("unknown"))
        .map(|e| e.strip_prefix("agent: ").unwrap_or(e.as_str()))
        .collect();

    if agents.is_empty() {
        return Ok("No AI agent activity detected.".to_string());
    }

    let now = chrono::Utc::now().timestamp_millis();
    let week_ago = now - 7 * 24 * 60 * 60 * 1000;

    let mut out = String::new();
    let _ = writeln!(out, "Agent activity (last 7 days):\n");
    let _ = writeln!(out, "  Detected agents: {}\n", agents.join(", "));

    for agent in &agents {
        let qf = QueryFilter {
            after: Some(week_ago),
            before: None,
            tag_id: None,
            exit_code: None,
            query: None,
            prefix_match: false,
            executor: Some(agent),
            cwd: None,
            field: SearchField::Command,
        };

        let total = repo.count_filtered(&qf).unwrap_or(0);
        let success_qf = QueryFilter {
            exit_code: Some(0),
            ..qf
        };
        let successes = repo.count_filtered(&success_qf).unwrap_or(0);
        let rate = if total > 0 {
            successes * 100 / total
        } else {
            0
        };

        let _ = writeln!(out, "  {agent}: {total} commands, {rate}% success");
    }

    Ok(out)
}

fn relative_time(now: i64, ms: i64) -> String {
    let diff = now - ms;
    let hours = diff / 3_600_000;
    let days = hours / 24;
    if days > 0 {
        format!("{days} day{} ago", if days == 1 { "" } else { "s" })
    } else if hours > 0 {
        format!("{hours} hour{} ago", if hours == 1 { "" } else { "s" })
    } else {
        "just now".to_string()
    }
}

fn read_agent_sessions(repo: &Repository) -> Result<String, String> {
    let now = chrono::Utc::now().timestamp_millis();
    let week_ago = now - 7 * 24 * 60 * 60 * 1000;

    let qf = QueryFilter {
        after: Some(week_ago),
        before: None,
        tag_id: None,
        exit_code: None,
        query: None,
        prefix_match: false,
        executor: None,
        cwd: None,
        field: SearchField::Command,
    };

    let entries = repo
        .get_entries_filtered(5000, 0, &qf)
        .map_err(|e| format!("query failed: {e}"))?;

    let sessions = group_agent_sessions(&entries);
    if sessions.is_empty() {
        return Ok("No agent sessions in the last 7 days.".to_string());
    }

    let mut out = String::new();
    let _ = writeln!(out, "Recent agent sessions (last 7 days):\n");
    for (i, (sid, executor, count, success, failure, last_at, prompt)) in
        sessions.iter().take(5).enumerate()
    {
        let rel = relative_time(now, *last_at);
        let status = if *failure == 0 {
            "all ok".to_string()
        } else {
            format!("{success} ok, {failure} failed")
        };
        let _ = writeln!(out, "{}. {sid} ({executor}, {rel})", i + 1);
        let _ = writeln!(out, "   {count} commands ({status})");
        if !prompt.is_empty() {
            let display = util::truncate_str(prompt, 80, "...");
            let _ = writeln!(out, "   \"{display}\"");
        }
        out.push('\n');
    }
    Ok(out)
}

/// Group agent entries by `session_id`, sorted by most recent.
fn group_agent_sessions(
    entries: &[crate::models::Entry],
) -> Vec<(&str, &str, usize, usize, usize, i64, String)> {
    let mut groups: std::collections::HashMap<&str, Vec<&crate::models::Entry>> =
        std::collections::HashMap::new();
    for entry in entries {
        if entry.executor_type.as_deref() != Some("agent") {
            continue;
        }
        groups
            .entry(entry.session_id.as_str())
            .or_default()
            .push(entry);
    }

    let mut sessions: Vec<_> = groups
        .into_iter()
        .map(|(session_id, cmds)| {
            let count = cmds.len();
            let success = cmds.iter().filter(|e| e.exit_code == Some(0)).count();
            let failure = cmds
                .iter()
                .filter(|e| e.exit_code.is_some_and(|c| c != 0))
                .count();
            let executor = cmds
                .first()
                .and_then(|e| e.executor.as_deref())
                .unwrap_or("unknown");
            let last_at = cmds.iter().map(|e| e.started_at).max().unwrap_or(0);
            let first_prompt = cmds
                .iter()
                .find_map(|e| {
                    e.context
                        .as_ref()
                        .and_then(|ctx| ctx.get("agent_prompt"))
                        .filter(|p| !p.is_empty())
                })
                .cloned()
                .unwrap_or_default();
            (
                session_id,
                executor,
                count,
                success,
                failure,
                last_at,
                first_prompt,
            )
        })
        .collect();

    sessions.sort_by(|a, b| b.5.cmp(&a.5));
    sessions
}

fn read_session_history(repo: &Repository, session_id: &str) -> Result<String, String> {
    if session_id.is_empty() {
        return Err("session_id is required".to_string());
    }

    let entries = repo
        .get_replay_entries(
            Some(session_id),
            &crate::repository::ReplayFilter {
                limit: Some(100),
                ..Default::default()
            },
        )
        .map_err(|e| format!("query failed: {e}"))?;

    if entries.is_empty() {
        return Ok(format!("No commands found for session {session_id}."));
    }

    let mut out = String::new();
    let _ = writeln!(out, "Session {session_id} ({} commands):\n", entries.len());
    for entry in &entries {
        let exit = match entry.exit_code {
            Some(0) => "ok",
            Some(_) => "FAIL",
            None => "?",
        };
        let _ = writeln!(
            out,
            "  [{}] {} | {} | {}",
            exit,
            entry.command,
            entry.cwd,
            format_time(entry.started_at),
        );
    }
    Ok(out)
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_resources_has_six() {
        let resp = list_resources(&json!(1));
        let resources = resp["result"]["resources"].as_array().unwrap();
        assert_eq!(resources.len(), 6);
        for r in resources {
            assert!(r["uri"].is_string());
            assert!(r["name"].is_string());
            assert!(r["mimeType"].is_string());
        }
    }

    #[test]
    fn test_list_templates_has_session() {
        let resp = list_resource_templates(&json!(1));
        let templates = resp["result"]["resourceTemplates"].as_array().unwrap();
        assert_eq!(templates.len(), 1);
        assert!(templates[0]["uriTemplate"]
            .as_str()
            .unwrap()
            .contains("{session_id}"));
    }

    #[test]
    fn test_read_recent_history_empty() {
        let (_dir, repo) = crate::test_utils::test_repo();
        let result = read_resource(&repo, "suvadu://history/recent");
        assert!(result.is_ok());
        let val = result.unwrap();
        assert!(val["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("No recent commands"));
    }

    #[test]
    fn test_read_failures_empty() {
        let (_dir, repo) = crate::test_utils::test_repo();
        let result = read_resource(&repo, "suvadu://failures/recent");
        assert!(result.is_ok());
        assert!(result.unwrap()["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("No failures"));
    }

    #[test]
    fn test_read_stats_empty() {
        let (_dir, repo) = crate::test_utils::test_repo();
        let result = read_resource(&repo, "suvadu://stats/today");
        assert!(result.is_ok());
        assert!(result.unwrap()["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Total commands: 0"));
    }

    #[test]
    fn test_read_risk_empty() {
        let (_dir, repo) = crate::test_utils::test_repo();
        let result = read_resource(&repo, "suvadu://risk/summary");
        assert!(result.is_ok());
    }

    #[test]
    fn test_read_agents_empty() {
        let (_dir, repo) = crate::test_utils::test_repo();
        let result = read_resource(&repo, "suvadu://agents/activity");
        assert!(result.is_ok());
        assert!(result.unwrap()["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("No AI agent activity"));
    }

    #[test]
    fn test_read_agent_sessions_empty() {
        let (_dir, repo) = crate::test_utils::test_repo();
        let result = read_resource(&repo, "suvadu://agents/sessions");
        assert!(result.is_ok());
        assert!(result.unwrap()["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("No agent sessions"));
    }

    #[test]
    fn test_read_agent_sessions_with_data() {
        let (_dir, repo) = crate::test_utils::test_repo();

        let session = crate::models::Session {
            id: "claude-test1".into(),
            hostname: "test".into(),
            created_at: chrono::Utc::now().timestamp_millis() - 3_600_000,
            tag_id: None,
        };
        repo.insert_session(&session).unwrap();

        let mut entry = crate::models::Entry::new(
            "claude-test1".into(),
            "cargo test".into(),
            "/project".into(),
            Some(0),
            chrono::Utc::now().timestamp_millis() - 3_600_000,
            chrono::Utc::now().timestamp_millis() - 3_599_000,
        );
        let mut ctx = std::collections::HashMap::new();
        ctx.insert("agent_prompt".into(), "run tests".into());
        entry.context = Some(ctx);
        entry.executor_type = Some("agent".into());
        entry.executor = Some("claude-code".into());
        repo.insert_entry(&entry).unwrap();

        let result = read_resource(&repo, "suvadu://agents/sessions");
        assert!(result.is_ok());
        let text = result.unwrap()["contents"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            text.contains("claude-test1"),
            "should contain session id: {text}"
        );
        assert!(
            text.contains("claude-code"),
            "should contain executor: {text}"
        );
        assert!(text.contains("run tests"), "should contain prompt: {text}");
    }

    #[test]
    fn test_read_unknown_resource() {
        let (_dir, repo) = crate::test_utils::test_repo();
        let result = read_resource(&repo, "suvadu://nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_read_session_empty_id() {
        let (_dir, repo) = crate::test_utils::test_repo();
        let result = read_resource(&repo, "suvadu://history/session/");
        assert!(result.is_err());
    }

    #[test]
    fn test_read_recent_with_data() {
        let (_dir, repo) = crate::test_utils::test_repo();
        let session = crate::models::Session {
            id: "s1".to_string(),
            hostname: "test".to_string(),
            created_at: chrono::Utc::now().timestamp_millis(),
            tag_id: None,
        };
        repo.insert_session(&session).unwrap();

        let entry = crate::models::Entry::new(
            "s1".to_string(),
            "cargo test".to_string(),
            "/project".to_string(),
            Some(0),
            chrono::Utc::now().timestamp_millis() - 1000,
            chrono::Utc::now().timestamp_millis(),
        );
        repo.insert_entry(&entry).unwrap();

        let result = read_resource(&repo, "suvadu://history/recent");
        assert!(result.is_ok());
        let text = result.unwrap()["contents"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            text.contains("cargo test"),
            "should contain command: {text}"
        );
        assert!(text.contains("/project"), "should contain dir: {text}");
    }
}
