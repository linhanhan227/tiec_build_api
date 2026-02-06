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

        // Use a unified structure to handle task execution and error propagation.
        // This avoids scattered fail_task/stop_lease calls.
        match execute_task(&data, task_id, task_timeout).await {
            Ok(_) => {
                log::info!("Task {} completed successfully execution flow", task_id);
            }
            Err((msg, status)) => {
                 log::error!("Task {} execution failed: {}", task_id, msg);
                 fail_task(&data, task_id, msg, status).await;
            }
        }

        stop_lease(&data, &task_id, &lease_handle).await;
    }
}

async fn execute_task(data: &Arc<AppState>, task_id: Uuid, task_timeout: u64) -> Result<(), (String, TaskStatus)> {
    // 1. Update status to Processing
    update_task_status(data, &task_id, TaskStatus::Processing, 5, Some("Initializing environment".into()), None).await;

    // 2. Prepare paths and unzip
    let (project_dir, file_path) = prepare_project_source(data, task_id).await?;

    // 3. Run binary (tiec construction)
    // Ensure assets are available
    if let Err(e) = data.ensure_assets_extracted(false) {
        return Err((format!("Failed to prepare assets: {}", e), TaskStatus::CompilationFailed));
    }
    
    let binary_path = get_compiler_binary_path(&data.tiecc_dir)
        .ok_or_else(|| ("Compiler binary not found or unsupported OS".to_string(), TaskStatus::CompilationFailed))?;
    
    update_progress(data, task_id, 20, "Starting build process...").await;

    let build_output_dir = format!("{}/build", project_dir);
    let app_config_root = format!("{}/project.json", project_dir);
    let app_config_in_build = format!("{}/build/project.json", project_dir);

    // Ensure app config at project root logic
    if !std::path::Path::new(&app_config_root).exists() && std::path::Path::new(&app_config_in_build).exists() {
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

    // Spawn and Monitor
    run_compiler_process(cmd, task_id, task_timeout).await
        .map_err(|e| (e, TaskStatus::CompilationFailed))?;

    // 4. Run Gradle Packaging
    update_progress(data, task_id, 80, "Packaging APK (release)...").await;
    if let Err(e) = run_gradle_build(&project_dir).await {
         // Gradle errors are non-fatal in the original code sense if an apk is somehow found?
         // But usually failure means no APK. The original code logged warning and continued to find_apk.
         // We'll mimic that structure by just logging but proceeding to check for APK.
         log::warn!("Gradle build step reported issues: {}", e);
    }
    
    // 5. Finalize Artifacts
    finalize_build_artifact(data, task_id, &project_dir, &file_path).await?;
    
    Ok(())
}

async fn prepare_project_source(data: &Arc<AppState>, task_id: Uuid) -> Result<(String, String), (String, TaskStatus)> {
    let (file_path, _file_id) = if let Some(task) = data.tasks.get(&task_id) {
        (task.file_path.clone(), task.file_id.clone())
    } else {
        return Err(("Task not found in memory".to_string(), TaskStatus::UnknownError));
    };

    let mut project_dir = file_path.clone();

    if !std::path::Path::new(&file_path).is_dir() {
        let file_stem = std::path::Path::new(&file_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let unzip_dir = format!("{}/{}", data.upload_dir, file_stem);

        if let Err(e) = fs::create_dir_all(&unzip_dir).await {
            return Err((format!("Failed to create source dir: {}", e), TaskStatus::CompilationFailed));
        }

        update_progress(data, task_id, 10, "Extracting project...").await;
        let file_path_clone = file_path.clone();
        let unzip_dir_clone = unzip_dir.clone();

        let unzip_result = web::block(move || -> Result<(), std::io::Error> {
            let file = std::fs::File::open(file_path_clone)?;
            let mut archive = zip::ZipArchive::new(file)?;
            use crate::utils::extract_zip_from_archive; 
            extract_zip_from_archive(&mut archive, &std::path::Path::new(&unzip_dir_clone))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
            Ok(())
        }).await;

        match unzip_result {
            Ok(Ok(_)) => {},
            _ => {
                return Err(("Failed to extract project".into(), TaskStatus::CompilationFailed));
            }
        }
        
        update_progress(data, task_id, 15, "Organizing project files...").await;
        project_dir = unzip_dir.clone();
    }
    
    Ok((project_dir, file_path))
}

fn get_compiler_binary_path(tiecc_dir: &str) -> Option<std::path::PathBuf> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let tiecc_root = std::path::Path::new(tiecc_dir);

    let binary_path = match (os, arch) {
        ("macos", _) => tiecc_root.join("macos/tiec"),
        ("linux", "x86_64") => tiecc_root.join("linux_x86_64/tiec"),
        ("linux", "aarch64") => tiecc_root.join("linux_arm64-v8a/tiec"),
        ("windows", "x86_64") => tiecc_root.join("win_x86_64/tiec.exe"),
        _ => return None,
    };
    
    if binary_path.exists() { Some(binary_path) } else { None }
}

async fn run_compiler_process(mut cmd: Command, task_id: Uuid, timeout: u64) -> Result<(), String> {
    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn builder: {}", e))?;
    
    let stdout = child.stdout.take().expect("Failed to open stdout");
    let stderr = child.stderr.take().expect("Failed to open stderr");
    
    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();

    let error_seen = Arc::new(AtomicBool::new(false));
    let last_error = Arc::new(tokio::sync::Mutex::new(None::<String>));
    let success_seen = Arc::new(AtomicBool::new(false));

    let success_flag = success_seen.clone();
    tokio::spawn(async move {
        while let Ok(Some(line)) = stdout_reader.next_line().await {
           log::info!("[Task {} Stdout]: {}", task_id, line);
           if line.contains("编译成功") {
                success_flag.store(true, Ordering::Relaxed);
           }
        }
    });

    let err_flag = error_seen.clone();
    let err_msg = last_error.clone();
    let success_flag2 = success_seen.clone();
    tokio::spawn(async move {
        while let Ok(Some(line)) = stderr_reader.next_line().await {
            log::error!("[Task {} Stderr]: {}", task_id, line);
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

    match tokio::time::timeout(std::time::Duration::from_secs(timeout), child.wait()).await {
        Ok(Ok(status)) => {
            if error_seen.load(Ordering::Relaxed) {
                let msg = last_error.lock().await.clone().unwrap_or_else(|| "Compiler error log detected".to_string());
                return Err(msg);
            }
            if status.success() || success_seen.load(Ordering::Relaxed) {
                return Ok(());
            }
            Err(format!("Build failed with exit code: {}", status))
        },
        Ok(Err(e)) => Err(format!("Wait error: {}", e)),
        Err(_) => {
            let _ = child.kill().await;
            Err("Build timed out".to_string())
        }
    }
}

async fn run_gradle_build(project_dir: &str) -> Result<(), String> {
    let gradle_dir_str = format!("{}/build", project_dir);
    let gradle_dir = std::path::Path::new(&gradle_dir_str);
    let gradlew_name = if cfg!(windows) { "gradlew.bat" } else { "gradlew" };
    let local_gradlew_build = gradle_dir.join(gradlew_name);

    // 强制使用项目自带的 gradlew
    if !local_gradlew_build.exists() {
        return Err(format!("未在构建目录检测到 {}，请确保项目生成了 Gradle Wrapper", gradlew_name));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&local_gradlew_build) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&local_gradlew_build, perms);
        }
    }

    let cmd_to_run = if cfg!(windows) { gradlew_name.to_string() } else { format!("./{}", gradlew_name) };
    let work_dir = gradle_dir;

    let mut gradle_process = Command::new(&cmd_to_run);
    gradle_process.current_dir(work_dir);
    gradle_process.arg("assembleRelease");
    gradle_process.arg("--no-daemon");

    let output = gradle_process.output().await.map_err(|e| format!("Failed to execute gradle: {}", e))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Gradle build failed: {}", stderr));
    } else {
        log::info!("Gradle build success");
        Ok(())
    }
}

async fn finalize_build_artifact(data: &Arc<AppState>, task_id: Uuid, project_dir: &str, file_path_cleanup: &str) -> Result<(), (String, TaskStatus)> {
    update_progress(data, task_id, 99, "Finalizing package...").await;
    
    let build_output_dir = format!("{}/build", project_dir);
    let gradle_apk_dir = format!("{}/build/outputs/apk", project_dir);
    
    let mut apk_path = find_apk_file(&gradle_apk_dir).await;
    if apk_path.is_none() {
        apk_path = find_apk_file(&build_output_dir).await;
    }

    if let Some(apk_file) = apk_path {
        let apk_path_path = std::path::Path::new(&apk_file);
        let new_filename = format!("app-{}.apk", task_id);
        let dest_path_buf = std::path::Path::new(&data.upload_dir).join(&new_filename);
        let dest_path = dest_path_buf.to_string_lossy().to_string();

        let mut move_success = false;
        if let Err(e) = tokio::fs::rename(apk_path_path, &dest_path_buf).await {
            log::warn!("Failed to move APK via rename, trying copy: {}", e);
            if let Ok(_) = tokio::fs::copy(apk_path_path, &dest_path_buf).await {
                move_success = true;
                let _ = tokio::fs::remove_file(apk_path_path).await;
            }
        } else {
            move_success = true;
        }

        if move_success {
            if let Some(mut task) = data.tasks.get_mut(&task_id) {
                task.output_path = Some(dest_path.clone());
            }
            update_task_status(data, &task_id, TaskStatus::Success, 100, Some("Build successful".into()), None).await;
            log::info!("Task {} completed successfully. APK stored at {}", task_id, dest_path);

            let project_dir_cleanup = project_dir.to_string();
            let zip_file_cleanup = file_path_cleanup.to_string();
            
            tokio::spawn(async move {
                if let Err(e) = tokio::fs::remove_dir_all(&project_dir_cleanup).await {
                    log::debug!("Cleanup: Failed to remove project dir: {}", e);
                }
                let zip_path = std::path::Path::new(&zip_file_cleanup);
                if zip_path.is_file() {
                    let _ = tokio::fs::remove_file(zip_path).await;
                }
            });
            Ok(())
        } else {
            Err(("Failed to save final APK file".to_string(), TaskStatus::UnknownError))
        }
    } else {
        // Simple file listing for debug
        Err((format!("Build command succeeded but APK file not found in {}", build_output_dir), TaskStatus::CompilationFailed))
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
        // Retry logic removed - always fail immediately on error
        {
            // Max retries reached, mark as permanently failed
            log::error!("Task {} failed permanently after {} attempts: {}", task_id, task.max_retries, error);

            let failed_msg = match final_status {
                TaskStatus::Timeout => "任务超时，请重新上传打包",
                TaskStatus::CompilationFailed => "编译失败，请检查代码",
                TaskStatus::Cancelled => "任务已取消",
                _ => "任务失败"
            };
            
            task.status = final_status.clone();
            task.current_step = Some(failed_msg.to_string());
            
            // Combine friendly message with detailed error
            if !error.is_empty() && error != failed_msg {
                task.error = Some(format!("{}: {}", failed_msg, error)); 
            } else {
                task.error = Some(failed_msg.to_string());
            }

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
                
                // Keep stem for extracted dir check even if file assumes deletion
                let stem = path.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string());

                // 1. Remove original uploaded file/folder
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
                // We check for extracted dir existence independently of source file status
                if let Some(s) = stem {
                     let extract_dir = format!("{}/{}", upload_dir, s);
                     let extract_path = std::path::Path::new(&extract_dir);
                     if extract_path.exists() {
                         if let Err(e) = tokio::fs::remove_dir_all(extract_path).await {
                             log::warn!("Failed to delete extracted dir {}: {}", extract_dir, e);
                         }
                     }
                }

            });
        }
    }
}

// fn compute_retry_delay_seconds(retry_count: u8) -> i64 {
//     let base = 5_i64;
//     let exp = 2_i64.saturating_pow(retry_count as u32);
//     let max_delay = 300_i64;
//     let jitter = (Utc::now().timestamp_subsec_millis() as i64) % 10;
//     (base.saturating_mul(exp)).min(max_delay) + jitter
// }

pub async fn cleanup_task(data: Arc<AppState>, cleanup_interval: u64, task_timeout: u64, retention_secs: u64) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(cleanup_interval));
    loop {
        interval.tick().await;
        log::info!("正在运行清理任务（文件保留 {} 秒，记录保留 {} 秒）", retention_secs, retention_secs * 3);

        let now = chrono::Utc::now();
        // File cleanup cutoffs
        let completed_cutoff = now - chrono::Duration::seconds(retention_secs as i64);
        let failed_cutoff = now - chrono::Duration::seconds(retention_secs as i64);
        let expired_cutoff = now - chrono::Duration::seconds(retention_secs as i64);
        
        // Record cleanup cutoff (hardcoded to 3x retention for now)
        let record_cutoff = now - chrono::Duration::seconds((retention_secs * 4) as i64);
        
        let _timeout_cutoff = now - chrono::Duration::seconds(task_timeout as i64);

        let tasks = match data.database.get_all_tasks().await {
            Ok(list) => list,
            Err(e) => {
                log::error!("Failed to load tasks for cleanup: {}", e);
                continue;
            }
        };

        let mut cleaned = 0usize;
        let mut _file_cleaned = 0usize;
        let mut _timeout_marked = 0usize;
        for task in tasks {
            // 如果任务正在运行，则跳过清理检查
            if task.status == crate::models::TaskStatus::Processing {
                continue;
            }

            // 1) Logic for handling timed-out processing tasks is removed as per request to skip running tasks.

            // 2) 彻底清理：如果超过记录保留时间，删除数据库记录和内存缓存
            if task.updated_at < record_cutoff {
                cleanup_task_files(&data.upload_dir, &task);
                data.tasks.remove(&task.task_id);
                if let Err(e) = data.database.delete_task(&task.task_id).await {
                    log::error!("Failed to delete task {}: {}", task.task_id, e);
                } else {
                    cleaned += 1;
                    log::info!("已彻底清理古老任务（记录+文件）：{}", task.task_id);
                }
                continue;
            }

            // 3) 文件清理：如果超过文件保留时间，仅清理物理文件，保留记录
            let should_cleanup_files = match task.status {
                crate::models::TaskStatus::Success => task.updated_at < completed_cutoff,
                crate::models::TaskStatus::UnknownError => task.updated_at < failed_cutoff,
                crate::models::TaskStatus::Timeout => task.updated_at < failed_cutoff,
                crate::models::TaskStatus::CompilationFailed => task.updated_at < failed_cutoff,
                crate::models::TaskStatus::Cancelled => task.updated_at < expired_cutoff,
                crate::models::TaskStatus::Queued => task.updated_at < expired_cutoff,
                crate::models::TaskStatus::Processing => false,
            };

            if should_cleanup_files {
                 // Check if files exist before trying to clean to avoid Spamming logs or useless ops
                 let file_path = std::path::Path::new(&task.file_path);
                 let output_path = task.output_path.as_deref().map(std::path::Path::new);
                 let has_files = file_path.exists() || output_path.map(|p| p.exists()).unwrap_or(false);
                 
                 if has_files {
                     cleanup_task_files(&data.upload_dir, &task);
                     _file_cleaned += 1;
                     log::info!("已清理过期任务文件（保留记录）：{}", task.task_id);
                 }
            }
        }

        if cleaned > 0 || _timeout_marked > 0 {
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
    let stem = file_path.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string());
    
    // 1. Delete source file (TSP file or directory)
    // We try to delete even if exists() returns false to handle race conditions or partial deletions gracefully
    // But since remove methods fail if not exists, we can just check existence.
    // Importantly, we capture is_dir before deletion if possible, but if it doesn't exist we guess based on extension or just skip logic that depends on type.
    if file_path.exists() {
        if file_path.is_dir() {
            if let Err(e) = std::fs::remove_dir_all(file_path) {
                log::warn!("Failed to remove task dir {}: {}", file_path.display(), e);
            }
        } else {
             if let Err(e) = std::fs::remove_file(file_path) {
                log::warn!("Failed to remove task file {}: {}", file_path.display(), e);
            }
        }
    } else {
        // Log that file was missing, might have been cleaned up already
        log::debug!("Task file {} already removed or missing", file_path.display());
    }

    // 2. Output artifacts
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

    // 3. Extracted directory
    // If we have a stem, we should check for an extracted directory regardless of whether the source file is currently present.
    // Source file might have been deleted in step 1 or previously.
    if let Some(stem_str) = stem {
         let extracted_dir = std::path::Path::new(upload_dir).join(format!("{}", stem_str));
         if extracted_dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&extracted_dir) {
                log::warn!("Failed to remove extracted dir {}: {}", extracted_dir.display(), e);
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

