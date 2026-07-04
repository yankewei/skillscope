use crate::error::Result;
use crate::events::SkillInvocation;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Default, Serialize)]
pub struct ParsedFile {
    pub path: String,
    pub canonical_path: Option<String>,
    pub file_size: u64,
    pub modified_at: Option<String>,
    pub byte_offset: u64,
    pub line_number: u64,
    pub partial_line: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub cwd: Option<String>,
    pub fingerprint: Option<String>,
    pub last_error: Option<String>,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(Self { conn })
    }

    pub fn init(&mut self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS schema_migrations (
              version INTEGER PRIMARY KEY,
              applied_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS parsed_files (
              path TEXT PRIMARY KEY,
              canonical_path TEXT,
              file_size INTEGER NOT NULL DEFAULT 0,
              modified_at TEXT,
              byte_offset INTEGER NOT NULL DEFAULT 0,
              line_number INTEGER NOT NULL DEFAULT 0,
              partial_line TEXT NOT NULL DEFAULT '',
              session_id TEXT,
              turn_id TEXT,
              cwd TEXT,
              fingerprint TEXT,
              last_error TEXT,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS skill_invocations (
              id TEXT PRIMARY KEY,
              runtime TEXT NOT NULL,
              source TEXT NOT NULL,
              trigger_source TEXT NOT NULL,
              invocation_type TEXT NOT NULL,
              skill_name TEXT NOT NULL,
              skill_path TEXT,
              skill_scope TEXT,
              plugin_id TEXT,
              session_id TEXT,
              turn_id TEXT,
              source_file TEXT NOT NULL,
              source_offset INTEGER NOT NULL,
              source_line INTEGER NOT NULL,
              tool_call_id TEXT,
              timestamp TEXT NOT NULL,
              confidence REAL NOT NULL,
              created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_skill_invocations_skill_name
              ON skill_invocations(skill_name);
            CREATE INDEX IF NOT EXISTS idx_skill_invocations_timestamp
              ON skill_invocations(timestamp);
            CREATE INDEX IF NOT EXISTS idx_skill_invocations_session
              ON skill_invocations(session_id);
            CREATE INDEX IF NOT EXISTS idx_skill_invocations_source_file
              ON skill_invocations(source_file);
            "#,
        )?;
        self.ensure_parsed_file_column("session_id", "TEXT")?;
        self.ensure_parsed_file_column("turn_id", "TEXT")?;
        self.ensure_parsed_file_column("cwd", "TEXT")?;
        self.conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (1, ?1)",
            params![Utc::now().to_rfc3339()],
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (2, ?1)",
            params![Utc::now().to_rfc3339()],
        )?;
        if !self.migration_applied(3)? {
            self.migrate_tool_call_ids_into_invocation_ids()?;
            self.conn.execute(
                "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (3, ?1)",
                params![Utc::now().to_rfc3339()],
            )?;
        }
        Ok(())
    }

    fn migration_applied(&self, version: i64) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM schema_migrations WHERE version = ?1",
                params![version],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    fn migrate_tool_call_ids_into_invocation_ids(&self) -> Result<()> {
        self.conn.execute(
            r#"
            UPDATE OR IGNORE skill_invocations
            SET id = runtime || ':' || source_file || ':' || source_offset || ':' ||
              trigger_source || ':' || COALESCE(skill_path, skill_name) || ':' || tool_call_id
            WHERE tool_call_id IS NOT NULL
              AND tool_call_id != ''
              AND id != runtime || ':' || source_file || ':' || source_offset || ':' ||
                trigger_source || ':' || COALESCE(skill_path, skill_name) || ':' || tool_call_id
            "#,
            [],
        )?;
        Ok(())
    }

    fn ensure_parsed_file_column(&self, name: &str, definition: &str) -> Result<()> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(parsed_files)")?;
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if !columns.iter().any(|column| column == name) {
            self.conn.execute(
                &format!("ALTER TABLE parsed_files ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
        Ok(())
    }

    pub fn parsed_file(&self, path: &Path) -> Result<Option<ParsedFile>> {
        let path = path.to_string_lossy();
        self.conn
            .query_row(
                r#"
                SELECT path, canonical_path, file_size, modified_at, byte_offset,
                       line_number, partial_line, session_id, turn_id, cwd,
                       fingerprint, last_error
                FROM parsed_files
                WHERE path = ?1
                "#,
                params![path.as_ref()],
                |row| {
                    Ok(ParsedFile {
                        path: row.get(0)?,
                        canonical_path: row.get(1)?,
                        file_size: row.get::<_, i64>(2)? as u64,
                        modified_at: row.get(3)?,
                        byte_offset: row.get::<_, i64>(4)? as u64,
                        line_number: row.get::<_, i64>(5)? as u64,
                        partial_line: row.get(6)?,
                        session_id: row.get(7)?,
                        turn_id: row.get(8)?,
                        cwd: row.get(9)?,
                        fingerprint: row.get(10)?,
                        last_error: row.get(11)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn upsert_parsed_file(&self, parsed: &ParsedFile) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO parsed_files (
              path, canonical_path, file_size, modified_at, byte_offset,
              line_number, partial_line, session_id, turn_id, cwd,
              fingerprint, last_error, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(path) DO UPDATE SET
              canonical_path = excluded.canonical_path,
              file_size = excluded.file_size,
              modified_at = excluded.modified_at,
              byte_offset = excluded.byte_offset,
              line_number = excluded.line_number,
              partial_line = excluded.partial_line,
              session_id = excluded.session_id,
              turn_id = excluded.turn_id,
              cwd = excluded.cwd,
              fingerprint = excluded.fingerprint,
              last_error = excluded.last_error,
              updated_at = excluded.updated_at
            "#,
            params![
                parsed.path,
                parsed.canonical_path,
                parsed.file_size as i64,
                parsed.modified_at,
                parsed.byte_offset as i64,
                parsed.line_number as i64,
                parsed.partial_line,
                parsed.session_id,
                parsed.turn_id,
                parsed.cwd,
                parsed.fingerprint,
                parsed.last_error,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn insert_invocation(&self, event: &SkillInvocation) -> Result<bool> {
        let rows = self.conn.execute(
            r#"
            INSERT OR IGNORE INTO skill_invocations (
              id, runtime, source, trigger_source, invocation_type, skill_name,
              skill_path, skill_scope, plugin_id, session_id, turn_id,
              source_file, source_offset, source_line, tool_call_id,
              timestamp, confidence, created_at
            ) VALUES (
              ?1, ?2, ?3, ?4, ?5, ?6,
              ?7, ?8, ?9, ?10, ?11,
              ?12, ?13, ?14, ?15,
              ?16, ?17, ?18
            )
            "#,
            params![
                event.id,
                event.runtime,
                event.source,
                event.trigger_source,
                event.invocation_type,
                event.skill_name,
                event.skill_path,
                event.skill_scope,
                event.plugin_id,
                event.session_id,
                event.turn_id,
                event.source_file,
                event.source_offset as i64,
                event.source_line as i64,
                event.tool_call_id,
                event.timestamp,
                event.confidence,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(rows > 0)
    }

    pub fn parsed_file_count(&self) -> Result<u64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM parsed_files", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    pub fn latest_parse_error(&self) -> Result<Option<String>> {
        self.conn
            .query_row(
                r#"
                SELECT last_error
                FROM parsed_files
                WHERE last_error IS NOT NULL AND last_error != ''
                ORDER BY updated_at DESC
                LIMIT 1
                "#,
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_migrates_existing_tool_call_invocation_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("skillscope.sqlite");
        let mut db = Database::open(&db_path).unwrap();
        db.init().unwrap();
        db.conn
            .execute("DELETE FROM schema_migrations WHERE version = 3", [])
            .unwrap();
        db.conn
            .execute(
                r#"
                INSERT INTO skill_invocations (
                  id, runtime, source, trigger_source, invocation_type, skill_name,
                  skill_path, skill_scope, plugin_id, session_id, turn_id,
                  source_file, source_offset, source_line, tool_call_id,
                  timestamp, confidence, created_at
                ) VALUES (
                  'claude_code:/tmp/session.jsonl:42:claude_skill_tool:think',
                  'claude_code', 'claude_project_jsonl', 'claude_skill_tool', 'skill', 'think',
                  NULL, NULL, NULL, 'session_1', NULL,
                  '/tmp/session.jsonl', 42, 1, 'toolu_1',
                  '2026-07-03T00:00:00Z', 1.0, '2026-07-03T00:00:00Z'
                )
                "#,
                [],
            )
            .unwrap();

        db.init().unwrap();

        let id: String = db
            .conn
            .query_row("SELECT id FROM skill_invocations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            id,
            "claude_code:/tmp/session.jsonl:42:claude_skill_tool:think:toolu_1"
        );
    }
}
