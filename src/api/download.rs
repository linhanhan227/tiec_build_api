use actix_files::NamedFile;
use actix_web::{get, web, HttpRequest, Responder};
use crate::state::AppState;
use crate::models::TaskStatus;
use crate::error::ApiError;
use uuid::Uuid;
use std::path::PathBuf;

#[utoipa::path(
    get,
    path = "/api/v1/build/{id}/download",
    params(
        ("id" = String, Path, description = "Task ID")
    ),
    responses(
        (status = 200, description = "Download APK", content_type = "application/vnd.android.package-archive"),
        (status = 400, description = "Build not successful"),
        (status = 404, description = "Task or File not found")
    )
)]
#[get("/build/{id}/download")]
pub async fn download_build(
    _req: HttpRequest,
    task_id_path: web::Path<String>,
    data: web::Data<AppState>,
) -> Result<impl Responder, ApiError> {
    let task_id_str = task_id_path.into_inner();
    let task_id = Uuid::parse_str(&task_id_str).map_err(|_| ApiError::BadRequest("Invalid Task ID".into()))?;

    let output_path = if let Some(task) = data.tasks.get(&task_id) {
        if task.status == TaskStatus::Success {
            task.output_path.clone()
        } else {
            return Err(ApiError::BadRequest("Build not successful yet".into()));
        }
    } else {
        return Err(ApiError::NotFound("Task not found".into()));
    };

    if let Some(path_str) = output_path {
        let path = PathBuf::from(path_str);
        let named_file = NamedFile::open(path).map_err(|_| ApiError::NotFound("File not found on disk".into()))?;
        
        Ok(named_file
            .use_last_modified(true)
            .set_content_disposition(actix_web::http::header::ContentDisposition {
                disposition: actix_web::http::header::DispositionType::Attachment,
                parameters: vec![
                    actix_web::http::header::DispositionParam::Filename(format!("app-{}.apk", task_id).into()),
                ],
            }))
    } else {
        Err(ApiError::NotFound("Output path not set".into()))
    }
}
