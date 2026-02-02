use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub enum TaskStatus {
    #[serde(rename = "排队中")]
    Queued,
    #[serde(rename = "处理中")]
    Processing,
    #[serde(rename = "编译成功")]
    Success,
    #[serde(rename = "未知错误")]
    UnknownError,
    #[serde(rename = "编译失败")]
    CompilationFailed,
    #[serde(rename = "编译超时")]
    Timeout,
    #[serde(rename = "已取消")]
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Task {
    pub task_id: Uuid,
    pub status: TaskStatus,
    pub progress: u8, // 0-100
    pub estimated_time_remaining: Option<u64>, // seconds
    pub current_step: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub retry_count: u8, // Number of retries attempted
    pub max_retries: u8, // Maximum allowed retries
    pub build_duration: Option<u64>, // Build duration in seconds
    pub priority: i32, // Task priority (higher = more important)
    #[serde(skip)]
    pub file_id: String, // SHA1 of uploaded file
    #[serde(skip)]
    pub file_path: String, // Path to the uploaded project file
    #[serde(skip)]
    pub output_path: Option<String>, // Path to the generated APK
    #[serde(skip)]
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TaskEvent {
    pub id: i64,
    pub task_id: Uuid,
    pub event_type: String,
    pub status: Option<TaskStatus>,
    pub message: Option<String>,
    pub worker_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UploadResponse {
    pub file_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BuildRequest {
    pub file_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BuildResponse {
    pub task_id: String,
    pub status: TaskStatus,
}
