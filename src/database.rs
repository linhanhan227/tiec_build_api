use rusqlite::{Connection, Result, OptionalExtension};
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::models::{Task, TaskStatus, TaskEvent};
use uuid::Uuid;
use chrono::{DateTime, Utc, Duration};

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

    fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
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

        if current_version < 2 {
            if !Self::column_exists(conn, "tasks", "build_duration")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN build_duration INTEGER",
                    [],
                )?;
            }

            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
                rusqlite::params![2, "add_build_duration_column", chrono::Utc::now().to_rfc3339()],
            )?;
        }

        if current_version < 3 {
            if !Self::column_exists(conn, "tasks", "priority")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN priority INTEGER NOT NULL DEFAULT 0",
                    [],
                )?;
            }

            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
                rusqlite::params![3, "add_task_priority_column", chrono::Utc::now().to_rfc3339()],
            )?;
        }

        if current_version < 4 {
            if !Self::column_exists(conn, "tasks", "next_run_at")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN next_run_at TEXT",
                    [],
                )?;
            }
            if !Self::column_exists(conn, "tasks", "lease_until")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN lease_until TEXT",
                    [],
                )?;
            }
            if !Self::column_exists(conn, "tasks", "locked_by")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN locked_by TEXT",
                    [],
                )?;
            }
            if !Self::column_exists(conn, "tasks", "last_error")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN last_error TEXT",
                    [],
                )?;
            }

            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_tasks_queue ON tasks(status, next_run_at, priority, created_at)",
                [],
            )?;

            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
                rusqlite::params![4, "add_queue_columns", chrono::Utc::now().to_rfc3339()],
            )?;
        }

        if current_version < 5 {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS task_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    task_id TEXT NOT NULL,
                    event_type TEXT NOT NULL,
                    status TEXT,
                    message TEXT,
                    worker_id TEXT,
                    created_at TEXT NOT NULL
                )",
                [],
            )?;

            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_task_events_task_id ON task_events(task_id)",
                [],
            )?;

            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_task_events_created_at ON task_events(created_at)",
                [],
            )?;

            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
                rusqlite::params![5, "create_task_events_table", chrono::Utc::now().to_rfc3339()],
            )?;
        }

        if current_version < 6 {
            if !Self::column_exists(conn, "tasks", "file_id")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN file_id TEXT NOT NULL DEFAULT ''",
                    [],
                )?;
            }

            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_tasks_file_user ON tasks(file_id, user_id)",
                [],
            )?;

            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
                rusqlite::params![6, "add_task_file_id_column", chrono::Utc::now().to_rfc3339()],
            )?;
        }

        // Future migrations can be added here

        Ok(())
    }

    pub async fn insert_task(&self, task: &Task) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO tasks (
                task_id, status, progress, estimated_time_remaining, current_step,
                error, created_at, updated_at, retry_count, max_retries,
                build_duration, priority, file_id, file_path, output_path, user_id
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                &task.file_id,
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
                    build_duration, priority, file_id, file_path, output_path, user_id
             FROM tasks WHERE task_id = ?"
        )?;

        let mut rows = stmt.query_map([task_id.to_string()], |row| {
            Ok(Task {
                task_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                status: match row.get::<_, String>(1)?.as_str() {
                    "排队中" => TaskStatus::Queued,
                    "处理中" => TaskStatus::Processing,
                    "成功" | "编译成功" => TaskStatus::Success,
                    "失败" | "未知错误" => TaskStatus::UnknownError,
                    "编译失败" => TaskStatus::CompilationFailed,
                    "超时" | "编译超时" => TaskStatus::Timeout,
                    "已取消" => TaskStatus::Cancelled,
                    _ => TaskStatus::UnknownError,
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
                file_id: row.get(12)?,
                file_path: row.get(13)?,
                output_path: row.get(14)?,
                user_id: row.get(15)?,
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

    pub async fn enqueue_task(&self, task_id: &Uuid) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE tasks SET status = ?, next_run_at = ?, lease_until = NULL, locked_by = NULL, updated_at = ? WHERE task_id = ?",
            rusqlite::params![
                serde_json::to_string(&TaskStatus::Queued).unwrap(),
                Utc::now().to_rfc3339(),
                Utc::now().to_rfc3339(),
                task_id.to_string(),
            ],
        )?;
        conn.execute(
            "INSERT INTO task_events (task_id, event_type, status, message, worker_id, created_at) VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                task_id.to_string(),
                "enqueued",
                serde_json::to_string(&TaskStatus::Queued).unwrap(),
                "Task enqueued",
                Option::<String>::None,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub async fn count_queued_tasks(&self) -> Result<u64> {
        let conn = self.conn.lock().await;
        let count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = ?",
            [serde_json::to_string(&TaskStatus::Queued).unwrap()],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub async fn get_oldest_queued_tasks(&self, limit: u64) -> Result<Vec<Uuid>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT task_id FROM tasks WHERE status = ? ORDER BY created_at ASC LIMIT ?",
        )?;

        let task_iter = stmt.query_map(
            rusqlite::params![serde_json::to_string(&TaskStatus::Queued).unwrap(), limit],
            |row| row.get::<_, String>(0),
        )?;

        let mut tasks = Vec::new();
        for task_id in task_iter {
            let id = task_id?;
            if let Ok(parsed) = Uuid::parse_str(&id) {
                tasks.push(parsed);
            }
        }
        Ok(tasks)
    }

    pub async fn cancel_task(&self, task_id: &Uuid, reason: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE tasks SET status = ?, error = ?, current_step = ?, updated_at = ?, lease_until = NULL, locked_by = NULL WHERE task_id = ?",
            rusqlite::params![
                serde_json::to_string(&TaskStatus::Cancelled).unwrap(),
                reason,
                "Cancelled",
                Utc::now().to_rfc3339(),
                task_id.to_string(),
            ],
        )?;
        conn.execute(
            "INSERT INTO task_events (task_id, event_type, status, message, worker_id, created_at) VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                task_id.to_string(),
                "cancelled",
                serde_json::to_string(&TaskStatus::Cancelled).unwrap(),
                reason,
                Option::<String>::None,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub async fn lease_next_task(&self, worker_id: &str, lease_secs: u64) -> Result<Option<Uuid>> {
        let conn = self.conn.lock().await;
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let lease_until = (now + Duration::seconds(lease_secs as i64)).to_rfc3339();

        conn.execute("BEGIN IMMEDIATE", [])?;

        let task_id: Option<String> = conn.query_row(
            "SELECT task_id FROM tasks
             WHERE status = ? AND (next_run_at IS NULL OR next_run_at <= ?)
             ORDER BY priority DESC, created_at ASC
             LIMIT 1",
            rusqlite::params![serde_json::to_string(&TaskStatus::Queued).unwrap(), now_str],
            |row| row.get(0),
        ).optional()?;

        if let Some(id) = task_id {
            let id_clone = id.clone();
            let updated = conn.execute(
                "UPDATE tasks SET status = ?, lease_until = ?, locked_by = ?, updated_at = ?, current_step = ?
                 WHERE task_id = ? AND status = ?",
                rusqlite::params![
                    serde_json::to_string(&TaskStatus::Processing).unwrap(),
                    lease_until,
                    worker_id,
                    now_str,
                    "Leased",
                    id,
                    serde_json::to_string(&TaskStatus::Queued).unwrap(),
                ],
            )?;

            if updated == 0 {
                conn.execute("ROLLBACK", [])?;
                return Ok(None);
            }

            conn.execute("COMMIT", [])?;
            conn.execute(
                "INSERT INTO task_events (task_id, event_type, status, message, worker_id, created_at) VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    id_clone.clone(),
                    "leased",
                    serde_json::to_string(&TaskStatus::Processing).unwrap(),
                    "Task leased",
                    worker_id,
                    Utc::now().to_rfc3339(),
                ],
            )?;
            return Ok(Uuid::parse_str(&id_clone).ok());
        }

        conn.execute("COMMIT", [])?;
        Ok(None)
    }

    pub async fn refresh_lease(&self, task_id: &Uuid, worker_id: &str, lease_secs: u64) -> Result<()> {
        let conn = self.conn.lock().await;
        let lease_until = (Utc::now() + Duration::seconds(lease_secs as i64)).to_rfc3339();
        conn.execute(
            "UPDATE tasks SET lease_until = ?, updated_at = ? WHERE task_id = ? AND locked_by = ?",
            rusqlite::params![lease_until, Utc::now().to_rfc3339(), task_id.to_string(), worker_id],
        )?;
        Ok(())
    }

    pub async fn clear_lease(&self, task_id: &Uuid) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE tasks SET lease_until = NULL, locked_by = NULL WHERE task_id = ?",
            rusqlite::params![task_id.to_string()],
        )?;
        Ok(())
    }

    pub async fn schedule_retry(&self, task_id: &Uuid, next_run_at: DateTime<Utc>, last_error: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE tasks SET status = ?, next_run_at = ?, last_error = ?, updated_at = ?, lease_until = NULL, locked_by = NULL WHERE task_id = ?",
            rusqlite::params![
                serde_json::to_string(&TaskStatus::Queued).unwrap(),
                next_run_at.to_rfc3339(),
                last_error,
                Utc::now().to_rfc3339(),
                task_id.to_string(),
            ],
        )?;
        conn.execute(
            "INSERT INTO task_events (task_id, event_type, status, message, worker_id, created_at) VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                task_id.to_string(),
                "retry_scheduled",
                serde_json::to_string(&TaskStatus::Queued).unwrap(),
                last_error,
                Option::<String>::None,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub async fn insert_task_event(
        &self,
        task_id: &Uuid,
        event_type: &str,
        status: Option<TaskStatus>,
        message: Option<&str>,
        worker_id: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        let status_str = status.map(|s| serde_json::to_string(&s).unwrap());
        conn.execute(
            "INSERT INTO task_events (task_id, event_type, status, message, worker_id, created_at) VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                task_id.to_string(),
                event_type,
                status_str,
                message,
                worker_id,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn delete_task(&self, task_id: &Uuid) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute("BEGIN IMMEDIATE", [])?;
        if let Err(e) = conn.execute("DELETE FROM task_events WHERE task_id = ?", [task_id.to_string()]) {
            let _ = conn.execute("ROLLBACK", []);
            return Err(e);
        }
        if let Err(e) = conn.execute("DELETE FROM tasks WHERE task_id = ?", [task_id.to_string()]) {
            let _ = conn.execute("ROLLBACK", []);
            return Err(e);
        }
        conn.execute("COMMIT", [])?;
        Ok(())
    }

    pub async fn get_all_tasks(&self) -> Result<Vec<Task>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
                "SELECT task_id, status, progress, estimated_time_remaining, current_step,
                    error, created_at, updated_at, retry_count, max_retries,
                    build_duration, priority, file_id, file_path, output_path, user_id
             FROM tasks ORDER BY created_at DESC"
        )?;

        let task_iter = stmt.query_map([], |row| {
            Ok(Task {
                task_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                status: match row.get::<_, String>(1)?.as_str() {
                    "排队中" => TaskStatus::Queued,
                    "处理中" => TaskStatus::Processing,
                    "成功" | "编译成功" => TaskStatus::Success,
                    "失败" | "未知错误" => TaskStatus::UnknownError,
                    "编译失败" => TaskStatus::CompilationFailed,
                    "超时" | "编译超时" => TaskStatus::Timeout,
                    "已取消" => TaskStatus::Cancelled,
                    _ => TaskStatus::UnknownError,
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
                file_id: row.get(12)?,
                file_path: row.get(13)?,
                output_path: row.get(14)?,
                user_id: row.get(15)?,
            })
        })?;

        let mut tasks = Vec::new();
        for task in task_iter {
            tasks.push(task?);
        }
        Ok(tasks)
    }

    pub async fn get_task_events(&self, task_id: &Uuid, limit: u64, offset: u64) -> Result<Vec<TaskEvent>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, event_type, status, message, worker_id, created_at
             FROM task_events
             WHERE task_id = ?
             ORDER BY created_at ASC
             LIMIT ? OFFSET ?",
        )?;

        let event_iter = stmt.query_map(
            rusqlite::params![task_id.to_string(), limit, offset],
            |row| {
                let status_str: Option<String> = row.get(3)?;
                let status = status_str.as_deref().map(|s| match s {
                    "排队中" => TaskStatus::Queued,
                    "处理中" => TaskStatus::Processing,
                    "成功" | "编译成功" => TaskStatus::Success,
                    "失败" | "未知错误" => TaskStatus::UnknownError,
                    "编译失败" => TaskStatus::CompilationFailed,
                    "超时" | "编译超时" => TaskStatus::Timeout,
                    "已取消" => TaskStatus::Cancelled,
                    _ => TaskStatus::UnknownError,
                });

                Ok(TaskEvent {
                    id: row.get(0)?,
                    task_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap(),
                    event_type: row.get(2)?,
                    status,
                    message: row.get(4)?,
                    worker_id: row.get(5)?,
                    created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                        .unwrap()
                        .with_timezone(&Utc),
                })
            },
        )?;

        let mut events = Vec::new();
        for event in event_iter {
            events.push(event?);
        }
        Ok(events)
    }

    pub async fn find_latest_task_by_file_id_and_user_id(&self, file_id: &str, user_id: &str) -> Result<Option<Task>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT task_id, status, progress, estimated_time_remaining, current_step,
                    error, created_at, updated_at, retry_count, max_retries,
                    build_duration, priority, file_id, file_path, output_path, user_id
             FROM tasks WHERE file_id = ? AND user_id = ?
             ORDER BY created_at DESC LIMIT 1"
        )?;

        let mut rows = stmt.query_map(rusqlite::params![file_id, user_id], |row| {
            Ok(Task {
                task_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                status: match row.get::<_, String>(1)?.as_str() {
                    "排队中" => TaskStatus::Queued,
                    "处理中" => TaskStatus::Processing,
                    "成功" | "编译成功" => TaskStatus::Success,
                    "失败" | "未知错误" => TaskStatus::UnknownError,
                    "编译失败" => TaskStatus::CompilationFailed,
                    "超时" | "编译超时" => TaskStatus::Timeout,
                    "已取消" => TaskStatus::Cancelled,
                    _ => TaskStatus::UnknownError,
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
                file_id: row.get(12)?,
                file_path: row.get(13)?,
                output_path: row.get(14)?,
                user_id: row.get(15)?,
            })
        })?;

        if let Some(task) = rows.next() {
            Ok(Some(task?))
        } else {
            Ok(None)
        }
    }

    #[allow(dead_code)]
    pub async fn get_tasks_by_status(&self, status: TaskStatus) -> Result<Vec<Task>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
                "SELECT task_id, status, progress, estimated_time_remaining, current_step,
                    error, created_at, updated_at, retry_count, max_retries,
                    build_duration, priority, file_id, file_path, output_path, user_id
             FROM tasks WHERE status = ? ORDER BY created_at DESC"
        )?;

        let task_iter = stmt.query_map([serde_json::to_string(&status).unwrap()], |row| {
            Ok(Task {
                task_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                status: match row.get::<_, String>(1)?.as_str() {
                    "排队中" => TaskStatus::Queued,
                    "处理中" => TaskStatus::Processing,
                    "成功" | "编译成功" => TaskStatus::Success,
                    "失败" | "未知错误" => TaskStatus::UnknownError,
                    "编译失败" => TaskStatus::CompilationFailed,
                    "超时" | "编译超时" => TaskStatus::Timeout,
                    "已取消" => TaskStatus::Cancelled,
                    _ => TaskStatus::UnknownError,
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
                file_id: row.get(12)?,
                file_path: row.get(13)?,
                output_path: row.get(14)?,
                user_id: row.get(15)?,
            })
        })?;

        let mut tasks = Vec::new();
        for task in task_iter {
            tasks.push(task?);
        }
        Ok(tasks)
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
            "SELECT COUNT(*) FROM tasks WHERE status IN (?, ?, ?)",
            rusqlite::params![
                serde_json::to_string(&TaskStatus::UnknownError).unwrap(),
                serde_json::to_string(&TaskStatus::CompilationFailed).unwrap(),
                serde_json::to_string(&TaskStatus::Timeout).unwrap()
            ],
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
            if !Self::column_exists(&conn, "tasks", "build_duration")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN build_duration INTEGER",
                    [],
                )?;
            }

            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
                rusqlite::params![2, "add_build_duration_column", chrono::Utc::now().to_rfc3339()],
            )?;

            applied_migrations.push("add_build_duration_column".to_string());
        }

        // Migration 3: Add task_priority column
        if current_version < 3 {
            if !Self::column_exists(&conn, "tasks", "priority")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN priority INTEGER NOT NULL DEFAULT 0",
                    [],
                )?;
            }

            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
                rusqlite::params![3, "add_task_priority_column", chrono::Utc::now().to_rfc3339()],
            )?;

            applied_migrations.push("add_task_priority_column".to_string());
        }

        // Migration 4: Add queue/lease columns
        if current_version < 4 {
            if !Self::column_exists(&conn, "tasks", "next_run_at")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN next_run_at TEXT",
                    [],
                )?;
            }
            if !Self::column_exists(&conn, "tasks", "lease_until")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN lease_until TEXT",
                    [],
                )?;
            }
            if !Self::column_exists(&conn, "tasks", "locked_by")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN locked_by TEXT",
                    [],
                )?;
            }
            if !Self::column_exists(&conn, "tasks", "last_error")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN last_error TEXT",
                    [],
                )?;
            }

            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_tasks_queue ON tasks(status, next_run_at, priority, created_at)",
                [],
            )?;

            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
                rusqlite::params![4, "add_queue_columns", chrono::Utc::now().to_rfc3339()],
            )?;

            applied_migrations.push("add_queue_columns".to_string());
        }

        // Migration 5: Add task events table
        if current_version < 5 {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS task_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    task_id TEXT NOT NULL,
                    event_type TEXT NOT NULL,
                    status TEXT,
                    message TEXT,
                    worker_id TEXT,
                    created_at TEXT NOT NULL
                )",
                [],
            )?;

            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_task_events_task_id ON task_events(task_id)",
                [],
            )?;

            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_task_events_created_at ON task_events(created_at)",
                [],
            )?;

            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
                rusqlite::params![5, "create_task_events_table", chrono::Utc::now().to_rfc3339()],
            )?;

            applied_migrations.push("create_task_events_table".to_string());
        }

        // Migration 6: Add file_id column
        if current_version < 6 {
            if !Self::column_exists(&conn, "tasks", "file_id")? {
                conn.execute(
                    "ALTER TABLE tasks ADD COLUMN file_id TEXT NOT NULL DEFAULT ''",
                    [],
                )?;
            }

            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_tasks_file_user ON tasks(file_id, user_id)",
                [],
            )?;

            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
                rusqlite::params![6, "add_task_file_id_column", chrono::Utc::now().to_rfc3339()],
            )?;

            applied_migrations.push("add_task_file_id_column".to_string());
        }

        Ok(applied_migrations)
    }
}