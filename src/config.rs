pub struct Config {
    pub host: String,
    pub port: u16,
    pub upload_dir: String,
    pub database_path: String,
    pub queue_capacity: usize,
    pub worker_count: usize,
    pub task_timeout: u64,
    pub cleanup_interval: u64,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            host: std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: std::env::var("PORT").unwrap_or_else(|_| "8080".into()).parse().unwrap_or(8080),
            upload_dir: std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "./.tiec/uploads".into()),
            database_path: std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./.tiec/tasks.db".into()),
            queue_capacity: std::env::var("QUEUE_CAPACITY")
                .unwrap_or_else(|_| "100".into())
                .parse()
                .unwrap_or(100),
            worker_count: std::env::var("WORKER_COUNT")
                .unwrap_or_else(|_| "1".into())
                .parse()
                .unwrap_or(1),
            task_timeout: std::env::var("TASK_TIMEOUT")
                .unwrap_or_else(|_| "900".into())
                .parse()
                .unwrap_or(900),
            cleanup_interval: std::env::var("CLEANUP_INTERVAL")
                .unwrap_or_else(|_| "3600".into())
                .parse()
                .unwrap_or(3600),
        }
    }
}
