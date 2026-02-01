use actix_web::{get, web, Responder, HttpRequest};
use serde::Serialize;
use utoipa::ToSchema;
use crate::state::AppState;
use crate::error::ApiError;
use crate::utils::json_response;

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub queue_size: usize,
    pub active_tasks: usize,
    pub total_tasks: u64,
    pub completed_tasks: u64,
    pub failed_tasks: u64,
    pub uptime: u64,
    pub version: String,
    pub build_time: String,
    pub git_commit: String,
    pub git_branch: String,
}

#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Health check response", body = HealthResponse)
    )
)]
#[get("/health")]
pub async fn health_check(
    _req: HttpRequest,
    data: web::Data<AppState>,
) -> Result<impl Responder, ApiError> {
    // Get statistics from database
    let (total_tasks, completed_tasks, failed_tasks, queued_tasks) = data.database.get_task_stats().await
        .map_err(|_| ApiError::InternalServerError)?;

    // Count active tasks from memory (currently processing)
    let active_tasks = data.tasks.iter()
        .filter(|task| matches!(task.status, crate::models::TaskStatus::Processing))
        .count();

    let health = HealthResponse {
        status: "healthy".to_string(),
        queue_size: queued_tasks as usize,
        active_tasks,
        total_tasks: total_tasks as u64,
        completed_tasks: completed_tasks as u64,
        failed_tasks: failed_tasks as u64,
        uptime: 0, // TODO: Implement uptime tracking
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_time: env!("BUILD_TIME").to_string(),
        git_commit: env!("GIT_COMMIT").to_string(),
        git_branch: env!("GIT_BRANCH").to_string(),
    };

    Ok(json_response(health))
}