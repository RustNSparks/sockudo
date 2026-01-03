#![allow(dead_code)]

use crate::cache::manager::CacheManager;
use crate::error::{Error, Result};
use async_trait::async_trait;
use redis::{AsyncCommands, Client, aio::ConnectionManager};
use std::time::Duration;
use tracing::warn;

/// Configuration for the Redis cache manager
#[derive(Clone, Debug)]
pub struct RedisCacheConfig {
    /// Redis URL
    pub url: String,
    /// Key prefix
    pub prefix: String,
    /// Response timeout
    pub response_timeout: Option<Duration>,
    /// Use RESP3 protocol
    pub use_resp3: bool,
}

impl Default for RedisCacheConfig {
    fn default() -> Self {
        Self {
            url: "redis://127.0.0.1:6379/".to_string(),
            prefix: "cache".to_string(),
            response_timeout: Some(Duration::from_secs(5)),
            use_resp3: false,
        }
    }
}

/// A Redis-based implementation of the CacheManager trait
pub struct RedisCacheManager {
    /// Redis client
    client: Client,
    /// Connection manager with automatic reconnection
    connection: redis::aio::ConnectionManager,
    /// Key prefix
    prefix: String,
}

impl RedisCacheManager {
    /// Creates a new Redis cache manager with configuration
    pub async fn new(config: RedisCacheConfig) -> Result<Self> {
        // Build the Redis URL with RESP3 if enabled
        let redis_url = if config.use_resp3 && !config.url.contains("protocol=resp3") {
            if config.url.contains('?') {
                format!("{}&protocol=resp3", config.url)
            } else {
                format!("{}?protocol=resp3", config.url)
            }
        } else {
            config.url
        };

        // Create Redis client
        let client = Client::open(redis_url)
            .map_err(|e| Error::Cache(format!("Failed to create Redis client: {e}")))?;

        // Create ConnectionManager with same config as RedisAdapter for consistency
        let connection_manager_config = redis::aio::ConnectionManagerConfig::new()
            .set_number_of_retries(5)
            .set_exponent_base(2)
            .set_factor(500)
            .set_max_delay(5000);

        let connection = client
            .get_connection_manager_with_config(connection_manager_config)
            .await
            .map_err(|e| Error::Cache(format!("Failed to connect to Redis: {e}")))?;

        Ok(Self {
            client,
            connection,
            prefix: config.prefix,
        })
    }

    /// Creates a new Redis cache manager with simple configuration
    pub async fn with_url(redis_url: &str, prefix: Option<&str>) -> Result<Self> {
        let config = RedisCacheConfig {
            url: redis_url.to_string(),
            prefix: prefix.unwrap_or("cache").to_string(),
            ..Default::default()
        };

        Self::new(config).await
    }

    /// Get the prefixed key
    fn prefixed_key(&self, key: &str) -> String {
        format!("{}:{}", self.prefix, key)
    }

    /// Scan for keys matching a pattern using SCAN command.
    /// This is more efficient and production-safe than KEYS command.
    /// Compatible with AWS Valkey Serverless which doesn't support KEYS.
    async fn scan_keys(&mut self, pattern: &str) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        let mut cursor: u64 = 0;
        let scan_count = 100; // Number of keys to fetch per iteration

        loop {
            let (new_cursor, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(scan_count)
                .query_async(&mut self.connection)
                .await
                .map_err(|e| Error::Cache(format!("Redis SCAN error: {e}")))?;

            keys.extend(batch);
            cursor = new_cursor;

            if cursor == 0 {
                break;
            }
        }

        Ok(keys)
    }
}

#[async_trait]
impl CacheManager for RedisCacheManager {
    /// Check if the given key exists in cache
    async fn has(&mut self, key: &str) -> Result<bool> {
        let exists: bool = self
            .connection
            .exists(self.prefixed_key(key))
            .await
            .map_err(|e| Error::Cache(format!("Redis exists error: {e}")))?;
        Ok(exists)
    }

    /// Get a key from the cache
    /// Returns None if cache does not exist
    async fn get(&mut self, key: &str) -> Result<Option<String>> {
        let value: Option<String> = self
            .connection
            .get(self.prefixed_key(key))
            .await
            .map_err(|e| Error::Cache(format!("Redis get error: {e}")))?;
        Ok(value)
    }

    /// Set or overwrite the value in the cache
    async fn set(&mut self, key: &str, value: &str, ttl_seconds: u64) -> Result<()> {
        let prefixed_key = self.prefixed_key(key);

        if ttl_seconds > 0 {
            // Set with expiration
            self.connection
                .set_ex::<_, _, ()>(prefixed_key, value, ttl_seconds)
                .await
                .map_err(|e| Error::Cache(format!("Redis set error: {e}")))?;
        } else {
            // Set without expiration
            self.connection
                .set::<_, _, ()>(prefixed_key, value)
                .await
                .map_err(|e| Error::Cache(format!("Redis set error: {e}")))?;
        }

        Ok(())
    }

    async fn remove(&mut self, key: &str) -> Result<()> {
        let deleted: i32 = self
            .connection
            .del(self.prefixed_key(key))
            .await
            .map_err(|e| Error::Cache(format!("Redis delete error: {e}")))?;
        if deleted > 0 {
            Ok(())
        } else {
            Err(Error::Cache(format!("Key '{key}' not found")))
        }
    }

    /// Disconnect the manager's made connections
    async fn disconnect(&mut self) -> Result<()> {
        // Clean up all keys with the current prefix using SCAN (Valkey Serverless compatible)
        let pattern = format!("{}:*", self.prefix);
        let keys = self.scan_keys(&pattern).await?;

        if !keys.is_empty() {
            // Delete keys in batches to avoid blocking
            for chunk in keys.chunks(100) {
                if let Err(e) = self.connection.del::<_, i32>(chunk.to_vec()).await {
                    warn!("Failed to delete cache keys during disconnect: {}", e);
                }
            }
        }

        Ok(())
    }

    async fn check_health(&self) -> Result<()> {
        // Use a dedicated connection for health check to avoid impacting main operations
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| Error::Cache(format!("Failed to acquire health check connection: {e}")))?;

        let response = redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .map_err(|e| Error::Cache(format!("Cache health check PING failed: {e}")))?;

        if response == "PONG" {
            Ok(())
        } else {
            Err(Error::Cache(format!(
                "Cache Redis PING returned unexpected response: {response}"
            )))
        }
    }
    async fn ttl(&mut self, key: &str) -> Result<Option<Duration>> {
        let ttl: i64 = self
            .connection
            .ttl(self.prefixed_key(key))
            .await
            .map_err(|e| Error::Cache(format!("Redis TTL error: {e}")))?;
        if ttl > 0 {
            Ok(Some(Duration::from_secs(ttl as u64)))
        } else {
            Ok(None)
        }
    }
}

// Additional utility methods for the cache manager
impl RedisCacheManager {
    /// Delete a key from the cache
    pub async fn delete(&mut self, key: &str) -> Result<bool> {
        let deleted: i32 = self
            .connection
            .del(self.prefixed_key(key))
            .await
            .map_err(|e| Error::Cache(format!("Redis delete error: {e}")))?;
        Ok(deleted > 0)
    }

    /// Clear all keys with the current prefix
    /// Uses SCAN instead of KEYS for Valkey Serverless compatibility
    pub async fn clear_prefix(&mut self) -> Result<usize> {
        let pattern = format!("{}:*", self.prefix);

        // Use SCAN to find keys (Valkey Serverless compatible)
        let keys = self.scan_keys(&pattern).await?;

        if keys.is_empty() {
            return Ok(0);
        }

        // Delete keys in batches to avoid blocking
        let mut total_deleted: usize = 0;
        for chunk in keys.chunks(100) {
            let deleted: i32 = self
                .connection
                .del(chunk.to_vec())
                .await
                .map_err(|e| Error::Cache(format!("Redis delete error: {e}")))?;
            total_deleted += deleted as usize;
        }

        Ok(total_deleted)
    }

    /// Set multiple key-value pairs at once
    pub async fn set_many(&mut self, pairs: &[(&str, &str)], ttl_seconds: u64) -> Result<()> {
        if pairs.is_empty() {
            return Ok(());
        }

        // Convert to prefixed keys
        let prefixed_pairs: Vec<(String, &str)> = pairs
            .iter()
            .map(|(k, v)| (self.prefixed_key(k), *v))
            .collect();

        // Use a pipeline for better performance
        let mut pipe = redis::pipe();

        for (key, value) in &prefixed_pairs {
            if ttl_seconds > 0 {
                pipe.set_ex(key, *value, ttl_seconds as usize as u64);
            } else {
                pipe.set(key, *value);
            }
        }

        // Execute pipeline
        pipe.query_async::<()>(&mut self.connection)
            .await
            .map_err(|e| Error::Cache(format!("Redis pipeline error: {e}")))?;

        Ok(())
    }

    /// Increment a counter in Redis
    pub async fn increment(&mut self, key: &str, by: i64) -> Result<i64> {
        let value: i64 = self
            .connection
            .incr(self.prefixed_key(key), by)
            .await
            .map_err(|e| Error::Cache(format!("Redis increment error: {e}")))?;
        Ok(value)
    }

    /// Get the remaining TTL for a key in seconds - todo
    pub async fn get_remaining_ttl() {
        todo!()
    }

    /// Get multiple keys at once
    pub async fn get_many(&mut self, keys: &[&str]) -> Result<Vec<Option<String>>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        // Convert to prefixed keys
        let prefixed_keys: Vec<String> = keys.iter().map(|k| self.prefixed_key(k)).collect();

        // Use MGET for better performance
        let values: Vec<Option<String>> = self
            .connection
            .mget(prefixed_keys)
            .await
            .map_err(|e| Error::Cache(format!("Redis mget error: {e}")))?;

        Ok(values)
    }

    /// Flush all keys from the current database
    pub async fn flush_db(&mut self) -> Result<()> {
        // Use the cmd method to execute FLUSHDB command
        redis::cmd("FLUSHDB")
            .query_async::<()>(&mut self.connection)
            .await
            .map_err(|e| Error::Cache(format!("Redis flushdb error: {e}")))?;

        Ok(())
    }

    /// Return the raw connection manager for advanced operations
    pub fn get_connection(&self) -> ConnectionManager {
        self.connection.clone()
    }
}

/// Factory for creating cache managers
pub struct CacheManagerFactory;

impl CacheManagerFactory {
    /// Create a new Redis cache manager
    pub async fn create_redis(
        redis_url: &str,
        prefix: Option<&str>,
        response_timeout: Option<Duration>,
    ) -> Result<Box<dyn CacheManager + Send>> {
        let config = RedisCacheConfig {
            url: redis_url.to_string(),
            prefix: prefix.unwrap_or("cache").to_string(),
            response_timeout,
            use_resp3: false,
        };

        let cache_manager = RedisCacheManager::new(config).await?;
        Ok(Box::new(cache_manager))
    }

    /// Create a new Redis cache manager with RESP3 protocol
    pub async fn create_redis_resp3(
        redis_url: &str,
        prefix: Option<&str>,
        response_timeout: Option<Duration>,
    ) -> Result<Box<dyn CacheManager + Send>> {
        let config = RedisCacheConfig {
            url: redis_url.to_string(),
            prefix: prefix.unwrap_or("cache").to_string(),
            response_timeout,
            use_resp3: true,
        };

        let cache_manager = RedisCacheManager::new(config).await?;
        Ok(Box::new(cache_manager))
    }
}
