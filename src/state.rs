use crate::models::Task;
use crate::database::Database;
use crate::assets::ASSETS;
use crate::utils::copy_dir_all;
use tokio::sync::mpsc;
use uuid::Uuid;

pub type TaskSender = mpsc::Sender<Uuid>;

use std::sync::atomic::AtomicU64;

pub struct AppState {
    pub tasks: dashmap::DashMap<Uuid, Task>, // Keep in-memory cache for performance
    pub upload_dir: String,
    pub tiecc_dir: String,
    pub stdlib_dir: String,
    pub queue_capacity: usize, // Maximum queue size
    pub database: Database,
    // Statistics (cached, updated from DB periodically)
    pub total_tasks: AtomicU64,
    pub completed_tasks: AtomicU64,
    pub failed_tasks: AtomicU64,
    #[allow(dead_code)]
    pub active_workers: AtomicU64,
    // Worker senders for load balancing
    #[allow(dead_code)]
    pub worker_senders: Vec<TaskSender>,
    pub max_retries: u8,
}

impl AppState {
    pub fn new(upload_dir: String, tiecc_dir: String, stdlib_dir: String, queue_capacity: usize, database: Database, max_retries: u8) -> Self {
        AppState {
            tasks: dashmap::DashMap::new(),
            upload_dir,
            tiecc_dir,
            stdlib_dir,
            queue_capacity,
            database,
            max_retries,
            total_tasks: AtomicU64::new(0),
            completed_tasks: AtomicU64::new(0),
            failed_tasks: AtomicU64::new(0),
            active_workers: AtomicU64::new(0),
            worker_senders: vec![], // Empty vector since we no longer use individual senders
        }
    }

    pub async fn load_tasks_from_db(&self) -> Result<(), Box<dyn std::error::Error>> {
        let db_tasks = self.database.get_all_tasks().await?;
        for task in db_tasks {
            self.tasks.insert(task.task_id, task);
        }

        // Update statistics
        let (total, completed, failed, _) = self.database.get_task_stats().await?;
        self.total_tasks.store(total, std::sync::atomic::Ordering::Relaxed);
        self.completed_tasks.store(completed, std::sync::atomic::Ordering::Relaxed);
        self.failed_tasks.store(failed, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }

    pub async fn save_task_to_db(&self, task: &Task) -> Result<(), Box<dyn std::error::Error>> {
        self.database.insert_task(task).await?;
        Ok(())
    }

    pub async fn enqueue_task(&self, task_id: Uuid, max_queue_size: usize) {
        if let Some(mut task) = self.tasks.get_mut(&task_id) {
            task.status = crate::models::TaskStatus::Queued;
            task.current_step = Some("Queued".into());
            task.updated_at = chrono::Utc::now();
        }

        self.enforce_queue_capacity(max_queue_size).await;

        if let Err(e) = self.database.enqueue_task(&task_id).await {
            log::error!("Failed to enqueue task {}: {}", task_id, e);
        }
    }

    pub async fn enforce_queue_capacity(&self, max_queue_size: usize) {
        let mut queued_count = match self.database.count_queued_tasks().await {
            Ok(count) => count as usize,
            Err(e) => {
                log::error!("Failed to count queued tasks: {}", e);
                return;
            }
        };

        if queued_count >= max_queue_size {
            let overflow = (queued_count + 1).saturating_sub(max_queue_size).max(1);
            match self.database.get_oldest_queued_tasks(overflow as u64).await {
                Ok(old_tasks) => {
                    for old_task_id in old_tasks {
                        if let Some(mut task) = self.tasks.get_mut(&old_task_id) {
                            task.status = crate::models::TaskStatus::Cancelled;
                            task.error = Some("Cancelled due to queue overflow".into());
                            task.updated_at = chrono::Utc::now();
                            log::warn!("Cancelled task {} due to queue overflow", old_task_id);
                        }
                        if let Err(e) = self.database.cancel_task(&old_task_id, "Cancelled due to queue overflow").await {
                            log::error!("Failed to cancel task {}: {}", old_task_id, e);
                        }
                        queued_count = queued_count.saturating_sub(1);
                    }
                }
                Err(e) => {
                    log::error!("Failed to fetch queued tasks for overflow handling: {}", e);
                }
            }
        }
    }

    pub fn ensure_assets_extracted(&self) -> Result<(), Box<dyn std::error::Error>> {
        // .tiec is the root for runtime assets
        let tiecc_path = std::path::Path::new(&self.tiecc_dir);
        let stdlib_path = std::path::Path::new(&self.stdlib_dir);
        let root_dir = tiecc_path.parent().unwrap_or(std::path::Path::new("./.tiec"));

        // Only initialize on first run or when .tiec is incomplete
        if !tiecc_path.exists() || !stdlib_path.exists() {
            log::info!("Assets missing or incomplete. Initializing embedded assets to {:?}", root_dir);
            std::fs::create_dir_all(root_dir)?;

            for asset in ASSETS {
                let target = root_dir.join(asset.path);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&target, asset.data)?;

                #[cfg(unix)]
                if let Some(mode) = asset.unix_mode {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = std::fs::metadata(&target)?.permissions();
                    perms.set_mode(mode);
                    std::fs::set_permissions(&target, perms)?;
                }
            }

            log::info!("Assets initialized successfully");
        } else {
            log::info!("Assets already exist at {:?}, skipping initialization", root_dir);
        }

        // Ensure Android base library exists in .tiec/安卓基本库
        let android_src = stdlib_path.join("android");
        let android_dst = root_dir.join("安卓基本库");
        if !android_dst.exists() {
            if android_src.exists() {
                if let Err(e) = copy_dir_all(&android_src, &android_dst) {
                    log::warn!("Failed to initialize 安卓基本库: {}", e);
                }
            } else {
                log::warn!("Android stdlib directory not found: {}", android_src.display());
            }
        }
        Ok(())
    }
}
