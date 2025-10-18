// src/rate_limiter/mod.rs
pub mod factory;
pub mod memory_limiter;
pub mod middleware;
#[cfg(feature = "redis-cluster")]
pub mod redis_cluster_limiter;
#[cfg(feature = "redis")]
pub mod redis_limiter;

use crate::error::Result;
use async_trait::async_trait;
use std::sync::Arc;

/// Configuration for rate limiters
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum number of requests
    pub max_requests: u32,
    /// Time window in seconds
    pub window_secs: u64,
    /// Optional identifier for the limiter (e.g., "api_calls", "websocket_connects")
    pub identifier: Option<String>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 60,
            window_secs: 60, // 60 requests per minute by default
            identifier: None,
        }
    }
}

/// Rate limit check result
#[derive(Debug, Clone)]
pub struct RateLimitResult {
    /// Whether the request should be allowed
    pub allowed: bool,
    /// Number of remaining requests in the current window
    pub remaining: u32,
    /// When the rate limit will reset (in seconds)
    pub reset_after: u64,
    /// Total limit for the window
    pub limit: u32,
}

/// Common trait for all rate limiters
#[async_trait]
pub trait RateLimiter: Send + Sync + 'static {
    /// Check if a request is allowed for a given key
    async fn check(&self, key: &str) -> Result<RateLimitResult>;
    /// Increment the counter for a key and check if the request is allowed
    /// Returns the same result as `check` but also increments the counter
    async fn increment(&self, key: &str) -> Result<RateLimitResult>;
    /// Reset the counter for a key
    async fn reset(&self, key: &str) -> Result<()>;
    /// Get the remaining requests for a key without incrementing
    async fn get_remaining(&self, key: &str) -> Result<u32>;
}

// Then modify the middleware module to accept Arc<dyn RateLimiter> directly
pub mod middleware_utils {
    use super::*;
    use crate::rate_limiter::middleware::{IpKeyExtractor, RateLimitLayer, RateLimitOptions};

    // Helper function to create rate limit middleware with Arc<dyn RateLimiter>
    pub fn with_arc_ip_limiter(
        limiter: Arc<dyn RateLimiter>,
        options: RateLimitOptions,
    ) -> RateLimitLayer<IpKeyExtractor> {
        RateLimitLayer::with_options(limiter, IpKeyExtractor::new(1), options)
    }
}
