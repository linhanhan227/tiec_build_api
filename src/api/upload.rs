use actix_multipart::Multipart;
use actix_web::{post, web, Responder, HttpRequest};
use futures::{StreamExt, TryStreamExt};
use std::io::Write;
use uuid::Uuid;
use sha1::{Sha1, Digest};
use crate::models::UploadResponse;
use crate::state::AppState;
use crate::error::ApiError;
use crate::utils::{json_response, copy_dir_all};

#[utoipa::path(
    post,
    path = "/api/v1/upload",
    request_body(content = String, description = "Multipart form data with file field", content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "File uploaded successfully", body = UploadResponse),
        (status = 400, description = "Bad Request"),
        (status = 500, description = "Internal Server Error")
    )
)]
#[post("/upload")]
pub async fn upload_file(
    _req: HttpRequest,
    mut payload: Multipart,
    data: web::Data<AppState>,
) -> Result<impl Responder, ApiError> {
    let mut file_id = None;

    while let Ok(Some(mut field)) = payload.try_next().await {
        let content_type = field.content_type();

        // 1. Validate extension (case-insensitive)
        let content_disposition = field.content_disposition().ok_or(ApiError::BadRequest("Missing Content-Disposition".into()))?;
        let filename = content_disposition.get_filename().unwrap_or("unknown");
        if !filename.to_lowercase().ends_with(".tsp") {
             return Err(ApiError::UploadError("Invalid file extension. Only .tsp files are allowed.".into()));
        }

        // 2. Validate MIME type if provided (allow common zip/octet-stream)
        if let Some(mime) = content_type {
            let is_zip = mime.type_() == mime::APPLICATION
                && (mime.subtype() == "zip"
                    || mime.subtype() == "x-zip-compressed"
                    || mime.subtype() == "octet-stream");
            if !is_zip {
                return Err(ApiError::UploadError("Invalid file type. Only .tsp (zip) files are allowed.".into()));
            }
        }

        let temp_id = Uuid::new_v4();
        let temp_filepath = format!("{}/{}.uploading", data.upload_dir, temp_id);
        let temp_filepath_clone = temp_filepath.clone();

        // Save file
        let mut f = web::block(move || std::fs::File::create(temp_filepath_clone))
            .await
            .map_err(|_e| ApiError::InternalServerError)?
            .map_err(|_e| ApiError::InternalServerError)?;

        let mut size = 0;
        let mut hasher = Sha1::new();
        while let Some(chunk) = field.next().await {
            let data = chunk.map_err(|_| ApiError::UploadError("Transfer error".into()))?;
            size += data.len();
            if size > 100 * 1024 * 1024 { // 100MB limit
                 // Clean up partial file
                 let _ = std::fs::remove_file(&temp_filepath);
                 return Err(ApiError::UploadError("File too large. Max 100MB.".into()));
            }
            hasher.update(&data);
            f = web::block(move || f.write_all(&data).map(|_| f))
                .await
                .map_err(|_e| ApiError::InternalServerError)?
                .map_err(|_e| ApiError::InternalServerError)?;
        }

        let sha1_hex = format!("{:x}", hasher.finalize());
        let final_filepath = format!("{}/{}.tsp", data.upload_dir, sha1_hex);

        // 3. Verify ZIP integrity
        let filepath_verify = temp_filepath.clone();
        let integrity_check = web::block(move || {
            let file = std::fs::File::open(&filepath_verify)?;
            let archive = zip::ZipArchive::new(file)?;
            if archive.len() == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Empty zip"));
            }
            Ok::<(), std::io::Error>(())
        }).await;

        match integrity_check {
             Ok(Ok(_)) => {},
             _ => {
                 let _ = std::fs::remove_file(&temp_filepath);
                 return Err(ApiError::UploadError("Corrupted or invalid ZIP file.".into()));
             }
        }

        // Move temp file to final path if not exists
        if !std::path::Path::new(&final_filepath).exists() {
            if let Err(_e) = std::fs::rename(&temp_filepath, &final_filepath) {
                let _ = std::fs::remove_file(&temp_filepath);
                return Err(ApiError::InternalServerError);
            }
        } else {
            let _ = std::fs::remove_file(&temp_filepath);
        }

        // 4. Extract ZIP file and copy 安卓基本库 to project root/绳包
        let extract_dir = format!("{}/{}_extracted", data.upload_dir, sha1_hex);
        let tiec_root = std::path::Path::new(&data.tiecc_dir)
            .parent()
            .unwrap_or(std::path::Path::new("./.tiec"));
        let base_src = tiec_root.join("安卓基本库");
        let base_dst = std::path::Path::new(&extract_dir).join("绳包").join("安卓基本库");

        // 3. Ensure stdlib is available (optional check here, mainly done at startup)
        if let Err(e) = data.ensure_assets_extracted(false) {
            log::error!("Failed to ensure assets extracted: {}", e);
            return Err(ApiError::InternalServerError);
        }

        if !std::path::Path::new(&extract_dir).exists() {
            // Create extraction directory
            if let Err(_e) = std::fs::create_dir_all(&extract_dir) {
                return Err(ApiError::InternalServerError);
            }

            // Extract ZIP
            let filepath_clone = final_filepath.clone();
            let extract_dir_clone = extract_dir.clone();
            let extract_result = web::block(move || {
                let file = std::fs::File::open(&filepath_clone)?;
                let mut archive = zip::ZipArchive::new(file)?;
                archive.extract(&extract_dir_clone)?;
                Ok::<(), std::io::Error>(())
            }).await;

            match extract_result {
                Ok(Ok(_)) => {},
                _ => {
                    let _ = std::fs::remove_dir_all(&extract_dir);
                    return Err(ApiError::UploadError("Failed to extract ZIP file.".into()));
                }
            }
        }

        // Copy .tiec/安卓基本库 to project root/绳包/安卓基本库
        if base_src.exists() {
            if let Err(e) = copy_dir_all(&base_src, &base_dst) {
                log::warn!("Failed to copy 安卓基本库 to project: {}", e);
                // Don't fail the upload, just log the warning
            }
        } else {
            log::warn!("安卓基本库 not found: {}", base_src.display());
        }

        file_id = Some(sha1_hex);
        // Only process the first file
        break; 
    }

    if let Some(id) = file_id {
        Ok(json_response(UploadResponse {
            file_id: id,
        }))
    } else {
        Err(ApiError::BadRequest("No file uploaded".into()))
    }
}
