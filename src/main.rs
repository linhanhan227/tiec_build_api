mod api;
mod assets;
mod config;
mod database;
mod error;
mod middleware;
mod models;
mod state;
mod utils;
mod worker;

use actix_cors::Cors;
use actix_web::{web, App, HttpServer, middleware::Logger};
use actix_governor::{Governor, GovernorConfigBuilder};
use config::Config;
use state::AppState;
use database::Database;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;
use std::sync::{Arc};
use std::collections::VecDeque;
#[cfg(not(windows))]
use std::net::IpAddr;
#[cfg(windows)]
use std::net::IpAddr;
use uuid::Uuid;
use tokio::sync::Mutex;

#[derive(OpenApi)]
#[openapi(
    paths(
        api::upload::upload_file,
        api::build::create_build,
        api::build::get_build_status,
        api::build::get_build_events,
        api::download::download_build,
        api::health::health_check
    ),
    components(
        schemas(models::Task, models::TaskStatus, models::TaskEvent, models::BuildRequest, models::BuildResponse, models::UploadResponse)
    ),
    tags(
        (name = "tie-api", description = "TieCloud Build API")
    )
)]
struct ApiDoc;

#[cfg(not(windows))]
fn get_local_ips() -> Vec<String> {
    let mut ips = Vec::new();
    if let Ok(ifaces) = get_if_addrs::get_if_addrs() {
        for iface in ifaces {
            if iface.is_loopback() {
                continue;
            }
            match iface.ip() {
                IpAddr::V4(ip) => ips.push(ip.to_string()),
                _ => {}
            }
        }
    }
    ips.sort();
    ips.dedup();
    ips
}

#[cfg(windows)]
fn get_local_ips() -> Vec<String> {
    let mut ips = Vec::new();
    if let Ok(adapters) = ipconfig::get_adapters() {
        for adapter in adapters {
            for ip in adapter.ip_addresses() {
                if ip.is_loopback() {
                    continue;
                }
                if let IpAddr::V4(ipv4) = ip {
                    ips.push(ipv4.to_string());
                }
            }
        }
    }
    ips.sort();
    ips.dedup();
    ips
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "error");
    env_logger::init();

    let config = Config::from_env();

    // Set paths for embedded resources (extracted on first use)测试构建上传
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let tiec_root = exe_dir.join(".tiec");
    let tiecc_dir = tiec_root.join("tiecc");
    let stdlib_dir = tiec_root.join("stdlib");

    let upload_dir = if std::path::Path::new(&config.upload_dir).is_absolute() {
        config.upload_dir.clone()
    } else {
        exe_dir.join(&config.upload_dir).to_string_lossy().to_string()
    };
    let database_path = if std::path::Path::new(&config.database_path).is_absolute() {
        config.database_path.clone()
    } else {
        exe_dir.join(&config.database_path).to_string_lossy().to_string()
    };

    // Create upload directory
    std::fs::create_dir_all(&upload_dir)?;

    // Initialize database
    let database = Database::new(&database_path).expect("Failed to initialize database");

    // Create shared task queue
    let task_queue: Arc<Mutex<VecDeque<Uuid>>> = Arc::new(Mutex::new(VecDeque::new()));
    let task_queue_clone = task_queue.clone();
    
    // Initialize state with assets zip
    let state = web::Data::new(AppState::new(
        upload_dir.clone(),
        tiecc_dir.to_string_lossy().to_string(),
        stdlib_dir.to_string_lossy().to_string(),
        config.queue_capacity,
        database,
    ));
    
    // Ensure assets are extracted on startup
    if let Err(e) = state.ensure_assets_extracted() {
        log::error!("Failed to extract assets: {}", e);
        // We might want to panic here if assets are critical
        panic!("Failed to extract assets: {}", e);
    }

    let state_clone = state.clone();

    // Load existing tasks from database
    state.load_tasks_from_db().await.expect("Failed to load tasks from database");

    // Start multiple workers
    for i in 0..config.worker_count {
        let state_clone = state.clone();
        let task_queue_worker = task_queue_clone.clone();
        let task_timeout = config.task_timeout;
        tokio::spawn(async move {
            worker::run_worker(state_clone.into_inner(), task_queue_worker, i, task_timeout).await;
        });
    }

    // Start cleanup task
    let cleanup_state = state.clone();
    let cleanup_interval = config.cleanup_interval;
    let task_timeout = config.task_timeout;
    tokio::spawn(async move {
        worker::cleanup_task(cleanup_state.into_inner(), cleanup_interval, task_timeout).await;
    });

    let local_ips = get_local_ips();
    if local_ips.is_empty() {
        log::info!("Local IP not detected; server will bind at http://{}:{}", config.host, config.port);
        println!("Default bind: http://0.0.0.0:8080");
        println!("Server running at http://{}:{}", config.host, config.port);
    } else {
        log::info!("Local IP(s): {}", local_ips.join(", "));
        log::info!("Starting server at http://{}:{}", config.host, config.port);
        println!("Default bind: http://0.0.0.0:8080");
        for ip in &local_ips {
            println!("Server running at http://{}:{}", ip, config.port);
        }
    }

    // Configure IP rate limiter: 120 requests per second per IP, ban for 7 days on violation
    let ip_limiter = middleware::IpRateLimiter::new(120, 7).await.expect("Failed to initialize IP rate limiter");

    // Configure rate limiting: 120 requests per second per IP
    let governor_conf = GovernorConfigBuilder::default()
        .per_second(120)
        .burst_size(120)
        .finish()
        .unwrap();

    let hourly_limiter = middleware::HourlyIpLimiter::new(config.hourly_ip_limit);

    HttpServer::new(move || {
        let cors = Cors::permissive(); // For development
        let ip_limiter_clone = ip_limiter.clone();

        App::new()
            .wrap(ip_limiter_clone)
            .wrap(hourly_limiter.clone())
            .wrap(Governor::new(&governor_conf))
            .wrap(cors)
            .wrap(Logger::default())
            .app_data(state_clone.clone())
            .service(api::health::health_check)
            .service(
                web::scope("/api/v1")
                    .service(api::upload::upload_file)
                    .service(api::build::create_build)
                    .service(api::build::get_build_status)
                        .service(api::build::get_build_events)
                    .service(api::download::download_build)
            )
            .service(
                SwaggerUi::new("/swagger-ui/{_:.*}")
                    .url("/api-docs/openapi.json", ApiDoc::openapi()),
            )
    })
    .bind(format!("{}:{}", config.host, config.port))?
    .run()
    .await
}
