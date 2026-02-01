use actix_web::{post, web, Responder, HttpRequest};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use crate::state::AppState;
use crate::error::ApiError;
use crate::utils::json_response;
use std::sync::atomic::Ordering;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CleanupRequest {
    /// 清理已完成任务的最大年龄（小时）
    pub completed_max_age_hours: Option<i64>,
    /// 清理失败任务的最大年龄（小时）
    pub failed_max_age_hours: Option<i64>,
    /// 清理过期任务的最大年龄（小时）
    pub expired_max_age_hours: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CleanupResponse {
    pub completed_cleaned: usize,
    pub failed_cleaned: usize,
    pub expired_cleaned: usize,
    pub total_cleaned: usize,
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/cleanup",
    request_body = CleanupRequest,
    responses(
        (status = 200, description = "Cleanup completed", body = CleanupResponse),
        (status = 400, description = "Bad Request"),
        (status = 500, description = "Internal Server Error")
    )
)]
#[post("/admin/cleanup")]
pub async fn cleanup_tasks(
    _req: HttpRequest,
    cleanup_req: web::Json<CleanupRequest>,
    data: web::Data<AppState>,
) -> Result<impl Responder, ApiError> {
    let completed_max_age = cleanup_req.completed_max_age_hours.unwrap_or(24); // 默认24小时
    let failed_max_age = cleanup_req.failed_max_age_hours.unwrap_or(168); // 默认7天
    let expired_max_age = cleanup_req.expired_max_age_hours.unwrap_or(1); // 默认1小时

    // Cleanup completed tasks
    let completed_cleaned = match data.database.cleanup_completed_tasks(completed_max_age).await {
        Ok(count) => count,
        Err(e) => {
            log::error!("Failed to cleanup completed tasks: {}", e);
            return Err(ApiError::InternalServerError);
        }
    };

    // Cleanup failed tasks
    let failed_cleaned = match data.database.cleanup_failed_tasks(failed_max_age).await {
        Ok(count) => count,
        Err(e) => {
            log::error!("Failed to cleanup failed tasks: {}", e);
            return Err(ApiError::InternalServerError);
        }
    };

    // Cleanup expired tasks (very old tasks regardless of status)
    let expired_cleaned = match data.database.cleanup_expired_tasks(expired_max_age).await {
        Ok(count) => count,
        Err(e) => {
            log::error!("Failed to cleanup expired tasks: {}", e);
            return Err(ApiError::InternalServerError);
        }
    };

    // Remove from in-memory cache
    let mut tasks_to_remove = Vec::new();
    for task in data.tasks.iter() {
        // This is a simplified check - in production you might want more sophisticated logic
        if task.created_at + chrono::Duration::hours(expired_max_age) < chrono::Utc::now() {
            tasks_to_remove.push(*task.key());
        }
    }

    for task_id in tasks_to_remove {
        data.tasks.remove(&task_id);
    }

    // Update statistics
    let (total, completed, failed, _) = data.database.get_task_stats().await
        .map_err(|_| ApiError::InternalServerError)?;
    data.total_tasks.store(total, Ordering::Relaxed);
    data.completed_tasks.store(completed, Ordering::Relaxed);
    data.failed_tasks.store(failed, Ordering::Relaxed);

    let total_cleaned = completed_cleaned + failed_cleaned + expired_cleaned;

    log::info!("Cleanup completed: {} completed, {} failed, {} expired, {} total",
               completed_cleaned, failed_cleaned, expired_cleaned, total_cleaned);

    Ok(json_response(CleanupResponse {
        completed_cleaned,
        failed_cleaned,
        expired_cleaned,
        total_cleaned,
    }))
}