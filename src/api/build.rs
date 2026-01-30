use actix_web::{get, post, web, Responder, HttpRequest};
use uuid::Uuid;
use chrono::Utc;
use crate::state::AppState;
use crate::models::{BuildRequest, BuildResponse, Task, TaskStatus};
use crate::error::ApiError;
use crate::utils::{json_response, accepted_json_response};
use std::sync::atomic::Ordering;

#[utoipa::path(
    post,
    path = "/api/v1/build",
    request_body = BuildRequest,
    responses(
        (status = 202, description = "Build task created", body = BuildResponse),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "File not found")
    )
)]
#[post("/build")]
pub async fn create_build(
    _req: HttpRequest,
    build_req: web::Json<BuildRequest>,
    data: web::Data<AppState>,
) -> Result<impl Responder, ApiError> {
    let file_id_str = &build_req.file_id;
    let file_id = Uuid::parse_str(file_id_str).map_err(|_| ApiError::BadRequest("Invalid File ID format".into()))?;

    let extracted_dir = format!("{}/{}_extracted", data.upload_dir, file_id);
    let tsp_path = format!("{}/{}.tsp", data.upload_dir, file_id);
    let file_path = if std::path::Path::new(&extracted_dir).exists() {
        extracted_dir
    } else if std::path::Path::new(&tsp_path).exists() {
        tsp_path
    } else {
        return Err(ApiError::NotFound("File not found or expired".into()));
    };

    let task_id = Uuid::new_v4();
    let task = Task {
        task_id,
        status: TaskStatus::Queued,
        progress: 0,
        estimated_time_remaining: None,
        current_step: Some("Queued".into()),
        error: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        retry_count: 0,
        max_retries: 3, // Allow up to 3 retries
        build_duration: None,
        priority: 0, // Default priority
        file_path: file_path.clone(),
        output_path: None,
        user_id: "test-user".into(), // Placeholder
    };

    data.tasks.insert(task_id, task.clone());
    data.total_tasks.fetch_add(1, Ordering::Relaxed);

    // Save to database
    if let Err(e) = data.save_task_to_db(&task).await {
        log::error!("Failed to save task to database: {}", e);
        return Err(ApiError::InternalServerError);
    }
    
    // Enqueue task
    data.enqueue_task(task_id, data.queue_capacity).await;

    Ok(accepted_json_response(BuildResponse {
        task_id: task_id.to_string(),
        status: TaskStatus::Queued,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/build/{id}/status",
    params(
        ("id" = String, Path, description = "Task ID")
    ),
    responses(
        (status = 200, description = "Task status", body = Task),
        (status = 404, description = "Task not found")
    )
)]
#[get("/build/{id}/status")]
pub async fn get_build_status(
    _req: HttpRequest,
    task_id_path: web::Path<String>,
    data: web::Data<AppState>,
) -> Result<impl Responder, ApiError> {
    let task_id_str = task_id_path.into_inner();
    let task_id = Uuid::parse_str(&task_id_str).map_err(|_| ApiError::BadRequest("Invalid Task ID".into()))?;
    if let Some(task) = data.tasks.get(&task_id) {
        Ok(json_response(task.clone()))
    } else {
        Err(ApiError::NotFound("Task not found".into()))
    }
}
