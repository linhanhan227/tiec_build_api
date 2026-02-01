use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpResponse,
};
use futures::future::LocalBoxFuture;
use std::{
    collections::HashMap,
    future::{ready, Ready},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    path::PathBuf,
};
use tokio::sync::RwLock;
use tokio::fs;

#[derive(Clone)]
pub struct IpRateLimiter {
    request_counts: Arc<RwLock<HashMap<String, (u32, Instant)>>>,
    ban_file_path: PathBuf,
    rate_limit: u32, // requests per second
    ban_duration: Duration,
}

impl IpRateLimiter {
    pub async fn new(rate_limit_per_second: u32, ban_duration_days: u64) -> Result<Self, Box<dyn std::error::Error>> {
        let ban_file_path = PathBuf::from("./.tiec/ip_ban.txt");

        // Ensure the directory exists
        if let Some(parent) = ban_file_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // Create the file if it doesn't exist
        if !ban_file_path.exists() {
            fs::File::create(&ban_file_path).await?;
        }

        Ok(Self {
            request_counts: Arc::new(RwLock::new(HashMap::new())),
            ban_file_path,
            rate_limit: rate_limit_per_second,
            ban_duration: Duration::from_secs(ban_duration_days * 24 * 60 * 60),
        })
    }



    pub async fn is_blocked(&self, ip: &str) -> Result<bool, Box<dyn std::error::Error>> {
        if !self.ban_file_path.exists() {
            return Ok(false);
        }

        let content = fs::read_to_string(&self.ban_file_path).await?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Parse line format: IP:timestamp
            if let Some((banned_ip, timestamp_str)) = line.split_once(':') {
                if banned_ip == ip {
                    if let Ok(timestamp) = timestamp_str.parse::<u64>() {
                        let elapsed = now.saturating_sub(timestamp);
                        if elapsed < (self.ban_duration.as_secs()) {
                            return Ok(true);
                        }
                    }
                }
            } else {
                // Legacy format: just IP address
                if line == ip {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    pub async fn add_banned_ip(&self, ip: &str) -> Result<(), Box<dyn std::error::Error>> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        // Read existing content
        let mut content = if self.ban_file_path.exists() {
            fs::read_to_string(&self.ban_file_path).await?
        } else {
            "# IP Ban List - Auto-generated\n# Format: IP_ADDRESS:TIMESTAMP\n\n".to_string()
        };

        // Check if IP is already banned
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let mut found = false;
        for line in &mut lines {
            if line.contains(&format!("{}:", ip)) || line == ip {
                *line = format!("{}:{}", ip, now);
                found = true;
                break;
            }
        }

        if !found {
            content.push_str(&format!("{}:{}\n", ip, now));
        } else {
            content = lines.join("\n") + "\n";
        }

        fs::write(&self.ban_file_path, content).await?;
        log::warn!("IP {} banned for {} days: exceeded {} requests per second", ip, self.ban_duration.as_secs() / (24 * 60 * 60), self.rate_limit);
        Ok(())
    }

    pub async fn cleanup_expired_bans(&self) -> Result<(), Box<dyn std::error::Error>> {
        if !self.ban_file_path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(&self.ban_file_path).await?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let mut new_content = String::new();
        let mut has_content = false;

        new_content.push_str("# IP Ban List - Auto-generated\n");
        new_content.push_str("# Format: IP_ADDRESS:TIMESTAMP\n");
        new_content.push_str(&format!("# Updated at: {}\n\n", now));

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((_ip, timestamp_str)) = line.split_once(':') {
                if let Ok(timestamp) = timestamp_str.parse::<u64>() {
                    let elapsed = now.saturating_sub(timestamp);
                    if elapsed < self.ban_duration.as_secs() {
                        new_content.push_str(line);
                        new_content.push('\n');
                        has_content = true;
                    }
                }
            }
        }

        if has_content {
            fs::write(&self.ban_file_path, new_content).await?;
        } else {
            // If no active bans, keep only header
            let header = "# IP Ban List - Auto-generated\n# Format: IP_ADDRESS:TIMESTAMP\n\n";
            fs::write(&self.ban_file_path, header).await?;
        }

        Ok(())
    }

    pub async fn check_and_update_rate(&self, ip: &str) -> bool {
        let mut request_counts = self.request_counts.write().await;
        let now = Instant::now();

        let (count, last_reset) = request_counts.entry(ip.to_string()).or_insert((0, now));

        // Reset counter if more than 1 second has passed
        if now.duration_since(*last_reset) >= Duration::from_secs(1) {
            *count = 1;
            *last_reset = now;
            return true; // Allow request
        }

        // Increment counter
        *count += 1;

        // Check if rate limit exceeded
        if *count > self.rate_limit {
            // Block the IP
            if let Err(e) = self.add_banned_ip(ip).await {
                log::error!("Failed to ban IP {}: {}", ip, e);
            }
            return false; // Block request
        }

        true // Allow request
    }

    // Cleanup expired blocks and old counters
    pub async fn cleanup(&self) {
        let mut request_counts = self.request_counts.write().await;
        let now = Instant::now();

        // Remove old counters (older than 10 seconds)
        request_counts.retain(|_, (_, last_reset)| {
            now.duration_since(*last_reset) < Duration::from_secs(10)
        });

        // Cleanup expired bans from file
        if let Err(e) = self.cleanup_expired_bans().await {
            log::error!("Failed to cleanup expired bans: {}", e);
        }
    }
}

pub struct IpRateLimiterMiddleware<S> {
    service: S,
    limiter: IpRateLimiter,
}

impl<S, B> Transform<S, ServiceRequest> for IpRateLimiter
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = IpRateLimiterMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(IpRateLimiterMiddleware {
            service,
            limiter: self.clone(),
        }))
    }
}

impl<S, B> Service<ServiceRequest> for IpRateLimiterMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let limiter = self.limiter.clone();
        let ip = req
            .connection_info()
            .peer_addr()
            .unwrap_or("unknown")
            .to_string();

        let fut = self.service.call(req);

        Box::pin(async move {
            // Check if IP is blocked
            if limiter.is_blocked(&ip).await.unwrap_or(false) {
                let _response = HttpResponse::Forbidden()
                    .json(serde_json::json!({
                        "error": "IP blocked due to rate limit violation",
                        "message": "Your IP has been temporarily blocked. Please try again later."
                    }));
                return Err(actix_web::error::ErrorForbidden(serde_json::json!({
                    "error": "IP blocked due to rate limit violation",
                    "message": "Your IP has been temporarily blocked. Please try again later."
                })));
            }

            // Check rate limit
            if !limiter.check_and_update_rate(&ip).await {
                let _response = HttpResponse::TooManyRequests()
                    .json(serde_json::json!({
                        "error": "Rate limit exceeded",
                        "message": "Too many requests. Your IP has been blocked for 7 days."
                    }));
                return Err(actix_web::error::ErrorTooManyRequests(serde_json::json!({
                    "error": "Rate limit exceeded",
                    "message": "Too many requests. Your IP has been blocked for 7 days."
                })));
            }

            let res = fut.await?;

            // Periodic cleanup (1% chance)
            if rand::random::<u8>() < 1 {
                limiter.cleanup().await;
            }

            Ok(res)
        })
    }
}

#[derive(Clone)]
pub struct HourlyIpLimiter {
    limit_per_hour: u32,
    request_counts: Arc<RwLock<HashMap<String, (u32, u64)>>>,
}

impl HourlyIpLimiter {
    pub fn new(limit_per_hour: u32) -> Self {
        Self {
            limit_per_hour,
            request_counts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn check_and_update(&self, ip: &str) -> bool {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let window_id = now / 3600;

        let mut request_counts = self.request_counts.write().await;
        let entry = request_counts.entry(ip.to_string()).or_insert((0, window_id));

        if entry.1 != window_id {
            *entry = (1, window_id);
            return true;
        }

        entry.0 += 1;
        entry.0 <= self.limit_per_hour
    }
}

pub struct HourlyIpLimiterMiddleware<S> {
    service: S,
    limiter: HourlyIpLimiter,
}

impl<S, B> Transform<S, ServiceRequest> for HourlyIpLimiter
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = HourlyIpLimiterMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(HourlyIpLimiterMiddleware {
            service,
            limiter: self.clone(),
        }))
    }
}

impl<S, B> Service<ServiceRequest> for HourlyIpLimiterMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let limiter = self.limiter.clone();
        let ip = req
            .connection_info()
            .peer_addr()
            .unwrap_or("unknown")
            .to_string();

        let path = req.path().to_string();
        let method = req.method().clone();
        let fut = self.service.call(req);

        Box::pin(async move {
            let limited = (path.starts_with("/api/v1/upload") && method == actix_web::http::Method::POST)
                || (path.starts_with("/api/v1/build") && method == actix_web::http::Method::POST)
                || (path.starts_with("/api/v1/build/") && (method == actix_web::http::Method::GET));

            if limited && !limiter.check_and_update(&ip).await {
                return Err(actix_web::error::ErrorTooManyRequests(serde_json::json!({
                    "error": "Hourly rate limit exceeded",
                    "message": "Too many requests from this IP. Max 20 requests per hour.",
                    "limit": 20,
                    "window": "1h"
                })));
            }

            let res = fut.await?;
            Ok(res)
        })
    }
}