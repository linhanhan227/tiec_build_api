use actix_web::{HttpResponse, ResponseError};
use derive_more::Display;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Display, Error)]
pub enum ApiError {
    #[display(fmt = "Internal Server Error")]
    InternalServerError,

    #[display(fmt = "Bad Request: {}", _0)]
    BadRequest(String),

    #[display(fmt = "Not Found: {}", _0)]
    NotFound(String),

    #[display(fmt = "Upload Error: {}", _0)]
    UploadError(String),
}

#[derive(Serialize)]
struct ErrorResponse {
    code: i32,
    message: String,
}

impl ResponseError for ApiError {
    fn error_response(&self) -> HttpResponse {
        let (status, code, message) = match self {
            ApiError::InternalServerError => (
                actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                500,
                "Internal Server Error".to_string(),
            ),
            ApiError::BadRequest(msg) => (
                actix_web::http::StatusCode::BAD_REQUEST,
                400,
                msg.clone(),
            ),
            ApiError::NotFound(msg) => (
                actix_web::http::StatusCode::NOT_FOUND,
                404,
                msg.clone(),
            ),
            ApiError::UploadError(msg) => (
                actix_web::http::StatusCode::BAD_REQUEST,
                400,
                msg.clone(),
            ),
        };

        HttpResponse::build(status)
            .content_type("application/json; charset=utf-8")
            .append_header(("X-API-Version", "v1"))
            .json(ErrorResponse {
                code,
                message,
            })
    }
}
