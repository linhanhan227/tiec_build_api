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
use std::sync::atomic::{AtomicBool, Ordering};

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
                fail_task(&data, task_id, format!("Failed to create source dir: {}", e), TaskStatus::CompilationFailed).await;
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
                    fail_task(&data, task_id, "Failed to extract project".into(), TaskStatus::CompilationFailed).await;
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
        let asset_err = match data.ensure_assets_extracted(false) {
            Ok(_) => None,
            Err(e) => Some(e.to_string()),
        };
        if let Some(err_msg) = asset_err {
            fail_task(&data, task_id, format!("Failed to prepare assets: {}", err_msg), TaskStatus::CompilationFailed).await;
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
                fail_task(&data, task_id, format!("Unsupported OS/arch: {}/{}", os, arch), TaskStatus::CompilationFailed).await;
                stop_lease(&data, &task_id, &lease_handle).await;
                continue;
            }
        };

        if !binary_path.exists() {
            fail_task(&data, task_id, format!("Compiler binary not found: {}", binary_path.display()), TaskStatus::CompilationFailed).await;
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
                fail_task(&data, task_id, format!("Failed to spawn builder: {}", e), TaskStatus::CompilationFailed).await;
                stop_lease(&data, &task_id, &lease_handle).await;
                continue;
            }
        };

        // Stream output
        let stdout = child.stdout.take().expect("Failed to open stdout");
        let stderr = child.stderr.take().expect("Failed to open stderr");
        
        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let error_seen = Arc::new(AtomicBool::new(false));
        let last_error = Arc::new(tokio::sync::Mutex::new(None::<String>));
        let success_seen = Arc::new(AtomicBool::new(false));

        // 5. Monitor execution
        // We need to concurrently read logs and wait for exit
        let _data_clone = data.clone();

        let success_flag = success_seen.clone();
        let _log_task = tokio::spawn(async move {
            while let Ok(Some(line)) = stdout_reader.next_line().await {
               log::info!("[Task {} Stdout]: {}", task_id, line);
               // Some compiler versions may exit non-zero even if it reports success-with-warnings.
               if line.contains("编译成功") {
                    success_flag.store(true, Ordering::Relaxed);
               }
               // Parse line for progress if possible
            }
        });

        let err_flag = error_seen.clone();
        let err_msg = last_error.clone();
        let success_flag2 = success_seen.clone();
        let _err_task = tokio::spawn(async move {
            while let Ok(Some(line)) = stderr_reader.next_line().await {
                log::error!("[Task {} Stderr]: {}", task_id, line);
                // Some builds may print success messages to stderr as well.
                if line.contains("编译成功") {
                    success_flag2.store(true, Ordering::Relaxed);
                }
                if line.contains("ERROR") || line.contains("错误") {
                    err_flag.store(true, Ordering::Relaxed);
                    let mut guard = err_msg.lock().await;
                    *guard = Some(line.clone());
                }
            }
        });

        // Use timeout
        match tokio::time::timeout(std::time::Duration::from_secs(task_timeout), child.wait()).await {
            Ok(Ok(status)) => {
                if error_seen.load(Ordering::Relaxed) {
                    let err_msg = last_error
                        .lock()
                        .await
                        .clone()
                        .unwrap_or_else(|| "Compiler error log detected".to_string());
                    fail_task(&data, task_id, err_msg, TaskStatus::CompilationFailed).await;
                } else if status.success() || success_seen.load(Ordering::Relaxed) {
                    // Treat as success if logs contain "编译成功" (e.g. success with warnings but non-zero exit)
                    
                    // Run gradlew assembleRelease explicitly
                    update_progress(&data, task_id, 80, "Packaging APK (gradlew)...").await;
                    let gradle_cmd = if cfg!(windows) { "gradlew.bat" } else { "gradlew" };
                    let gradle_dir = format!("{}/build", project_dir);
                    let mut gradle_process = Command::new(format!("./{}", gradle_cmd));
                    gradle_process.current_dir(&gradle_dir);
                    gradle_process.arg("assembleRelease");
                    
                    // Make sure gradlew is executable on Unix
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let gradlew_path = std::path::Path::new(&gradle_dir).join("gradlew");
                        if gradlew_path.exists() {
                            if let Ok(metadata) = std::fs::metadata(&gradle_path) {
                                let mut perms = metadata.permissions();
                                perms.set_mode(0o755);
                                let _ = std::fs::set_permissions(&gradle_path, perms);
                            }
                        }
                    }

                    match gradle_process.output().await {
                         Ok(output) => {
                            if !output.status.success() {
                                let stderr = String::from_utf8_lossy(&output.stderr);
                                log::warn!("gradlew assembleRelease failed: {}", stderr);
                                // Continue to check for APK anyway, in case it was built despite error
                            } else {
                                log::info!("gradlew assembleRelease executed successfully");
                            }
                         },
                         Err(e) => {
                             log::warn!("Failed to execute gradlew: {}", e);
                         }
                    }

                    let apk_path = find_apk_file(&build_output_dir).await;
                    if let Some(apk_file) = apk_path {
                        update_progress(&data, task_id, 100, "Build successful").await;
                        if let Some(mut task) = data.tasks.get_mut(&task_id) {
                            task.output_path = Some(apk_file);
                        }
                        update_task_status(&data, &task_id, TaskStatus::Success, 100, Some("Build successful".into()), None).await;
                        log::info!("Task {} completed successfully", task_id);
                    } else {
                        // Debugging: List files in build_output_dir to help diagnose
                        let mut files_found = String::new();
                        let mut queue = VecDeque::new();
                        queue.push_back(std::path::PathBuf::from(&build_output_dir));
                        let mut count = 0;
                        
                        while let Some(d) = queue.pop_front() {
                            if let Ok(mut entries) = tokio::fs::read_dir(&d).await {
                                while let Ok(Some(e)) = entries.next_entry().await {
                                    if e.path().is_dir() {
                                        queue.push_back(e.path());
                                    } else {
                                        if count < 20 {
                                            files_found.push_str(&format!("{}, ", e.path().display()));
                                        }
                                        count += 1;
                                    }
                                }
                            }
                            if count >= 20 { break; }
                        }
                        if count >= 20 { files_found.push_str("..."); }
                        
                        let err_msg = format!("Build command succeeded but APK file not found in {}. Found: [{}]", build_output_dir, files_found);
                        log::error!("Task {} failed: {}", task_id, err_msg);
                        fail_task(&data, task_id, err_msg, TaskStatus::CompilationFailed).await;
                    }
                } else {
                    fail_task(&data, task_id, format!("Build failed with exit code: {}", status), TaskStatus::CompilationFailed).await;
                }
            },
            Ok(Err(e)) => {
                if error_seen.load(Ordering::Relaxed) {
                    let err_msg = last_error
                        .lock()
                        .await
                        .clone()
                        .unwrap_or_else(|| "Compiler error log detected".to_string());
                    fail_task(&data, task_id, err_msg, TaskStatus::CompilationFailed).await;
                } else {
                    fail_task(&data, task_id, format!("Wait error: {}", e), TaskStatus::UnknownError).await;
                }
            },
            Err(_) => {
                let _ = child.kill().await;
                if error_seen.load(Ordering::Relaxed) {
                    let err_msg = last_error
                        .lock()
                        .await
                        .clone()
                        .unwrap_or_else(|| "Compiler error log detected".to_string());
                    fail_task(&data, task_id, err_msg, TaskStatus::CompilationFailed).await;
                } else {
                    fail_task(&data, task_id, "Build timed out".into(), TaskStatus::Timeout).await;
                }
            }
        }

        stop_lease(&data, &task_id, &lease_handle).await;
        
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
        // Retry logic:
        // - Only retry if status is Timeout (transient error).
        // - All other errors (CompilationFailed, UnknownError) are treated as fatal and not retryable.
        let is_timeout = matches!(final_status, TaskStatus::Timeout);
        let should_retry = is_timeout && task.retry_count < task.max_retries;

        if should_retry {
            // Increment retry count
            task.retry_count += 1;
            let backoff_secs = compute_retry_delay_seconds(task.retry_count);
            let next_run_at = Utc::now() + chrono::Duration::seconds(backoff_secs);
            
            let retry_msg = format!("Retry {}/{}: scheduled in {}s. Error: {}", 
                                    task.retry_count, task.max_retries, backoff_secs, error);
            
            // Update in-memory state
            task.status = TaskStatus::Queued;
            task.progress = 0; // Reset progress
            task.current_step = Some(format!("Waiting for retry (backoff {}s)", backoff_secs));
            
            // Don't overwrite `error` with retry message if we want to see the original error in logs? 
            // Better to prepend/append. But `error` field in Task is usually "last error".
            // However, user said "already reported error but still retries". 
            // They might mean the API response shows "Error: ..." but status is something else?
            // If status is Queued, the user sees "Queued" but with an error message?
            // Let's explicitly set error to indicate it's a transient failure.
            task.error = Some(retry_msg.clone());
            task.updated_at = Utc::now();
            
            log::info!("Task {} scheduled for retry (attempt {}/{}) at {} (in {}s)", 
                       task_id, task.retry_count, task.max_retries, next_run_at, backoff_secs);

            // Update database (schedules retry and clears lease)
            if let Err(e) = data.database.schedule_retry(&task_id, next_run_at, &error).await {
                log::error!("Failed to schedule retry for task {}: {}", task_id, e);
            }
            
            // Also update the task definition in DB to persist retry_count and such
            // IMPORTANT: We need to make sure we don't overwrite the schedule_retry updates (status/next_run_at)
            // update_task updates status too. It uses `task.status` which we set to Queued above.
            // update_task DOES NOT update `next_run_at`.
            // So calling update_task AFTER schedule_retry is fine, provided `next_run_at` is not touched by update_task.
            // `update_task` implementation: updates status, progress, ..., retry_count. Correct.
            if let Err(e) = data.database.update_task(&task).await {
                log::error!("Failed to update task retry state for {}: {}", task_id, e);
            }

        } else {
            // Max retries reached, mark as permanently failed
            log::error!("Task {} failed permanently after {} attempts: {}", task_id, task.max_retries, error);

            let is_timeout = matches!(final_status, TaskStatus::Timeout);
            // User requested specific error messages for API response
            let failed_msg = if is_timeout { "任务超时，请重新上传打包" } else { "任务失败" };
            
            task.status = final_status.clone();
            task.current_step = Some(failed_msg.into());
            task.error = Some(failed_msg.into()); 
            task.updated_at = Utc::now();
            
            // Update stats
            data.failed_tasks.fetch_add(1, Ordering::Relaxed);

            // Save final state to database
            if let Err(e) = data.database.update_task(&task).await {
                log::error!("Failed to update task failure in database: {}", e);
            }

            // Write failure event
            if let Err(e) = data.database.insert_task_event(&task_id, "failed", Some(final_status), task.error.as_deref(), None).await {
                log::warn!("Failed to write task failure event for {}: {}", task_id, e);
            }

            // Clear lease to allow cleanup or inspection (though lease strictly is for processing)
            if let Err(e) = data.database.clear_lease(&task_id).await {
                log::warn!("Failed to clear lease for task {}: {}", task_id, e);
            }

            // Cleanup related files immediately for failed tasks (keep record for query)
            let file_path = task.file_path.clone();
            let upload_dir = data.upload_dir.clone();
            let task_id_clone = task_id;
            
            tokio::spawn(async move {
                log::info!("Cleaning up files for failed/timeout task {}", task_id_clone);
                let path = std::path::Path::new(&file_path);
                
                // 1. Remove original uploaded file
                if path.exists() {
                    let res = if path.is_dir() {
                        tokio::fs::remove_dir_all(path).await
                    } else {
                        tokio::fs::remove_file(path).await
                    };
                    if let Err(e) = res {
                        log::warn!("Failed to delete task file {}: {}", path.display(), e);
                    }
                }

                // 2. Remove extracted directory
                if !path.is_dir() {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                         let extract_dir = format!("{}/{}_extracted", upload_dir, stem);
                         let extract_path = std::path::Path::new(&extract_dir);
                         if extract_path.exists() {
                             if let Err(e) = tokio::fs::remove_dir_all(extract_path).await {
                                 log::warn!("Failed to delete extracted dir {}: {}", extract_dir, e);
                             }
                         }
                    }
                }

            });
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
    
    let mut queue = VecDeque::new();
    queue.push_back(build_path.to_path_buf());

    while let Some(current_dir) = queue.pop_front() {
        let mut entries = match fs::read_dir(&current_dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                queue.push_back(path);
            } else if let Some(ext) = path.extension() {
                if ext == "apk" {
                    return path.to_str().map(|s| s.to_string());
                }
            }
        }
    }
    
    None
}

