use crate::cli::GroupBy;
use crate::db::Database;
use crate::error::Result;
use rusqlite::params;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct SkillStat {
    runtime: String,
    skill_name: String,
    total: u64,
    explicit: u64,
    implicit: u64,
    skill: u64,
    first_seen: Option<String>,
    last_seen: Option<String>,
}

#[derive(Debug, Serialize)]
struct InvocationTypeStat {
    runtime: String,
    invocation_type: String,
    total: u64,
    first_seen: Option<String>,
    last_seen: Option<String>,
}

pub fn print_stats(
    db: &Database,
    group_by: GroupBy,
    since: Option<String>,
    json: bool,
) -> Result<()> {
    match group_by {
        GroupBy::Skill => print_skill_stats(db, since, json),
        GroupBy::InvocationType => print_invocation_type_stats(db, since, json),
    }
}

fn print_skill_stats(db: &Database, since: Option<String>, json: bool) -> Result<()> {
    let stats = skill_stats(db, since.as_deref())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }

    println!(
        "{:<14} {:<32} {:>8} {:>9} {:>9} {:>7} {:<25} {:<25}",
        "runtime", "skill", "total", "explicit", "implicit", "tool", "first_seen", "last_seen"
    );
    for stat in stats {
        println!(
            "{:<14} {:<32} {:>8} {:>9} {:>9} {:>7} {:<25} {:<25}",
            stat.runtime,
            stat.skill_name,
            stat.total,
            stat.explicit,
            stat.implicit,
            stat.skill,
            stat.first_seen.unwrap_or_default(),
            stat.last_seen.unwrap_or_default()
        );
    }
    Ok(())
}

fn print_invocation_type_stats(db: &Database, since: Option<String>, json: bool) -> Result<()> {
    let stats = invocation_type_stats(db, since.as_deref())?;
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

fn skill_stats(db: &Database, since: Option<&str>) -> Result<Vec<SkillStat>> {
    let sql = if since.is_some() {
        r#"
        SELECT
          runtime,
          skill_name,
          COUNT(*) AS total_count,
          SUM(CASE WHEN invocation_type = 'explicit' THEN 1 ELSE 0 END) AS explicit_count,
          SUM(CASE WHEN invocation_type = 'implicit' THEN 1 ELSE 0 END) AS implicit_count,
          SUM(CASE WHEN invocation_type = 'skill' THEN 1 ELSE 0 END) AS skill_count,
          MIN(timestamp) AS first_seen,
          MAX(timestamp) AS last_seen
        FROM skill_invocations
        WHERE timestamp >= ?1
        GROUP BY runtime, skill_name
        ORDER BY total_count DESC, runtime ASC, skill_name ASC
        "#
    } else {
        r#"
        SELECT
          runtime,
          skill_name,
          COUNT(*) AS total_count,
          SUM(CASE WHEN invocation_type = 'explicit' THEN 1 ELSE 0 END) AS explicit_count,
          SUM(CASE WHEN invocation_type = 'implicit' THEN 1 ELSE 0 END) AS implicit_count,
          SUM(CASE WHEN invocation_type = 'skill' THEN 1 ELSE 0 END) AS skill_count,
          MIN(timestamp) AS first_seen,
          MAX(timestamp) AS last_seen
        FROM skill_invocations
        GROUP BY runtime, skill_name
        ORDER BY total_count DESC, runtime ASC, skill_name ASC
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

fn invocation_type_stats(db: &Database, since: Option<&str>) -> Result<Vec<InvocationTypeStat>> {
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
        runtime: row.get(0)?,
        skill_name: row.get(1)?,
        total: row.get::<_, i64>(2)? as u64,
        explicit: row.get::<_, i64>(3)? as u64,
        implicit: row.get::<_, i64>(4)? as u64,
        skill: row.get::<_, i64>(5)? as u64,
        first_seen: row.get(6)?,
        last_seen: row.get(7)?,
    })
}
