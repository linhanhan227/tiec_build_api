use actix_web::{get, post, web, Responder, HttpRequest};
use uuid::Uuid;
use chrono::Utc;
use crate::state::AppState;
use crate::models::{BuildRequest, BuildResponse, Task, TaskStatus, TaskEvent};
use crate::error::ApiError;
use crate::utils::{json_response, accepted_json_response, get_client_ip};
use std::sync::atomic::Ordering;

pub async fn create_build_task_for_user(
    file_id_str: &str,
    user_id: String,
    data: web::Data<AppState>,
) -> Result<(Uuid, TaskStatus), ApiError> {
    if file_id_str.len() != 40 || !file_id_str.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest("Invalid File ID format".into()));
    }

    let extracted_dir = format!("{}/{}_extracted", data.upload_dir, file_id_str);
    let tsp_path = format!("{}/{}.tsp", data.upload_dir, file_id_str);
    let file_path = if std::path::Path::new(&extracted_dir).exists() {
        extracted_dir
    } else if std::path::Path::new(&tsp_path).exists() {
        tsp_path
    } else {
        return Err(ApiError::NotFound("File not found or expired".into()));
    };

    if let Ok(Some(existing_task)) = data
        .database
        .find_latest_task_by_file_id_and_user_id(file_id_str, &user_id)
        .await
    {
        match existing_task.status {
            TaskStatus::UnknownError | TaskStatus::Cancelled | TaskStatus::Timeout | TaskStatus::CompilationFailed => {}
            _ => {
                return Ok((existing_task.task_id, existing_task.status));
            }
        }
    }

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
        max_retries: data.max_retries,
        build_duration: None,
        priority: 0,
        file_id: file_id_str.to_string(),
        file_path: file_path.clone(),
        output_path: None,
        user_id,
    };

    data.tasks.insert(task_id, task.clone());
    data.total_tasks.fetch_add(1, Ordering::Relaxed);

    if let Err(e) = data.save_task_to_db(&task).await {
        log::error!("Failed to save task to database: {}", e);
        return Err(ApiError::InternalServerError);
    }

    data.enqueue_task(task_id, data.queue_capacity).await;

    Ok((task_id, TaskStatus::Queued))
}

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
    req: HttpRequest,
    build_req: web::Json<BuildRequest>,
    data: web::Data<AppState>,
) -> Result<impl Responder, ApiError> {
    let user_id = get_client_ip(&req);
    let (task_id, status) = create_build_task_for_user(&build_req.file_id, user_id, data).await?;

    Ok(accepted_json_response(BuildResponse {
        task_id: task_id.to_string(),
        status,
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

#[utoipa::path(
    get,
    path = "/api/v1/build/{id}/events",
    params(
        ("id" = String, Path, description = "Task ID"),
        ("limit" = Option<u64>, Query, description = "Max events to return (default 50)") ,
        ("offset" = Option<u64>, Query, description = "Offset for pagination (default 0)")
    ),
    responses(
        (status = 200, description = "Task events", body = [TaskEvent]),
        (status = 404, description = "Task not found")
    )
)]
#[get("/build/{id}/events")]
pub async fn get_build_events(
    _req: HttpRequest,
    task_id_path: web::Path<String>,
    query: web::Query<std::collections::HashMap<String, String>>,
    data: web::Data<AppState>,
) -> Result<impl Responder, ApiError> {
    let task_id_str = task_id_path.into_inner();
    let task_id = Uuid::parse_str(&task_id_str).map_err(|_| ApiError::BadRequest("Invalid Task ID".into()))?;

    if data.tasks.get(&task_id).is_none() {
        return Err(ApiError::NotFound("Task not found".into()));
    }

    let limit = query
        .get("limit")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(50)
        .min(200);
    let offset = query
        .get("offset")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    let events = data.database.get_task_events(&task_id, limit, offset).await
        .map_err(|_| ApiError::InternalServerError)?;

    Ok(json_response(events))
}
