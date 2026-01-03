#![allow(dead_code)]

// src/cache/factory.rs
use crate::cache::manager::CacheManager;
use crate::cache::memory_cache_manager::MemoryCacheManager; // Assuming MemoryCacheConfig is from options
#[cfg(feature = "redis")]
use crate::cache::redis_cache_manager::{
    RedisCacheConfig as StandaloneRedisCacheConfig, RedisCacheManager,
};
#[cfg(feature = "redis-cluster")]
use crate::cache::redis_cluster_cache_manager::{
    RedisClusterCacheConfig, RedisClusterCacheManager,
};
use crate::error::{Error, Result};

use crate::options::{CacheConfig, CacheDriver, MemoryCacheOptions, RedisConnection};
use std::sync::Arc;
use tokio::sync::Mutex;
#[cfg(any(feature = "redis", feature = "redis-cluster"))]
use tracing::error;
use tracing::info;

pub struct CacheManagerFactory;

impl CacheManagerFactory {
    #[allow(unused_variables)]
    pub async fn create(
        config: &CacheConfig,
        global_redis_conn_details: &RedisConnection,
    ) -> Result<Arc<Mutex<dyn CacheManager + Send + Sync>>> {
        // Corrected return type
        info!(
            "{}",
            format!("Initializing CacheManager with driver: {:?}", config.driver)
        );

        match config.driver {
            #[cfg(feature = "redis")]
            CacheDriver::Redis => {
                if config.redis.cluster_mode {
                    info!("{}", "Cache: Using Redis Cluster driver.".to_string());
                    if global_redis_conn_details.cluster_nodes.is_empty() {
                        error!("{}", "Cache: Redis cluster mode enabled, but no cluster_nodes configured in database.redis section.".to_string());
                        return Err(Error::Cache(
                            "Cache: Redis cluster nodes not configured.".to_string(),
                        ));
                    }
                    let nodes: Vec<String> = global_redis_conn_details
                        .cluster_nodes
                        .iter()
                        .map(|node| node.to_url())
                        .collect();

                    let prefix =
                        config.redis.prefix.clone().unwrap_or_else(|| {
                            global_redis_conn_details.key_prefix.clone() + "cache:"
                        });

                    let cluster_cache_config = RedisClusterCacheConfig {
                        nodes,
                        prefix,
                        ..Default::default()
                    };
                    let manager = RedisClusterCacheManager::new(cluster_cache_config).await?;
                    // Directly create Arc<Mutex<ConcreteType>> which coerces to Arc<Mutex<dyn Trait>>
                    Ok(Arc::new(Mutex::new(manager)))
                } else {
                    info!("{}", "Cache: Using standalone Redis driver.".to_string());
                    let redis_url = config
                        .redis
                        .url_override
                        .clone()
                        .unwrap_or_else(|| global_redis_conn_details.to_url());
                    if global_redis_conn_details.is_sentinel_mode() {
                        info!(
                            "Cache: Using Redis Sentinel mode with {} sentinel nodes",
                            global_redis_conn_details.sentinels.len()
                        );
                    }

                    let prefix =
                        config.redis.prefix.clone().unwrap_or_else(|| {
                            global_redis_conn_details.key_prefix.clone() + "cache:"
                        });

                    let standalone_redis_cache_config = StandaloneRedisCacheConfig {
                        url: redis_url,
                        prefix,
                        ..Default::default()
                    };
                    let manager = RedisCacheManager::new(standalone_redis_cache_config).await?;
                    Ok(Arc::new(Mutex::new(manager)))
                }
            }
            #[cfg(feature = "redis-cluster")]
            CacheDriver::RedisCluster => {
                info!(
                    "{}",
                    "Cache: Using Redis Cluster driver (explicitly selected).".to_string()
                );
                if global_redis_conn_details.cluster_nodes.is_empty() {
                    error!("{}", "Cache: Redis cluster driver selected, but no cluster_nodes configured in database.redis section.".to_string());
                    return Err(Error::Cache(
                        "Cache: Redis cluster nodes not configured for explicit cluster driver."
                            .to_string(),
                    ));
                }
                let nodes: Vec<String> = global_redis_conn_details
                    .cluster_nodes
                    .iter()
                    .map(|node| node.to_url())
                    .collect();

                let prefix = config
                    .redis
                    .prefix
                    .clone()
                    .unwrap_or_else(|| global_redis_conn_details.key_prefix.clone() + "cache:");

                let cluster_cache_config = RedisClusterCacheConfig {
                    nodes,
                    prefix,
                    ..Default::default()
                };
                let manager = RedisClusterCacheManager::new(cluster_cache_config).await?;
                Ok(Arc::new(Mutex::new(manager)))
            }
            CacheDriver::Memory => {
                info!("{}", "Using memory cache manager.".to_string());
                // Assuming MemoryCacheOptions is the correct config struct for MemoryCacheManager
                let _mem_config = MemoryCacheOptions {
                    ttl: config.memory.ttl,
                    cleanup_interval: config.memory.cleanup_interval,
                    max_capacity: config.memory.max_capacity,
                };
                let manager =
                    MemoryCacheManager::new("default_mem_cache".to_string(), config.memory.clone()); // Pass prefix and MemoryCacheOptions
                Ok(Arc::new(Mutex::new(manager)))
            }
            CacheDriver::None => {
                info!(
                    "{}",
                    "Cache driver is 'None'. Cache will be disabled.".to_string()
                );
                Err(Error::Cache(
                    "Cache driver explicitly set to 'None'.".to_string(),
                ))
            }
            #[cfg(not(feature = "redis"))]
            CacheDriver::Redis => {
                info!(
                    "{}",
                    "Redis cache manager requested but not compiled in. Falling back to memory cache."
                );
                let manager =
                    MemoryCacheManager::new("default_mem_cache".to_string(), config.memory.clone());
                Ok(Arc::new(Mutex::new(manager)))
            }
            #[cfg(not(feature = "redis-cluster"))]
            CacheDriver::RedisCluster => {
                info!(
                    "Redis Cluster cache manager requested but not compiled in. Falling back to memory cache."
                );
                let manager =
                    MemoryCacheManager::new("default_mem_cache".to_string(), config.memory.clone());
                Ok(Arc::new(Mutex::new(manager)))
            }
        }
    }
}
