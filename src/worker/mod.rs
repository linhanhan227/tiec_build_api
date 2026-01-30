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
            TaskStatus::Failed => {
                data.failed_tasks.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }

        // Save to database
        if let Err(e) = data.database.update_task(&task).await {
            log::error!("Failed to update task in database: {}", e);
        }
    }
}

pub async fn run_worker(data: Arc<AppState>, task_queue: Arc<tokio::sync::Mutex<VecDeque<Uuid>>>, worker_id: usize, task_timeout: u64) {
    log::info!("Worker {} started", worker_id);
    loop {
        // Try to get a task from the queue
        let task_id = {
            let mut queue = task_queue.lock().await;
            queue.pop_front()
        };

        let task_id = match task_id {
            Some(id) => id,
            None => {
                // No task available, wait a bit and try again
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };
// ...
// (I need to be careful not to delete the existing code, I'll use append or smart replace. 
//  Since I cannot easily append, I will add the function at the END of the file or replacing the end of file)
        log::info!("Processing task: {}", task_id);
        
        // 1. Update status to Processing
        update_task_status(&data, &task_id, TaskStatus::Processing, 5, Some("Initializing environment".into()), None).await;

        // 2. Prepare paths
        let (file_path, _user_id) = if let Some(task) = data.tasks.get(&task_id) {
            (task.file_path.clone(), task.user_id.clone())
        } else {
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
                fail_task(&data, task_id, format!("Failed to create source dir: {}", e)).await;
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
                    fail_task(&data, task_id, "Failed to extract project".into()).await;
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
            fail_task(&data, task_id, format!("Failed to prepare assets: {}", err_msg)).await;
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
                fail_task(&data, task_id, format!("Unsupported OS/arch: {}/{}", os, arch)).await;
                continue;
            }
        };

        if !binary_path.exists() {
            fail_task(&data, task_id, format!("Compiler binary not found: {}", binary_path.display())).await;
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
                fail_task(&data, task_id, format!("Failed to spawn builder: {}", e)).await;
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
                         fail_task(&data, task_id, "Build command succeeded but APK file not found".into()).await;
                    }
                } else {
                     fail_task(&data, task_id, format!("Build failed with exit code: {}", status)).await;
                }
            },
            Ok(Err(e)) => {
                 fail_task(&data, task_id, format!("Wait error: {}", e)).await;
            },
            Err(_) => {
                let _ = child.kill().await;
                fail_task(&data, task_id, "Build timed out".into()).await;
            }
        }
        
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

async fn fail_task(data: &Arc<AppState>, task_id: Uuid, error: String) {
    if let Some(mut task) = data.tasks.get_mut(&task_id) {
        if task.retry_count < task.max_retries {
            // Retry the task
            task.retry_count += 1;
            task.error = Some(format!("Previous attempt failed: {}. Retrying...", error));
            task.updated_at = Utc::now();

            // Save retry state to database
            if let Err(e) = data.database.update_task(&task).await {
                log::error!("Failed to update retry state in database: {}", e);
            }

            log::info!("Retrying task {} (attempt {}/{})", task_id, task.retry_count, task.max_retries);

            // Re-queue the task
            data.enqueue_task(task_id, data.queue_capacity).await;
            update_task_status(data, &task_id, TaskStatus::Queued, 0, Some(format!("Retrying (attempt {}/{})", task.retry_count, task.max_retries)), task.error.clone()).await;
        } else {
            // Max retries reached, mark as failed
            update_task_status(data, &task_id, TaskStatus::Failed, task.progress, Some("Build failed".into()), Some(format!("Task failed after {} attempts. Last error: {}", task.max_retries, error))).await;
            log::error!("Task {} failed permanently after {} attempts: {}", task_id, task.max_retries, error);
        }
    }
}

pub async fn cleanup_task(data: Arc<AppState>, cleanup_interval: u64) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(cleanup_interval)); 
    loop {
        interval.tick().await;
        log::info!("Running cleanup task");

        // Cleanup expired tasks from memory (older than 24 hours)
        let mut tasks_to_remove = Vec::new();
        for task in data.tasks.iter() {
            if task.created_at + chrono::Duration::hours(24) < chrono::Utc::now() {
                tasks_to_remove.push(*task.key());
            }
        }
        for task_id in tasks_to_remove {
            data.tasks.remove(&task_id);
            log::info!("Removed expired task from memory: {}", task_id);
        }
        
        let mut active_files = std::collections::HashSet::new();
        // Collect files currently in use by valid tasks (Queued, Processing, Success)
        for task in data.tasks.iter() {
             active_files.insert(task.value().file_path.clone());
        }

        // Iterate directory
        if let Ok(entries) = std::fs::read_dir(&data.upload_dir) {
            for entry in entries.flatten() {
                 if let Ok(path) = entry.path().canonicalize() {
                     if let Some(path_str) = path.to_str() {
                         // Check if file is in use
                         if active_files.contains(path_str) {
                             continue; 
                         }
                         
                         // Check age
                         if let Ok(metadata) = entry.metadata() {
                             if let Ok(modified) = metadata.modified() {
                                 if let Ok(duration) = modified.elapsed() {
                                     if duration.as_secs() > 86400 { // 24 hours
                                         log::info!("Cleaning up old file: {}", path_str);
                                         let _ = std::fs::remove_file(path);
                                     }
                                 }
                             }
                         }
                     }
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

