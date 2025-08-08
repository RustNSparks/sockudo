// src/rate_limiter/memory_limiter.rs
use super::{RateLimitConfig, RateLimitResult, RateLimiter};
use crate::error::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::interval;
/// Entry in the rate limiter map
#[derive(Clone)]
struct RateLimitEntry {
    /// Current count of requests
    count: u32,
    /// When the window started
    window_start: Instant,
    /// When the window will reset
    expiry: Instant,
}

/// In-memory rate limiter implementation
pub struct MemoryRateLimiter {
    /// Storage for rate limit counters
    limits: Arc<DashMap<String, RateLimitEntry, ahash::RandomState>>,
    /// Configuration for rate limiting
    config: RateLimitConfig,
    /// Cleanup task handle
    #[allow(dead_code)]
    cleanup_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl MemoryRateLimiter {
    /// Create a new memory-based rate limiter
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self::with_config(RateLimitConfig {
            max_requests,
            window_secs,
            identifier: Some("memory".to_string()),
        })
    }

    /// Create a new memory-based rate limiter with a specific configuration
    pub fn with_config(config: RateLimitConfig) -> Self {
        let limits: Arc<DashMap<String, RateLimitEntry, ahash::RandomState>> =
            Arc::new(DashMap::with_hasher(ahash::RandomState::new()));
        let limits_clone = Arc::clone(&limits);

        // Start cleanup task
        let cleanup_task = tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(10)); // Check every 10 seconds
            loop {
                interval.tick().await;
                let now = Instant::now();
                // remove all expired limits
                limits_clone.retain(|_, value| value.expiry > now);
            }
        });

        Self {
            limits,
            config,
            cleanup_task: Arc::new(Mutex::new(Some(cleanup_task))),
        }
    }
}

#[async_trait]
impl RateLimiter for MemoryRateLimiter {
    async fn check(&self, key: &str) -> Result<RateLimitResult> {
        let now = Instant::now();

        if let Some(entry) = self.limits.get(key) {
            // Check if the window has expired
            if entry.expiry <= now {
                // Window expired, will be cleaned up later
                return Ok(RateLimitResult {
                    allowed: true,
                    remaining: self.config.max_requests,
                    reset_after: self.config.window_secs,
                    limit: self.config.max_requests,
                });
            }

            // Calculate remaining requests and time to reset
            let remaining = self.config.max_requests.saturating_sub(entry.count);
            let reset_after = entry.expiry.saturating_duration_since(now).as_secs();

            Ok(RateLimitResult {
                allowed: remaining > 0,
                remaining,
                reset_after,
                limit: self.config.max_requests,
            })
        } else {
            // No entry yet, so full allowance
            Ok(RateLimitResult {
                allowed: true,
                remaining: self.config.max_requests,
                reset_after: self.config.window_secs,
                limit: self.config.max_requests,
            })
        }
    }

    async fn increment(&self, key: &str) -> Result<RateLimitResult> {
        let now = Instant::now();

        // Try to get or create an entry
        let result = if let Some(mut entry) = self.limits.get_mut(key) {
            // Check if the window has expired
            if entry.expiry <= now {
                // Reset the window
                entry.count = 1;
                entry.window_start = now;
                entry.expiry = now + Duration::from_secs(self.config.window_secs);

                RateLimitResult {
                    allowed: true,
                    remaining: self.config.max_requests - 1,
                    reset_after: self.config.window_secs,
                    limit: self.config.max_requests,
                }
            } else {
                // Increment the counter
                let new_count = entry.count + 1;
                entry.count = new_count;

                let remaining = self.config.max_requests.saturating_sub(new_count);
                let reset_after = entry.expiry.saturating_duration_since(now).as_secs();

                RateLimitResult {
                    allowed: remaining > 0,
                    remaining,
                    reset_after,
                    limit: self.config.max_requests,
                }
            }
        } else {
            // Create a new entry
            let entry = RateLimitEntry {
                count: 1,
                window_start: now,
                expiry: now + Duration::from_secs(self.config.window_secs),
            };

            self.limits.insert(key.to_string(), entry);

            RateLimitResult {
                allowed: true,
                remaining: self.config.max_requests - 1,
                reset_after: self.config.window_secs,
                limit: self.config.max_requests,
            }
        };

        Ok(result)
    }

    async fn reset(&self, key: &str) -> Result<()> {
        self.limits.remove(key);
        Ok(())
    }

    async fn get_remaining(&self, key: &str) -> Result<u32> {
        let now = Instant::now();

        if let Some(entry) = self.limits.get(key) {
            // Check if the window has expired
            if entry.expiry <= now {
                // Window expired, will be cleaned up later
                return Ok(self.config.max_requests);
            }

            // Calculate remaining requests
            let remaining = self.config.max_requests.saturating_sub(entry.count);
            Ok(remaining)
        } else {
            // No entry yet, so full allowance
            Ok(self.config.max_requests)
        }
    }
}

impl Drop for MemoryRateLimiter {
    fn drop(&mut self) {
        // Attempt to abort the cleanup task if it's still running
        if let Ok(mut task_guard) = self.cleanup_task.try_lock()
            && let Some(task) = task_guard.take()
        {
            task.abort();
        }
    }
}
