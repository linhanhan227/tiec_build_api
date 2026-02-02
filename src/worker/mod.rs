use crate::state::AppState;
use crate::models::{TaskStatus};
use actix_web::web;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use std::sync::Arc;
use tokio::fs;
use uuid::Uuid;
use chrono::Utc;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;

async fn update_task_status(data: &Arc<AppState>, task_id: &Uuid, status: TaskStatus, progress: u8, current_step: Option<String>, error: Option<String>) {
    if let Some(mut task) = data.tasks.get_mut(task_id) {
        let status_clone = status.clone();
        let message = error.clone().or_else(|| current_step.clone());
        task.status = status;
        task.progress = progress;
        task.current_step = current_step;
        task.error = error;
        task.updated_at = Utc::now();

        // Update statistics
        match task.status {
            TaskStatus::Success => {
                data.completed_tasks.fetch_add(1, Ordering::Relaxed);
            }
            TaskStatus::UnknownError | TaskStatus::Timeout | TaskStatus::CompilationFailed => {
                data.failed_tasks.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }

        // Save to database
        if let Err(e) = data.database.update_task(&task).await {
            log::error!("Failed to update task in database: {}", e);
        }

        if let Err(e) = data.database.insert_task_event(task_id, "status_change", Some(status_clone), message.as_deref(), None).await {
            log::warn!("Failed to write task event for {}: {}", task_id, e);
        }

        if task.status != TaskStatus::Processing {
            if let Err(e) = data.database.clear_lease(task_id).await {
                log::warn!("Failed to clear lease for task {}: {}", task_id, e);
            }
        }
    }
}

async fn stop_lease(data: &Arc<AppState>, task_id: &Uuid, lease_handle: &tokio::task::JoinHandle<()>) {
    lease_handle.abort();
    if let Err(e) = data.database.clear_lease(task_id).await {
        log::warn!("Failed to clear lease for task {}: {}", task_id, e);
    }
}

pub async fn run_worker(data: Arc<AppState>, _task_queue: Arc<tokio::sync::Mutex<VecDeque<Uuid>>>, worker_id: usize, task_timeout: u64) {
    log::info!("Worker {} started", worker_id);
    let worker_id_str = format!("worker-{}", worker_id);
    let lease_refresh_secs = std::cmp::max(5, task_timeout / 2);
    loop {
        let task_id = match data.database.lease_next_task(&worker_id_str, task_timeout + 60).await {
            Ok(Some(id)) => id,
            Ok(None) => {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                continue;
            }
            Err(e) => {
                log::error!("Failed to lease task: {}", e);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                continue;
            }
        };
// ...
// (I need to be careful not to delete the existing code, I'll use append or smart replace. 
//  Since I cannot easily append, I will add the function at the END of the file or replacing the end of file)
        log::info!("Processing task: {}", task_id);

        let lease_task_id = task_id;
        let lease_data = data.clone();
        let lease_worker = worker_id_str.clone();
        let lease_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(lease_refresh_secs));
            loop {
                interval.tick().await;
                if let Err(e) = lease_data.database.refresh_lease(&lease_task_id, &lease_worker, task_timeout + 60).await {
                    log::warn!("Failed to refresh lease for task {}: {}", lease_task_id, e);
                }
            }
        });
        
        // 1. Update status to Processing
        update_task_status(&data, &task_id, TaskStatus::Processing, 5, Some("Initializing environment".into()), None).await;

        // 2. Prepare paths
        let (file_path, _file_id, _user_id) = if let Some(task) = data.tasks.get(&task_id) {
            (task.file_path.clone(), task.file_id.clone(), task.user_id.clone())
        } else {
            stop_lease(&data, &task_id, &lease_handle).await;
            continue;
        };


        let mut project_dir = file_path.clone();

        if !std::path::Path::new(&file_path).is_dir() {
            let file_stem = std::path::Path::new(&file_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let unzip_dir = format!("{}/{}_extracted", data.upload_dir, file_stem);

            if let Err(e) = fs::create_dir_all(&unzip_dir).await {
                fail_task(&data, task_id, format!("Failed to create source dir: {}", e), TaskStatus::UnknownError).await;
                stop_lease(&data, &task_id, &lease_handle).await;
                continue;
            }

            // 3. Unzip
            update_progress(&data, task_id, 10, "Extracting project...").await;
            let file_path_clone = file_path.clone();
            let unzip_dir_clone = unzip_dir.clone();

            let unzip_result = web::block(move || -> Result<(), std::io::Error> {
                let file = std::fs::File::open(file_path_clone)?;
                let mut archive = zip::ZipArchive::new(file)?;
                use crate::utils::extract_zip_from_archive; // Use the safe extraction
                extract_zip_from_archive(&mut archive, &std::path::Path::new(&unzip_dir_clone))
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
                Ok(())
            }).await;

            match unzip_result {
                Ok(Ok(_)) => {},
                _ => {
                    fail_task(&data, task_id, "Failed to extract project".into(), TaskStatus::UnknownError).await;
                    stop_lease(&data, &task_id, &lease_handle).await;
                    continue;
                }
            }

            // 3.5. Use extracted directory as project directory
            update_progress(&data, task_id, 15, "Organizing project files...").await;
            project_dir = unzip_dir.clone();
        }

        // 4. Run binary
        // Ensure assets are available
        let asset_err = match data.ensure_assets_extracted() {
            Ok(_) => None,
            Err(e) => Some(e.to_string()),
        };
        if let Some(err_msg) = asset_err {
            fail_task(&data, task_id, format!("Failed to prepare assets: {}", err_msg), TaskStatus::UnknownError).await;
            stop_lease(&data, &task_id, &lease_handle).await;
            continue;
        }

        // Detect OS and architecture to choose binary path under .tiec/tiecc
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let tiecc_root = std::path::Path::new(&data.tiecc_dir);

        let binary_path = match (os, arch) {
            ("macos", _) => tiecc_root.join("macos/tiec"),
            ("linux", "x86_64") => tiecc_root.join("linux_x86_64/tiec"),
            ("linux", "aarch64") => tiecc_root.join("linux_arm64-v8a/tiec"),
            ("windows", "x86_64") => tiecc_root.join("win_x86_64/tiec.exe"),
            _ => {
                fail_task(&data, task_id, format!("Unsupported OS/arch: {}/{}", os, arch), TaskStatus::UnknownError).await;
                stop_lease(&data, &task_id, &lease_handle).await;
                continue;
            }
        };

        if !binary_path.exists() {
            fail_task(&data, task_id, format!("Compiler binary not found: {}", binary_path.display()), TaskStatus::UnknownError).await;
            stop_lease(&data, &task_id, &lease_handle).await;
            continue;
        }

        update_progress(&data, task_id, 20, "Starting build process...").await;

        // Construct command
        let build_output_dir = format!("{}/build", project_dir);
        let app_config_root = format!("{}/project.json", project_dir);
        let app_config_in_build = format!("{}/build/project.json", project_dir);

        // Ensure app config at project root (remove build path usage)
        if !std::path::Path::new(&app_config_root).exists()
            && std::path::Path::new(&app_config_in_build).exists()
        {
            let _ = fs::copy(&app_config_in_build, &app_config_root).await;
        }

        let mut cmd = Command::new(&binary_path);
        cmd.current_dir(&project_dir);
        cmd.arg("-o").arg(&build_output_dir);
        cmd.arg("--platform").arg("android");
        cmd.arg("--android.gradle");
        cmd.arg("--android.app.config").arg(&app_config_root);
        cmd.arg("--release");
        cmd.arg("--log-level").arg("error");
        cmd.arg("--dir").arg(&project_dir);
        
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Spawn the process
        let child_result = cmd.spawn();
        
        let mut child = match child_result {
            Ok(c) => c,
            Err(e) => {
                fail_task(&data, task_id, format!("Failed to spawn builder: {}", e), TaskStatus::UnknownError).await;
                stop_lease(&data, &task_id, &lease_handle).await;
                continue;
            }
        };

        // Stream output
        let stdout = child.stdout.take().expect("Failed to open stdout");
        let stderr = child.stderr.take().expect("Failed to open stderr");
        
        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        // 5. Monitor execution
        // We need to concurrently read logs and wait for exit
        let _data_clone = data.clone();
        
        let _log_task = tokio::spawn(async move {
            while let Ok(Some(line)) = stdout_reader.next_line().await {
               log::info!("[Task {} Stdout]: {}", task_id, line);
               // Parse line for progress if possible
            }
        });
        
        let _err_task = tokio::spawn(async move {
             while let Ok(Some(line)) = stderr_reader.next_line().await {
               log::error!("[Task {} Stderr]: {}", task_id, line);
            }
        });

        // Use timeout
        match tokio::time::timeout(std::time::Duration::from_secs(task_timeout), child.wait()).await {
            Ok(Ok(status)) => {
                if status.success() {
                    // Find APK file in build directory
                    let apk_path = find_apk_file(&build_output_dir).await;
                    if let Some(apk_file) = apk_path {
                        update_progress(&data, task_id, 100, "Build successful").await;
                        if let Some(mut task) = data.tasks.get_mut(&task_id) {
                            task.output_path = Some(apk_file);
                        }
                        update_task_status(&data, &task_id, TaskStatus::Success, 100, Some("Build successful".into()), None).await;
                        log::info!("Task {} completed successfully", task_id);
                    } else {
                         fail_task(&data, task_id, "Build command succeeded but APK file not found".into(), TaskStatus::CompilationFailed).await;
                    }
                } else {
                     fail_task(&data, task_id, format!("Build failed with exit code: {}", status), TaskStatus::CompilationFailed).await;
                }
            },
            Ok(Err(e)) => {
                 fail_task(&data, task_id, format!("Wait error: {}", e), TaskStatus::UnknownError).await;
            },
            Err(_) => {
                let _ = child.kill().await;
                fail_task(&data, task_id, "Build timed out".into(), TaskStatus::Timeout).await;
            }
        }

        stop_lease(&data, &task_id, &lease_handle).await;
        
        // Clean up uploads/work dir if needed (user requirement 1 mentioned cleanup task, 
        // but here we might want to keep output? "output_dir" should persist for download)
        // Cleanup source and file_path can be done here or by cron.
        
        // Check "retention" policy.
        // For now, keep the APK.
    }
}

async fn update_progress(data: &Arc<AppState>, task_id: Uuid, progress: u8, step: &str) {
    if let Some(mut task) = data.tasks.get_mut(&task_id) {
        task.progress = progress;
        task.current_step = Some(step.into());
        task.updated_at = Utc::now();

        // Update in database
        if let Err(e) = data.database.update_task(&task).await {
            log::error!("Failed to update progress in database: {}", e);
        }
    }
}

async fn fail_task(data: &Arc<AppState>, task_id: Uuid, error: String, final_status: TaskStatus) {
    if let Some(mut task) = data.tasks.get_mut(&task_id) {
        if task.retry_count < task.max_retries {
            // Retry the task
            task.retry_count += 1;
            task.error = Some(format!("Previous attempt failed: {}. Retrying...", error));
            task.updated_at = Utc::now();

            let backoff_secs = compute_retry_delay_seconds(task.retry_count);
            let next_run_at = Utc::now() + chrono::Duration::seconds(backoff_secs);
                if let Err(e) = data.database.schedule_retry(&task_id, next_run_at, &error).await {
                log::error!("Failed to schedule retry in database: {}", e);
            }

            // Save retry state to database
            if let Err(e) = data.database.update_task(&task).await {
                log::error!("Failed to update retry state in database: {}", e);
            }

            log::info!(
                "Retrying task {} (attempt {}/{}), next_run_at={} ({}s)",
                task_id,
                task.retry_count,
                task.max_retries,
                next_run_at.to_rfc3339(),
                backoff_secs
            );

            let countdown_task_id = task_id;
            let countdown_secs = backoff_secs.max(1) as u64;
            let countdown_data = data.clone();
            tokio::spawn(async move {
                log::info!("倒计时开始：任务 {}，总计 {} 秒", countdown_task_id, countdown_secs);
                let mut remaining = countdown_secs;
                let step = if countdown_secs > 10 { 5 } else { 1 };
                while remaining > 0 {
                    log::info!("倒计时重试：任务 {} 剩余 {} 秒", countdown_task_id, remaining);
                    if let Err(e) = countdown_data
                        .database
                        .insert_task_event(
                            &countdown_task_id,
                            "retry_countdown",
                            Some(TaskStatus::Queued),
                            Some(&format!("倒计时重试：剩余 {} 秒", remaining)),
                            None,
                        )
                        .await
                    {
                        log::warn!("倒计时事件写入失败 {}: {}", countdown_task_id, e);
                    }
                    let sleep_for = std::cmp::min(step, remaining);
                    tokio::time::sleep(std::time::Duration::from_secs(sleep_for)).await;
                    remaining = remaining.saturating_sub(sleep_for);
                }
                log::info!("倒计时结束：任务 {}，等待重试调度", countdown_task_id);
                if let Err(e) = countdown_data
                    .database
                    .insert_task_event(
                        &countdown_task_id,
                        "retry_countdown_done",
                        Some(TaskStatus::Queued),
                        Some("倒计时结束，等待重试调度"),
                        None,
                    )
                    .await
                {
                    log::warn!("倒计时结束事件写入失败 {}: {}", countdown_task_id, e);
                }
            });

            // Re-queue the task with backoff
            // Skipped enforce_queue_capacity to avoid potential deadlock and prioritize retries
            
            // INLINED update_task_status LOGIC TO AVOID DEADLOCK
            task.status = TaskStatus::Queued;
            task.progress = 0;
            task.current_step = Some(format!("Retrying in {}s (attempt {}/{})", backoff_secs, task.retry_count, task.max_retries));
            task.updated_at = Utc::now();
            
            // Save to database
            if let Err(e) = data.database.update_task(&task).await {
                log::error!("Failed to update task in database: {}", e);
            }
            
            // Write event
            if let Err(e) = data.database.insert_task_event(&task_id, "status_change", Some(TaskStatus::Queued), task.error.as_deref(), None).await {
                log::warn!("Failed to write task event for {}: {}", task_id, e);
            }

            // Clear lease
            if let Err(e) = data.database.clear_lease(&task_id).await {
                log::warn!("Failed to clear lease for task {}: {}", task_id, e);
            }

        } else {
            // Max retries reached, mark as failed
            // INLINED update_task_status LOGIC TO AVOID DEADLOCK
            let is_timeout = match final_status {
                TaskStatus::Timeout => true,
                _ => false
            };
            
            task.status = final_status.clone();
            
            let failed_msg = if is_timeout { "Build failed (Timeout)" } else { "Build failed" };
            task.current_step = Some(failed_msg.into());
            task.error = Some(format!("Task failed after {} attempts. Last error: {}", task.max_retries, error));
            task.updated_at = Utc::now();
            
            // Update stats
            data.failed_tasks.fetch_add(1, Ordering::Relaxed);

            // Save to database
            if let Err(e) = data.database.update_task(&task).await {
                log::error!("Failed to update task in database: {}", e);
            }

            // Write event
            if let Err(e) = data.database.insert_task_event(&task_id, "status_change", Some(final_status), task.error.as_deref(), None).await {
                log::warn!("Failed to write task event for {}: {}", task_id, e);
            }

            // Clear lease
            if let Err(e) = data.database.clear_lease(&task_id).await {
                log::warn!("Failed to clear lease for task {}: {}", task_id, e);
            }

            log::error!("Task {} failed permanently after {} attempts: {}", task_id, task.max_retries, error);
        }
    }
}

fn compute_retry_delay_seconds(retry_count: u8) -> i64 {
    let base = 5_i64;
    let exp = 2_i64.saturating_pow(retry_count as u32);
    let max_delay = 300_i64;
    let jitter = (Utc::now().timestamp_subsec_millis() as i64) % 10;
    (base.saturating_mul(exp)).min(max_delay) + jitter
}

pub async fn cleanup_task(data: Arc<AppState>, cleanup_interval: u64, task_timeout: u64, retention_secs: u64) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(cleanup_interval));
    loop {
        interval.tick().await;
        log::info!("正在运行清理任务（保留窗口 {} 秒）", retention_secs);

        let now = chrono::Utc::now();
        let completed_cutoff = now - chrono::Duration::seconds(retention_secs as i64);
        let failed_cutoff = now - chrono::Duration::seconds(retention_secs as i64);
        let expired_cutoff = now - chrono::Duration::seconds(retention_secs as i64);
        let timeout_cutoff = now - chrono::Duration::seconds(task_timeout as i64);

        let tasks = match data.database.get_all_tasks().await {
            Ok(list) => list,
            Err(e) => {
                log::error!("Failed to load tasks for cleanup: {}", e);
                continue;
            }
        };

        let mut cleaned = 0usize;
        for task in tasks {
            let should_cleanup = match task.status {
                crate::models::TaskStatus::Success => task.updated_at < completed_cutoff,
                crate::models::TaskStatus::UnknownError => task.updated_at < failed_cutoff,
                crate::models::TaskStatus::Timeout => task.updated_at < failed_cutoff,
                crate::models::TaskStatus::CompilationFailed => task.updated_at < failed_cutoff,
                crate::models::TaskStatus::Cancelled => task.updated_at < expired_cutoff,
                crate::models::TaskStatus::Queued => task.updated_at < expired_cutoff,
                crate::models::TaskStatus::Processing => task.updated_at < timeout_cutoff,
            };

            if !should_cleanup {
                continue;
            }

            cleanup_task_files(&data.upload_dir, &task);

            data.tasks.remove(&task.task_id);
            if let Err(e) = data.database.delete_task(&task.task_id).await {
                log::error!("Failed to delete task {}: {}", task.task_id, e);
                continue;
            }

            cleaned += 1;
            log::info!("已清理过期/超时任务：{}", task.task_id);
        }

        if cleaned > 0 {
            if let Ok((total, completed, failed, _)) = data.database.get_task_stats().await {
                data.total_tasks.store(total, std::sync::atomic::Ordering::Relaxed);
                data.completed_tasks.store(completed, std::sync::atomic::Ordering::Relaxed);
                data.failed_tasks.store(failed, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}

fn cleanup_task_files(upload_dir: &str, task: &crate::models::Task) {
    let file_path = std::path::Path::new(&task.file_path);
    if file_path.exists() {
        if file_path.is_dir() {
            if let Err(e) = std::fs::remove_dir_all(file_path) {
                log::warn!("Failed to remove task dir {}: {}", file_path.display(), e);
            }
        } else if let Err(e) = std::fs::remove_file(file_path) {
            log::warn!("Failed to remove task file {}: {}", file_path.display(), e);
        }
    }

    if let Some(output_path) = task.output_path.as_ref() {
        let output = std::path::Path::new(output_path);
        if output.exists() {
            if output.is_dir() {
                let _ = std::fs::remove_dir_all(output);
            } else {
                let _ = std::fs::remove_file(output);
            }
        }
    }

    if file_path.is_file() {
        if let Some(stem) = file_path.file_stem().and_then(|s| s.to_str()) {
            let extracted_dir = std::path::Path::new(upload_dir).join(format!("{}_extracted", stem));
            if extracted_dir.exists() {
                if let Err(e) = std::fs::remove_dir_all(&extracted_dir) {
                    log::warn!("Failed to remove extracted dir {}: {}", extracted_dir.display(), e);
                }
            }
        }
    }
}

async fn find_apk_file(build_dir: &str) -> Option<String> {
    use tokio::fs;
    
    let build_path = std::path::Path::new(build_dir);
    if !build_path.exists() {
        return None;
    }
    
    let mut entries = match fs::read_dir(build_path).await {
        Ok(e) => e,
        Err(_) => return None,
    };
    
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if let Some(ext) = path.extension() {
            if ext == "apk" {
                return path.to_str().map(|s| s.to_string());
            }
        }
    }
    
    None
}

