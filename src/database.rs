use rusqlite::{Connection, Result};
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::models::{Task, TaskStatus};
use uuid::Uuid;
use chrono::{DateTime, Utc};

pub type DbConnection = Arc<Mutex<Connection>>;

pub struct Database {
    conn: DbConnection,
}

impl Clone for Database {
    fn clone(&self) -> Self {
        Database {
            conn: Arc::clone(&self.conn),
        }
    }
}

impl Database {
    pub fn new(db_path: &str) -> Result<Self> {
        // Ensure the parent directory exists
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            std::fs::create_dir_all(parent).expect("Failed to create database directory");
        }

        let conn = Connection::open(db_path)?;
        Self::init_tables(&conn)?;
        Ok(Database {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn init_tables(conn: &Connection) -> Result<()> {
        // Create migrations table first
        conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL
            )",
            [],
        )?;

        // Run migrations
        Self::run_migrations(conn)?;

        Ok(())
    }

    fn run_migrations(conn: &Connection) -> Result<()> {
        let current_version: i64 = conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        // Migration 1: Create initial tasks table
        if current_version < 1 {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS tasks (
                    task_id TEXT PRIMARY KEY,
                    status TEXT NOT NULL,
                    progress INTEGER NOT NULL DEFAULT 0,
                    estimated_time_remaining INTEGER,
                    current_step TEXT,
                    error TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    retry_count INTEGER NOT NULL DEFAULT 0,
                    max_retries INTEGER NOT NULL DEFAULT 3,
                    build_duration INTEGER,
                    priority INTEGER NOT NULL DEFAULT 0,
                    file_path TEXT NOT NULL,
                    output_path TEXT,
                    user_id TEXT NOT NULL
                )",
                [],
            )?;

            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
                rusqlite::params![1, "create_tasks_table", chrono::Utc::now().to_rfc3339()],
            )?;
        }

        // Future migrations can be added here
        // Migration 2: Add new columns
        // if current_version < 2 {
        //     // Add migration logic here
        // }

        Ok(())
    }

    pub async fn insert_task(&self, task: &Task) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO tasks (
                task_id, status, progress, estimated_time_remaining, current_step,
                error, created_at, updated_at, retry_count, max_retries,
                build_duration, priority, file_path, output_path, user_id
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                &task.task_id.to_string(),
                &serde_json::to_string(&task.status).unwrap(),
                &task.progress.to_string(),
                &task.estimated_time_remaining.map(|t| t.to_string()).unwrap_or_else(|| "".to_string()),
                &task.current_step.as_ref().unwrap_or(&"".to_string()),
                &task.error.as_ref().unwrap_or(&"".to_string()),
                &task.created_at.to_rfc3339(),
                &task.updated_at.to_rfc3339(),
                &task.retry_count.to_string(),
                &task.max_retries.to_string(),
                &task.build_duration.map(|t| t.to_string()).unwrap_or_else(|| "".to_string()),
                &task.priority.to_string(),
                &task.file_path,
                &task.output_path.as_ref().unwrap_or(&"".to_string()),
                &task.user_id,
            ],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn get_task(&self, task_id: &Uuid) -> Result<Option<Task>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT task_id, status, progress, estimated_time_remaining, current_step,
                    error, created_at, updated_at, retry_count, max_retries,
                    build_duration, priority, file_path, output_path, user_id
             FROM tasks WHERE task_id = ?"
        )?;

        let mut rows = stmt.query_map([task_id.to_string()], |row| {
            Ok(Task {
                task_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                status: match row.get::<_, String>(1)?.as_str() {
                    "排队中" => TaskStatus::Queued,
                    "处理中" => TaskStatus::Processing,
                    "成功" => TaskStatus::Success,
                    "失败" => TaskStatus::Failed,
                    _ => TaskStatus::Failed,
                },
                progress: row.get(2)?,
                estimated_time_remaining: row.get::<_, Option<String>>(3)?
                    .and_then(|s| s.parse().ok()),
                current_step: row.get(4)?,
                error: row.get(5)?,
                created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                    .unwrap().with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                    .unwrap().with_timezone(&Utc),
                retry_count: row.get(8)?,
                max_retries: row.get(9)?,
                build_duration: row.get::<_, Option<String>>(10)?
                    .and_then(|s| s.parse().ok()),
                priority: row.get(11)?,
                file_path: row.get(12)?,
                output_path: row.get(13)?,
                user_id: row.get(14)?,
            })
        })?;

        if let Some(task) = rows.next() {
            Ok(Some(task?))
        } else {
            Ok(None)
        }
    }

    pub async fn update_task(&self, task: &Task) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE tasks SET
                status = ?, progress = ?, estimated_time_remaining = ?,
                current_step = ?, error = ?, updated_at = ?,
                retry_count = ?, output_path = ?
             WHERE task_id = ?",
            &[
                &serde_json::to_string(&task.status).unwrap(),
                &task.progress.to_string(),
                &task.estimated_time_remaining.map(|t| t.to_string()).unwrap_or_else(|| "".to_string()),
                &task.current_step.as_ref().unwrap_or(&"".to_string()),
                &task.error.as_ref().unwrap_or(&"".to_string()),
                &task.updated_at.to_rfc3339(),
                &task.retry_count.to_string(),
                &task.output_path.as_ref().unwrap_or(&"".to_string()),
                &task.task_id.to_string(),
            ],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn delete_task(&self, task_id: &Uuid) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute("DELETE FROM tasks WHERE task_id = ?", [task_id.to_string()])?;
        Ok(())
    }

    pub async fn get_all_tasks(&self) -> Result<Vec<Task>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT task_id, status, progress, estimated_time_remaining, current_step,
                    error, created_at, updated_at, retry_count, max_retries,
                    build_duration, priority, file_path, output_path, user_id
             FROM tasks ORDER BY created_at DESC"
        )?;

        let task_iter = stmt.query_map([], |row| {
            Ok(Task {
                task_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                status: match row.get::<_, String>(1)?.as_str() {
                    "排队中" => TaskStatus::Queued,
                    "处理中" => TaskStatus::Processing,
                    "成功" => TaskStatus::Success,
                    "失败" => TaskStatus::Failed,
                    _ => TaskStatus::Failed,
                },
                progress: row.get(2)?,
                estimated_time_remaining: row.get::<_, Option<String>>(3)?
                    .and_then(|s| s.parse().ok()),
                current_step: row.get(4)?,
                error: row.get(5)?,
                created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                    .unwrap().with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                    .unwrap().with_timezone(&Utc),
                retry_count: row.get(8)?,
                max_retries: row.get(9)?,
                build_duration: row.get::<_, Option<String>>(10)?
                    .and_then(|s| s.parse().ok()),
                priority: row.get(11)?,
                file_path: row.get(12)?,
                output_path: row.get(13)?,
                user_id: row.get(14)?,
            })
        })?;

        let mut tasks = Vec::new();
        for task in task_iter {
            tasks.push(task?);
        }
        Ok(tasks)
    }

    #[allow(dead_code)]
    pub async fn get_tasks_by_status(&self, status: TaskStatus) -> Result<Vec<Task>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT task_id, status, progress, estimated_time_remaining, current_step,
                    error, created_at, updated_at, retry_count, max_retries,
                    build_duration, priority, file_path, output_path, user_id
             FROM tasks WHERE status = ? ORDER BY created_at DESC"
        )?;

        let task_iter = stmt.query_map([serde_json::to_string(&status).unwrap()], |row| {
            Ok(Task {
                task_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                status: match row.get::<_, String>(1)?.as_str() {
                    "排队中" => TaskStatus::Queued,
                    "处理中" => TaskStatus::Processing,
                    "成功" => TaskStatus::Success,
                    "失败" => TaskStatus::Failed,
                    _ => TaskStatus::Failed,
                },
                progress: row.get(2)?,
                estimated_time_remaining: row.get::<_, Option<String>>(3)?
                    .and_then(|s| s.parse().ok()),
                current_step: row.get(4)?,
                error: row.get(5)?,
                created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                    .unwrap().with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                    .unwrap().with_timezone(&Utc),
                retry_count: row.get(8)?,
                max_retries: row.get(9)?,
                build_duration: row.get::<_, Option<String>>(10)?
                    .and_then(|s| s.parse().ok()),
                priority: row.get(11)?,
                file_path: row.get(12)?,
                output_path: row.get(13)?,
                user_id: row.get(14)?,
            })
        })?;

        let mut tasks = Vec::new();
        for task in task_iter {
            tasks.push(task?);
        }
        Ok(tasks)
    }

    pub async fn cleanup_completed_tasks(&self, max_age_hours: i64) -> Result<usize> {
        let conn = self.conn.lock().await;
        let cutoff_time = Utc::now() - chrono::Duration::hours(max_age_hours);

        let rows_affected = conn.execute(
            "DELETE FROM tasks WHERE status = ? AND updated_at < ?",
            [
                serde_json::to_string(&TaskStatus::Success).unwrap(),
                cutoff_time.to_rfc3339(),
            ],
        )?;

        Ok(rows_affected)
    }

    pub async fn cleanup_failed_tasks(&self, max_age_hours: i64) -> Result<usize> {
        let conn = self.conn.lock().await;
        let cutoff_time = Utc::now() - chrono::Duration::hours(max_age_hours);

        let rows_affected = conn.execute(
            "DELETE FROM tasks WHERE status = ? AND updated_at < ?",
            [
                serde_json::to_string(&TaskStatus::Failed).unwrap(),
                cutoff_time.to_rfc3339(),
            ],
        )?;

        Ok(rows_affected)
    }

    pub async fn cleanup_expired_tasks(&self, max_age_hours: i64) -> Result<usize> {
        let conn = self.conn.lock().await;
        let cutoff_time = Utc::now() - chrono::Duration::hours(max_age_hours);

        let rows_affected = conn.execute(
            "DELETE FROM tasks WHERE created_at < ?",
            [cutoff_time.to_rfc3339()],
        )?;

        Ok(rows_affected)
    }

    pub async fn get_task_stats(&self) -> Result<(u64, u64, u64, u64)> {
        let conn = self.conn.lock().await;

        let total: u64 = conn.query_row(
            "SELECT COUNT(*) FROM tasks",
            [],
            |row| row.get(0),
        )?;

        let completed: u64 = conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = ?",
            [serde_json::to_string(&TaskStatus::Success).unwrap()],
            |row| row.get(0),
        )?;

        let failed: u64 = conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = ?",
            [serde_json::to_string(&TaskStatus::Failed).unwrap()],
            |row| row.get(0),
        )?;

        let queued: u64 = conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = ?",
            [serde_json::to_string(&TaskStatus::Queued).unwrap()],
            |row| row.get(0),
        )?;

        Ok((total, completed, failed, queued))
    }

    #[allow(dead_code)]
    pub async fn get_schema_version(&self) -> Result<i64> {
        let conn = self.conn.lock().await;
        let version: i64 = conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        Ok(version)
    }

    #[allow(dead_code)]
    pub async fn run_pending_migrations(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().await;
        let current_version: i64 = conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        let mut applied_migrations = Vec::new();

        // Migration 2: Add build_duration column for performance tracking
        if current_version < 2 {
            conn.execute(
                "ALTER TABLE tasks ADD COLUMN build_duration INTEGER",
                [],
            )?;

            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
                rusqlite::params![2, "add_build_duration_column", chrono::Utc::now().to_rfc3339()],
            )?;

            applied_migrations.push("add_build_duration_column".to_string());
        }

        // Migration 3: Add task_priority column
        if current_version < 3 {
            conn.execute(
                "ALTER TABLE tasks ADD COLUMN priority INTEGER NOT NULL DEFAULT 0",
                [],
            )?;

            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
                rusqlite::params![3, "add_task_priority_column", chrono::Utc::now().to_rfc3339()],
            )?;

            applied_migrations.push("add_task_priority_column".to_string());
        }

        Ok(applied_migrations)
    }
}