use crate::cache::manager::CacheManager;
use crate::error::{Error, Result};
use async_trait::async_trait;
use redis::AsyncCommands;
use redis::cluster::{ClusterClient, ClusterClientBuilder};
use redis::cluster_async::ClusterConnection;
use std::time::Duration;

/// Configuration for the Redis Cluster cache manager
#[derive(Clone, Debug)]
pub struct RedisClusterCacheConfig {
    /// Redis cluster nodes (array of "host:port" strings)
    pub nodes: Vec<String>,
    /// Key prefix
    pub prefix: String,
    /// Response timeout
    pub response_timeout: Option<Duration>,
    /// Read from replicas (if supported)
    pub read_from_replicas: bool,
}

impl Default for RedisClusterCacheConfig {
    fn default() -> Self {
        Self {
            nodes: vec!["127.0.0.1:6379".to_string()],
            prefix: "cache".to_string(),
            response_timeout: Some(Duration::from_secs(5)),
            read_from_replicas: false,
        }
    }
}

/// A Redis Cluster-based implementation of the CacheManager trait
pub struct RedisClusterCacheManager {
    /// Redis cluster client
    client: ClusterClient,
    /// Cluster connection
    connection: ClusterConnection,
    /// Key prefix
    prefix: String,
}

impl RedisClusterCacheManager {
    /// Creates a new Redis Cluster cache manager with configuration
    pub async fn new(config: RedisClusterCacheConfig) -> Result<Self> {
        // Create Redis cluster client builder
        let mut builder = ClusterClientBuilder::new(config.nodes.clone());
        if let Some(timeout) = config.response_timeout {
            // Note: This is a no-op in current redis-rs version for cluster connections
            // but kept for future compatibility
            builder = builder.response_timeout(timeout)
        }

        // Configure builder with read from replicas if enabled
        if config.read_from_replicas {
            builder = builder.read_from_replicas();
        }

        // Build the cluster client
        let client = builder
            .build()
            .map_err(|e| Error::Cache(format!("Failed to create Redis Cluster client: {e}")))?;

        // Get cluster connection
        let connection = client
            .get_async_connection()
            .await
            .map_err(|e| Error::Cache(format!("Failed to connect to Redis Cluster: {e}")))?;

        Ok(Self {
            client,
            connection,
            prefix: config.prefix,
        })
    }

    /// Creates a new Redis Cluster cache manager with simple configuration
    pub async fn with_nodes(nodes: Vec<String>, prefix: Option<&str>) -> Result<Self> {
        let config = RedisClusterCacheConfig {
            nodes,
            prefix: prefix.unwrap_or("cache").to_string(),
            ..Default::default()
        };

        Self::new(config).await
    }

    /// Get the prefixed key
    fn prefixed_key(&self, key: &str) -> String {
        format!("{}:{}", self.prefix, key)
    }
}

#[async_trait]
impl CacheManager for RedisClusterCacheManager {
    /// Check if the given key exists in cache
    async fn has(&mut self, key: &str) -> Result<bool> {
        let exists: bool = self
            .connection
            .exists(self.prefixed_key(key))
            .await
            .map_err(|e| Error::Cache(format!("Redis Cluster exists error: {e}")))?;
        Ok(exists)
    }

    /// Get a key from the cache
    /// Returns None if cache does not exist
    async fn get(&mut self, key: &str) -> Result<Option<String>> {
        let value: Option<String> = self
            .connection
            .get(self.prefixed_key(key))
            .await
            .map_err(|e| Error::Cache(format!("Redis Cluster get error: {e}")))?;
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
                .map_err(|e| Error::Cache(format!("Redis Cluster set error: {e}")))?;
        } else {
            // Set without expiration
            self.connection
                .set::<_, _, ()>(prefixed_key, value)
                .await
                .map_err(|e| Error::Cache(format!("Redis Cluster set error: {e}")))?;
        }

        Ok(())
    }

    async fn remove(&mut self, key: &str) -> Result<()> {
        let deleted: i32 = self
            .connection
            .del(self.prefixed_key(key))
            .await
            .map_err(|e| Error::Cache(format!("Redis Cluster delete error: {e}")))?;
        if deleted == 0 {
            return Err(Error::Cache(format!("Key '{key}' not found")));
        }
        Ok(())
    }

    /// Disconnect the manager's made connections
    async fn disconnect(&mut self) -> Result<()> {
        // lcear all the cache
        self.clear_prefix().await?;
        Ok(())
    }

    async fn check_health(&self) -> Result<()> {
        // Use a dedicated connection for health check to avoid impacting main operations
        let mut connection =
            self.client.get_async_connection().await.map_err(|e| {
                Error::Cache(format!("Failed to acquire health check connection: {e}"))
            })?;

        let response = redis::cmd("PING")
            .query_async::<String>(&mut connection)
            .await
            .map_err(|e| {
                Error::Cache(format!("Cache Redis Cluster health check PING failed: {e}"))
            })?;

        if response == "PONG" {
            Ok(())
        } else {
            Err(Error::Cache(format!(
                "Cache Redis Cluster PING returned unexpected response: {response}"
            )))
        }
    }

    async fn ttl(&mut self, key: &str) -> Result<Option<Duration>> {
        let ttl: i64 = self
            .connection
            .ttl(self.prefixed_key(key))
            .await
            .map_err(|e| Error::Cache(format!("Redis Cluster TTL error: {e}")))?;
        if ttl < 0 {
            return Ok(None);
        }
        Ok(Some(Duration::from_secs(ttl as u64)))
    }
}

// Additional utility methods for the cache manager
impl RedisClusterCacheManager {
    /// Delete a key from the cache
    pub async fn delete(&mut self, key: &str) -> Result<bool> {
        let deleted: i32 = self
            .connection
            .del(self.prefixed_key(key))
            .await
            .map_err(|e| Error::Cache(format!("Redis Cluster delete error: {e}")))?;
        Ok(deleted > 0)
    }

    /// Clear all keys with the current prefix
    pub async fn clear_prefix(&mut self) -> Result<usize> {
        // Note: KEYS command is not directly supported in Redis Cluster across slots
        // This is a simplified implementation that may not be efficient for large datasets
        let pattern = format!("{}:*", self.prefix);

        // First get the keys
        // Warning: This may not work as expected in large clusters due to slot distribution
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(&pattern)
            .query_async(&mut self.connection)
            .await
            .map_err(|e| Error::Cache(format!("Redis Cluster keys error: {e}")))?;

        if keys.is_empty() {
            return Ok(0);
        }

        // Delete keys one by one to handle cluster slot distribution
        let mut deleted_count = 0;
        for key in keys {
            let deleted: i32 = self
                .connection
                .del(&key)
                .await
                .map_err(|e| Error::Cache(format!("Redis Cluster delete error: {e}")))?;
            deleted_count += deleted as usize;
        }

        Ok(deleted_count)
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

        // In Redis Cluster, MSET might span multiple slots, so we handle each pair individually
        for (key, value) in &prefixed_pairs {
            if ttl_seconds > 0 {
                self.connection
                    .set_ex::<_, _, ()>(key, *value, ttl_seconds)
                    .await
                    .map_err(|e| Error::Cache(format!("Redis Cluster set_ex error: {e}")))?;
            } else {
                self.connection
                    .set::<_, _, ()>(key, *value)
                    .await
                    .map_err(|e| Error::Cache(format!("Redis Cluster set error: {e}")))?;
            }
        }

        Ok(())
    }

    /// Increment a counter in Redis Cluster
    pub async fn increment(&mut self, key: &str, by: i64) -> Result<i64> {
        let value: i64 = self
            .connection
            .incr(self.prefixed_key(key), by)
            .await
            .map_err(|e| Error::Cache(format!("Redis Cluster increment error: {e}")))?;
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

        // In Redis Cluster, MGET might span multiple slots, so we handle each key individually
        let mut results = Vec::with_capacity(keys.len());
        for key in keys {
            let value: Option<String> = self
                .connection
                .get(self.prefixed_key(key))
                .await
                .map_err(|e| Error::Cache(format!("Redis Cluster get error: {e}")))?;
            results.push(value);
        }

        Ok(results)
    }

    /// Get the cluster client for advanced operations
    pub fn get_client(&self) -> ClusterClient {
        self.client.clone()
    }

    /// Get a new connection to the cluster
    pub async fn get_connection(&self) -> Result<ClusterConnection> {
        self.client
            .get_async_connection()
            .await
            .map_err(|e| Error::Cache(format!("Failed to get Redis Cluster connection: {e}")))
    }

    /// Get cluster info
    pub async fn get_cluster_info(&mut self) -> Result<String> {
        let info: String = redis::cmd("CLUSTER")
            .arg("INFO")
            .query_async(&mut self.connection)
            .await
            .map_err(|e| Error::Cache(format!("Redis Cluster info error: {e}")))?;

        Ok(info)
    }

    /// Get cluster nodes
    pub async fn get_cluster_nodes(&mut self) -> Result<String> {
        let nodes: String = redis::cmd("CLUSTER")
            .arg("NODES")
            .query_async(&mut self.connection)
            .await
            .map_err(|e| Error::Cache(format!("Redis Cluster nodes error: {e}")))?;

        Ok(nodes)
    }
}

/// Factory for creating cache managers
pub struct ClusterCacheManagerFactory;

impl ClusterCacheManagerFactory {
    /// Create a new Redis Cluster cache manager
    pub async fn create_redis_cluster(
        nodes: Vec<String>,
        prefix: Option<&str>,
        response_timeout: Option<Duration>,
        read_from_replicas: bool,
    ) -> Result<Box<dyn CacheManager + Send>> {
        let config = RedisClusterCacheConfig {
            nodes,
            prefix: prefix.unwrap_or("cache").to_string(),
            response_timeout,
            read_from_replicas,
        };

        let cache_manager = RedisClusterCacheManager::new(config).await?;
        Ok(Box::new(cache_manager))
    }
}
