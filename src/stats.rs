use crate::db::Database;
use crate::error::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct SkillStat {
    pub skill_name: String,
    pub total: u64,
    pub codex: u64,
    pub claude_code: u64,
    pub explicit: u64,
    pub implicit: u64,
    pub skill: u64,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct InvocationTypeStat {
    pub runtime: String,
    pub invocation_type: String,
    pub total: u64,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
}

pub fn print_skill_stats_rows(stats: Vec<SkillStat>, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }

    println!(
        "{:<32} {:>8} {:>8} {:>12} {:>9} {:>9} {:<25} {:<25}",
        "skill", "total", "codex", "claude_code", "explicit", "implicit", "first_seen", "last_seen"
    );
    for stat in stats {
        println!(
            "{:<32} {:>8} {:>8} {:>12} {:>9} {:>9} {:<25} {:<25}",
            stat.skill_name,
            stat.total,
            stat.codex,
            stat.claude_code,
            stat.explicit,
            stat.implicit,
            stat.first_seen.unwrap_or_default(),
            stat.last_seen.unwrap_or_default()
        );
    }
    Ok(())
}

pub fn print_invocation_type_stats_rows(stats: Vec<InvocationTypeStat>, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }

    println!(
        "{:<14} {:<18} {:>8} {:<25} {:<25}",
        "runtime", "invocation_type", "total", "first_seen", "last_seen"
    );
    for stat in stats {
        println!(
            "{:<14} {:<18} {:>8} {:<25} {:<25}",
            stat.runtime,
            stat.invocation_type,
            stat.total,
            stat.first_seen.unwrap_or_default(),
            stat.last_seen.unwrap_or_default()
        );
    }
    Ok(())
}

pub fn skill_stats(db: &Database, since: Option<&str>) -> Result<Vec<SkillStat>> {
    let sql = if since.is_some() {
        r#"
        SELECT
          skill_name,
          COUNT(*) AS total_count,
          SUM(CASE WHEN runtime = 'codex' THEN 1 ELSE 0 END) AS codex_count,
          SUM(CASE WHEN runtime = 'claude_code' THEN 1 ELSE 0 END) AS claude_code_count,
          SUM(CASE WHEN invocation_type = 'explicit' THEN 1 ELSE 0 END) AS explicit_count,
          SUM(CASE WHEN invocation_type = 'implicit' THEN 1 ELSE 0 END) AS implicit_count,
          SUM(CASE WHEN invocation_type = 'skill' THEN 1 ELSE 0 END) AS skill_count,
          MIN(timestamp) AS first_seen,
          MAX(timestamp) AS last_seen
        FROM skill_invocations
        WHERE timestamp >= ?1
        GROUP BY skill_name
        ORDER BY total_count DESC, skill_name ASC
        "#
    } else {
        r#"
        SELECT
          skill_name,
          COUNT(*) AS total_count,
          SUM(CASE WHEN runtime = 'codex' THEN 1 ELSE 0 END) AS codex_count,
          SUM(CASE WHEN runtime = 'claude_code' THEN 1 ELSE 0 END) AS claude_code_count,
          SUM(CASE WHEN invocation_type = 'explicit' THEN 1 ELSE 0 END) AS explicit_count,
          SUM(CASE WHEN invocation_type = 'implicit' THEN 1 ELSE 0 END) AS implicit_count,
          SUM(CASE WHEN invocation_type = 'skill' THEN 1 ELSE 0 END) AS skill_count,
          MIN(timestamp) AS first_seen,
          MAX(timestamp) AS last_seen
        FROM skill_invocations
        GROUP BY skill_name
        ORDER BY total_count DESC, skill_name ASC
        "#
    };

    let mut stmt = db.connection().prepare(sql)?;
    let rows = if let Some(since) = since {
        stmt.query_map(params![since], map_skill_stat)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], map_skill_stat)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    Ok(rows)
}

pub fn invocation_type_stats(
    db: &Database,
    since: Option<&str>,
) -> Result<Vec<InvocationTypeStat>> {
    let sql = if since.is_some() {
        r#"
        SELECT runtime, invocation_type, COUNT(*) AS total_count, MIN(timestamp), MAX(timestamp)
        FROM skill_invocations
        WHERE timestamp >= ?1
        GROUP BY runtime, invocation_type
        ORDER BY total_count DESC, runtime ASC, invocation_type ASC
        "#
    } else {
        r#"
        SELECT runtime, invocation_type, COUNT(*) AS total_count, MIN(timestamp), MAX(timestamp)
        FROM skill_invocations
        GROUP BY runtime, invocation_type
        ORDER BY total_count DESC, runtime ASC, invocation_type ASC
        "#
    };

    let mut stmt = db.connection().prepare(sql)?;
    let rows = if let Some(since) = since {
        stmt.query_map(params![since], |row| {
            Ok(InvocationTypeStat {
                runtime: row.get(0)?,
                invocation_type: row.get(1)?,
                total: row.get::<_, i64>(2)? as u64,
                first_seen: row.get(3)?,
                last_seen: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], |row| {
            Ok(InvocationTypeStat {
                runtime: row.get(0)?,
                invocation_type: row.get(1)?,
                total: row.get::<_, i64>(2)? as u64,
                first_seen: row.get(3)?,
                last_seen: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
    };
    Ok(rows)
}

fn map_skill_stat(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillStat> {
    Ok(SkillStat {
        skill_name: row.get(0)?,
        total: row.get::<_, i64>(1)? as u64,
        codex: row.get::<_, i64>(2)? as u64,
        claude_code: row.get::<_, i64>(3)? as u64,
        explicit: row.get::<_, i64>(4)? as u64,
        implicit: row.get::<_, i64>(5)? as u64,
        skill: row.get::<_, i64>(6)? as u64,
        first_seen: row.get(7)?,
        last_seen: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::SkillInvocation;
    use std::path::Path;

    #[test]
    fn skill_stats_groups_counts_by_skill_across_coding_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let mut db = Database::open(&tmp.path().join("skillscope.sqlite")).unwrap();
        db.init().unwrap();
        insert_event(
            &db,
            "codex",
            "codex_session_jsonl",
            "explicit_skill_injection",
            "explicit",
            "code-review",
            1,
            "2026-07-01T00:00:00Z",
        );
        insert_event(
            &db,
            "claude_code",
            "claude_project_jsonl",
            "claude_skill_tool",
            "skill",
            "code-review",
            2,
            "2026-07-02T00:00:00Z",
        );
        insert_event(
            &db,
            "codex",
            "codex_session_jsonl",
            "implicit_skill_command",
            "implicit",
            "diagnose",
            3,
            "2026-07-03T00:00:00Z",
        );

        let stats = skill_stats(&db, None).unwrap();
        let code_review = stats
            .iter()
            .find(|stat| stat.skill_name == "code-review")
            .unwrap();

        assert_eq!(stats.len(), 2);
        assert_eq!(code_review.total, 2);
        assert_eq!(code_review.codex, 1);
        assert_eq!(code_review.claude_code, 1);
        assert_eq!(code_review.explicit, 1);
        assert_eq!(code_review.implicit, 0);
        assert_eq!(code_review.skill, 1);
        assert_eq!(
            code_review.first_seen.as_deref(),
            Some("2026-07-01T00:00:00Z")
        );
        assert_eq!(
            code_review.last_seen.as_deref(),
            Some("2026-07-02T00:00:00Z")
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_event(
        db: &Database,
        runtime: &str,
        source: &str,
        trigger_source: &str,
        invocation_type: &str,
        skill_name: &str,
        source_offset: u64,
        timestamp: &str,
    ) {
        let event = SkillInvocation::new_for_runtime(
            runtime,
            source,
            trigger_source,
            invocation_type,
            skill_name.to_string(),
            None,
            None,
            None,
            Some("session_1".to_string()),
            None,
            Path::new("/tmp/session.jsonl"),
            source_offset,
            1,
            None,
            timestamp.to_string(),
            1.0,
        );
        db.insert_invocation(&event).unwrap();
    }
}
