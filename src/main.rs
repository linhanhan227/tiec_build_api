mod api;
mod assets;
mod config;
mod database;
mod error;
use serde::Deserialize;
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::VecDeque;
#[cfg(not(windows))]
use std::net::IpAddr;
#[cfg(windows)]
use std::net::IpAddr;
use uuid::Uuid;
use tokio::sync::Mutex;

#[derive(Deserialize)]
struct ApiResponse<T> {
    code: i32,
    data: T,
}

#[derive(Deserialize)]
struct UploadData {
    file_id: String,
}

#[derive(Deserialize)]
struct BuildData {
    task_id: String,
    status: models::TaskStatus,
}

#[derive(Deserialize, Default)]
struct BuildTestConfig {
    base_url: Option<String>,
    file_path: Option<String>,
    file_paths: Option<Vec<String>>,
    interval_ms: Option<u64>,
    task_timeout: Option<u64>,
    queue_capacity: Option<usize>,
    cleanup_interval: Option<u64>,
    cleanup_retention_secs: Option<u64>,
    hourly_ip_limit: Option<u32>,
    max_retries: Option<u8>,
}

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

fn build_test_file_paths_with_config(config: Option<&BuildTestConfig>) -> Vec<String> {
    let mut paths = Vec::new();

    if let Some(cfg) = config {
        if let Some(path) = &cfg.file_path {
            if !path.trim().is_empty() {
                paths.push(path.trim().to_string());
            }
        }

        if let Some(list) = &cfg.file_paths {
            for item in list {
                let value = item.trim();
                if !value.is_empty() {
                    paths.push(value.to_string());
                }
            }
        }
    }

    if let Ok(path) = std::env::var("BUILD_TEST_FILE_PATH") {
        if !path.trim().is_empty() {
            paths.push(path.trim().to_string());
        }
    }

    if let Ok(list) = std::env::var("BUILD_TEST_FILE_PATHS") {
        for item in list.split(',') {
            let value = item.trim();
            if !value.is_empty() {
                paths.push(value.to_string());
            }
        }
    }

    paths
}

fn load_build_test_config() -> Option<BuildTestConfig> {
    let config_path = std::env::var("BUILD_TEST_CONFIG").unwrap_or_else(|_| "test.json".into());
    let path = std::path::Path::new(&config_path);

    let mut candidates = Vec::new();
    if path.is_absolute() {
        candidates.push(path.to_path_buf());
    } else {
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join(path));
        }

        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                candidates.push(exe_dir.join(path));
                if let Some(parent) = exe_dir.parent() {
                    candidates.push(parent.join(path));
                    if let Some(grandparent) = parent.parent() {
                        candidates.push(grandparent.join(path));
                    }
                }
            }
        }
    }

    let resolved_path = candidates
        .into_iter()
        .find(|p| p.exists());

    let resolved_path = match resolved_path {
        Some(p) => p,
        None => {
            log::error!("build-test：未找到 test.json（可设置 BUILD_TEST_CONFIG 指定路径）");
            return None;
        }
    };

    let content = match std::fs::read_to_string(&resolved_path) {
        Ok(v) => v,
        Err(e) => {
            log::error!("build-test：读取 test.json 失败 {}: {}", resolved_path.display(), e);
            return None;
        }
    };

    match serde_json::from_str::<BuildTestConfig>(&content) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            log::error!("build-test：解析 test.json 失败 {}: {}", resolved_path.display(), e);
            None
        }
    }
}

async fn wait_for_health(client: &reqwest::Client, base_url: &str) -> bool {
    let url = format!("{}/health", base_url.trim_end_matches('/'));
    for _ in 0..30 {
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    false
}

async fn run_build_test_via_api(base_url: String, file_paths: Vec<String>, interval_ms: u64) {
    let client = reqwest::Client::new();
    if !wait_for_health(&client, &base_url).await {
        log::error!("build-test：健康检查失败，服务未就绪");
        return;
    }

    log::info!("build-test：服务就绪，开始按 API 文档执行测试");

    let mut uploaded_ids = Vec::new();
    for path in &file_paths {
        let form = match reqwest::multipart::Form::new().file("file", path).await {
            Ok(f) => f,
            Err(e) => {
                log::error!("build-test：无法读取测试文件 {}: {}", path, e);
                continue;
            }
        };

        let url = format!("{}/api/v1/upload", base_url.trim_end_matches('/'));
        let resp = match client.post(&url).multipart(form).send().await {
            Ok(r) => r,
            Err(e) => {
                log::error!("build-test：上传失败 {}: {}", path, e);
                continue;
            }
        };

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            log::error!("build-test：上传响应异常 {}: {}", path, body);
            continue;
        }

        match resp.json::<ApiResponse<UploadData>>().await {
            Ok(payload) => {
                if payload.code != 200 {
                    log::error!("build-test：上传返回非 200: {}", payload.code);
                    continue;
                }
                log::info!("build-test：上传成功 {} -> file_id={}", path, payload.data.file_id);
                uploaded_ids.push(payload.data.file_id);
            }
            Err(e) => {
                log::error!("build-test：解析上传响应失败 {}: {}", path, e);
            }
        }
    }

    for (idx, file_id) in uploaded_ids.iter().enumerate() {
        let url = format!("{}/api/v1/build", base_url.trim_end_matches('/'));
        let resp = match client
            .post(&url)
            .json(&serde_json::json!({"file_id": file_id}))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                log::error!("build-test：创建构建任务失败（第 {} 个）: {}", idx + 1, e);
                continue;
            }
        };

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            log::error!("build-test：构建响应异常（第 {} 个）: {}", idx + 1, body);
            continue;
        }

        match resp.json::<ApiResponse<BuildData>>().await {
            Ok(payload) => {
                log::info!("build-test：已创建构建任务 {}（状态：{:?}）", payload.data.task_id, payload.data.status);
                
                let mon_client = client.clone();
                let mon_base = base_url.clone();
                let mon_task = payload.data.task_id.clone();
                tokio::spawn(async move {
                    monitor_build_task(mon_client, mon_base, mon_task).await;
                });
            }
            Err(e) => {
                log::error!("build-test：解析构建响应失败（第 {} 个）: {}", idx + 1, e);
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
    }
}

async fn monitor_build_task(client: reqwest::Client, base_url: String, task_id: String) {
    let status_url = format!("{}/api/v1/build/{}/status", base_url.trim_end_matches('/'), task_id);
    let events_url = format!("{}/api/v1/build/{}/events", base_url.trim_end_matches('/'), task_id);
    let download_url = format!("{}/api/v1/build/{}/download", base_url.trim_end_matches('/'), task_id);

    log::info!("build-test：监控任务 {} (API: status/events/download)", task_id);

    let running = Arc::new(AtomicBool::new(true));
    
    // 1. Independent Event Polling Task
    let events_client = client.clone();
    let events_task_id = task_id.clone();
    let events_running = running.clone();
    
    let _event_handle = tokio::spawn(async move {
        while events_running.load(Ordering::Relaxed) {
            match events_client.get(&events_url).send().await {
                Ok(resp) => {
                     if let Ok(payload) = resp.json::<ApiResponse<Vec<models::TaskEvent>>>().await {
                         if let Some(last) = payload.data.last() {
                             log::info!("build-test：任务 {} 事件 -> [{}] {}", events_task_id, last.event_type, last.message.as_deref().unwrap_or(""));
                         }
                     }
                },
                Err(_) => {}
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });

    let mut final_status = models::TaskStatus::Queued;
    
    // 2. Status Polling Loop (Main monitor flow)
    for _ in 0..600 {
        let resp = match client.get(&status_url).send().await {
            Ok(r) => r,
            Err(e) => {
                log::warn!("build-test：获取任务 {} 状态失败: {}", task_id, e);
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                continue;
            }
        };
        
        if let Ok(payload) = resp.json::<ApiResponse<models::Task>>().await {
             match payload.data.status {
                models::TaskStatus::Success => {
                    log::info!("build-test：任务 {} 构建成功！", task_id);
                    final_status = models::TaskStatus::Success;
                    break;
                },
                models::TaskStatus::CompilationFailed | models::TaskStatus::Timeout | models::TaskStatus::Cancelled | models::TaskStatus::UnknownError => {
                    log::error!("build-test：任务 {} 结束但失败，状态: {:?}", task_id, payload.data.status);
                    final_status = payload.data.status;
                    break;
                },
                _ => {}
             }
        }
        
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }

    // Stop event polling
    running.store(false, Ordering::Relaxed);
    // We don't necessarily need to wait for event_handle to finish, it will stop on next tick.

    // 3. Independent Download Task (if success)
    if final_status == models::TaskStatus::Success {
        tokio::spawn(async move {
            log::info!("build-test：正在下载 APK (任务 {}) ...", task_id);
            match client.get(&download_url).send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        let len = resp.content_length().unwrap_or(0);
                        log::info!("build-test：下载测试通过！任务: {}, 大小: {} bytes", task_id, len);
                    } else {
                        log::error!("build-test：下载请求返回异常状态 {}: {}", task_id, resp.status());
                    }
                },
                Err(e) => {
                    log::error!("build-test：下载请求失败 {}: {}", task_id, e);
                }
            }
        });
        // We spawn download detached so this monitor function can return (freeing up the logical 'monitor' slot if we track them)
        // Check if event_handle needs to be awaited? No, detached is fine.
    }
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

fn configure_build_test_environment(config: &Option<BuildTestConfig>) {
    std::env::set_var("RUST_LOG", "info");

    if let Some(cfg) = config {
        if let Some(value) = cfg.task_timeout {
            std::env::set_var("TASK_TIMEOUT", value.to_string());
        }
        if let Some(value) = cfg.queue_capacity {
            std::env::set_var("QUEUE_CAPACITY", value.to_string());
        }
        if let Some(value) = cfg.cleanup_interval {
            std::env::set_var("CLEANUP_INTERVAL", value.to_string());
        }
        if let Some(value) = cfg.cleanup_retention_secs {
            std::env::set_var("CLEANUP_RETENTION_SECS", value.to_string());
        }
        if let Some(value) = cfg.hourly_ip_limit {
            std::env::set_var("HOURLY_IP_LIMIT", value.to_string());
        }
        if let Some(value) = cfg.max_retries {
            std::env::set_var("MAX_RETRIES", value.to_string());
        }
    }

    if let Ok(v) = std::env::var("BUILD_TEST_TASK_TIMEOUT") {
        std::env::set_var("TASK_TIMEOUT", v);
    }
    if let Ok(v) = std::env::var("BUILD_TEST_QUEUE_CAPACITY") {
        std::env::set_var("QUEUE_CAPACITY", v);
    }
    
    // Set defaults if not provided in config
    if config.as_ref().and_then(|cfg| cfg.task_timeout).is_none() {
        std::env::set_var("TASK_TIMEOUT", "1800");
    }
    if config.as_ref().and_then(|cfg| cfg.cleanup_interval).is_none() {
        std::env::set_var("CLEANUP_INTERVAL", "86400");
    }
    if config.as_ref().and_then(|cfg| cfg.cleanup_retention_secs).is_none() {
        std::env::set_var("CLEANUP_RETENTION_SECS", "86400");
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let build_test = args.iter().any(|arg| arg == "--build-test" || arg == "build-test");

    // Check for --port argument
    if let Some(index) = args.iter().position(|arg| arg == "--port") {
        if let Some(port) = args.get(index + 1) {
            std::env::set_var("PORT", port);
        }
    }

    let build_test_config = if build_test {
        load_build_test_config()
    } else {
        None
    };

    if build_test {
        configure_build_test_environment(&build_test_config);
    } else {
        std::env::set_var("RUST_LOG", "error");
    }
    env_logger::init();

    let config = Config::from_env();

    if build_test {
        // Remove variables that might conflict or were temporary
        std::env::remove_var("TASK_TIMEOUT");
        std::env::remove_var("QUEUE_CAPACITY");
        std::env::remove_var("CLEANUP_INTERVAL");
        std::env::remove_var("CLEANUP_RETENTION_SECS");
        std::env::remove_var("HOURLY_IP_LIMIT");
    }

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
        config.max_retries,
    ));
    
    // Ensure assets are extracted on startup
    if let Err(e) = state.ensure_assets_extracted(build_test) {
        log::error!("Failed to extract assets: {}", e);
        // We might want to panic here if assets are critical
        panic!("Failed to extract assets: {}", e);
    }

    let state_clone = state.clone();

    // Load existing tasks from database
    state.load_tasks_from_db().await.expect("Failed to load tasks from database");

    if build_test {
        let base_url = build_test_config
            .as_ref()
            .and_then(|cfg| cfg.base_url.clone())
            .or_else(|| std::env::var("BUILD_TEST_BASE_URL").ok())
            .unwrap_or_else(|| format!("http://127.0.0.1:{}", config.port));
        let test_interval_ms = build_test_config
            .as_ref()
            .and_then(|cfg| cfg.interval_ms)
            .or_else(|| std::env::var("BUILD_TEST_INTERVAL_MS").ok().and_then(|v| v.parse::<u64>().ok()))
            .unwrap_or(200);

        let file_paths = build_test_file_paths_with_config(build_test_config.as_ref());
        if file_paths.is_empty() {
            log::error!("build-test：缺少 test.json 配置或 BUILD_TEST_FILE_PATH/BUILD_TEST_FILE_PATHS");
        } else {
            let hourly_limit = config.hourly_ip_limit as usize;
            let expected_requests = file_paths.len().saturating_mul(2);
            if expected_requests > hourly_limit {
                log::warn!("build-test：预计请求数={} 超过小时限额={}，可能触发限流", expected_requests, hourly_limit);
            }

            tokio::spawn(async move {
                run_build_test_via_api(base_url, file_paths, test_interval_ms).await;
            });
        }
    }

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
    let cleanup_retention_secs = config.cleanup_retention_secs;
    let task_timeout = config.task_timeout;
    tokio::spawn(async move {
        worker::cleanup_task(cleanup_state.into_inner(), cleanup_interval, task_timeout, cleanup_retention_secs).await;
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

    let server = HttpServer::new(move || {
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
    .run();

    let server_handle = server.handle();
    tokio::select! {
        result = server => result,
        _ = tokio::signal::ctrl_c() => {
            log::info!("收到 Ctrl+C，正在停止服务...");
            server_handle.stop(true).await;
            Ok(())
        }
    }
}
