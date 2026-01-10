#![allow(dead_code)]
#![allow(unused_variables)]

use crate::error::Result;
use crate::rate_limiter::RateLimiter;
use std::sync::Arc;
#[cfg(any(feature = "redis", feature = "redis-cluster"))]
use tracing::error;
use tracing::{info, warn};

use crate::options::{CacheDriver, RateLimiterConfig, RedisConnection};
use crate::rate_limiter::memory_limiter::MemoryRateLimiter;
#[cfg(feature = "redis-cluster")]
use crate::rate_limiter::redis_cluster_limiter::RedisClusterRateLimiter;
#[cfg(feature = "redis")]
use crate::rate_limiter::redis_limiter::RedisRateLimiter;

pub struct RateLimiterFactory;

impl RateLimiterFactory {
    pub async fn create(
        config: &RateLimiterConfig,
        global_redis_conn_details: &RedisConnection,
    ) -> Result<Arc<dyn RateLimiter + Send + Sync>> {
        if !config.enabled {
            info!("HTTP API Rate limiting is globally disabled. Returning a permissive limiter.");
            return Ok(Arc::new(MemoryRateLimiter::new(u32::MAX, 1))); // Allows all
        }

        info!(
            "Initializing HTTP API RateLimiter with driver: {:?}",
            config.driver
        );

        match config.driver {
            #[cfg(feature = "redis")]
            CacheDriver::Redis => {
                Self::create_redis_limiter(config, global_redis_conn_details).await
            }
            #[cfg(feature = "redis-cluster")]
            CacheDriver::RedisCluster => {
                Self::create_redis_cluster_limiter(config, global_redis_conn_details).await
            }
            CacheDriver::Memory => Self::create_memory_limiter(config),
            CacheDriver::None => {
                warn!("Rate limiter driver set to 'None'. Using memory limiter as fallback.");
                Ok(Arc::new(MemoryRateLimiter::new(
                    config.api_rate_limit.max_requests,
                    config.api_rate_limit.window_seconds,
                )))
            }
            #[cfg(not(feature = "redis"))]
            CacheDriver::Redis => {
                warn!(
                    "Redis rate limiter requested but not compiled in. Falling back to memory limiter."
                );
                Self::create_memory_limiter(config)
            }
            #[cfg(not(feature = "redis-cluster"))]
            CacheDriver::RedisCluster => {
                warn!(
                    "Redis Cluster rate limiter requested but not compiled in. Falling back to memory limiter."
                );
                Self::create_memory_limiter(config)
            }
        }
    }

    #[cfg(feature = "redis")]
    async fn create_redis_limiter(
        config: &RateLimiterConfig,
        global_redis_conn_details: &RedisConnection,
    ) -> Result<Arc<dyn RateLimiter + Send + Sync>> {
        info!("RateLimiter: Using standalone Redis backend.");

        let redis_url = config.redis.url_override.clone().unwrap_or_else(|| {
            format!(
                "redis://{}:{}",
                global_redis_conn_details.host, global_redis_conn_details.port
            )
        });

        let prefix = config
            .redis
            .prefix
            .clone()
            .unwrap_or_else(|| global_redis_conn_details.key_prefix.clone() + "rl_http:");

        let client = redis::Client::open(redis_url.as_str()).map_err(|e| {
            crate::error::Error::Redis(format!(
                "Failed to create Redis client for rate limiter: {e}"
            ))
        })?;

        let limiter = RedisRateLimiter::new(
            client,
            prefix,
            config.api_rate_limit.max_requests,
            config.api_rate_limit.window_seconds,
        )
        .await?;

        Ok(Arc::new(limiter))
    }

    #[cfg(feature = "redis-cluster")]
    async fn create_redis_cluster_limiter(
        config: &RateLimiterConfig,
        global_redis_conn_details: &RedisConnection,
    ) -> Result<Arc<dyn RateLimiter + Send + Sync>> {
        info!("RateLimiter: Using Redis Cluster backend.");

        if global_redis_conn_details.cluster_nodes.is_empty() {
            error!("RateLimiter: Redis cluster driver selected, but no cluster_nodes configured.");
            return Err(crate::error::Error::Configuration(
                "RateLimiter: Redis cluster nodes not configured.".to_string(),
            ));
        }

        let nodes: Vec<String> = global_redis_conn_details
            .cluster_nodes
            .iter()
            .map(|node| format!("redis://{}:{}", node.host, node.port))
            .collect();

        let prefix = config
            .redis
            .prefix
            .clone()
            .unwrap_or_else(|| global_redis_conn_details.key_prefix.clone() + "rl_http:");

        let client = redis::cluster::ClusterClient::new(nodes).map_err(|e| {
            crate::error::Error::Redis(format!(
                "Failed to create Redis cluster client for rate limiter: {e}"
            ))
        })?;

        let limiter = RedisClusterRateLimiter::new(
            client,
            prefix,
            config.api_rate_limit.max_requests,
            config.api_rate_limit.window_seconds,
        )
        .await?;

        Ok(Arc::new(limiter))
    }

    fn create_memory_limiter(
        config: &RateLimiterConfig,
    ) -> Result<Arc<dyn RateLimiter + Send + Sync>> {
        info!("Using memory rate limiter for HTTP API.");
        let limiter = MemoryRateLimiter::new(
            config.api_rate_limit.max_requests,
            config.api_rate_limit.window_seconds,
        );
        Ok(Arc::new(limiter))
    }
}
