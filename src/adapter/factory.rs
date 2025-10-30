use std::sync::Arc;
use tokio::sync::Mutex;
// src/adapter/factory.rs
use crate::adapter::ConnectionManager;
use crate::adapter::local_adapter::LocalAdapter;
#[cfg(feature = "nats")]
use crate::adapter::nats_adapter::NatsAdapter;
#[cfg(feature = "redis")]
use crate::adapter::redis_adapter::{RedisAdapter, RedisAdapterOptions};
#[cfg(feature = "redis-cluster")]
use crate::adapter::redis_cluster_adapter::RedisClusterAdapter;
use crate::error::Result;

#[cfg(feature = "nats")]
use crate::options::NatsAdapterConfig;
#[cfg(feature = "redis-cluster")]
use crate::options::RedisClusterAdapterConfig; // Import AdapterDriver, RedisConnection
use crate::options::{AdapterConfig, AdapterDriver, DatabaseConfig};
use tracing::{info, warn};

pub struct AdapterFactory;

impl AdapterFactory {
    #[allow(unused_variables)]
    pub async fn create(
        config: &AdapterConfig,
        db_config: &DatabaseConfig,
    ) -> Result<Arc<Mutex<dyn ConnectionManager + Send + Sync>>> {
        info!(
            "{}",
            format!(
                "Initializing ConnectionManager with driver: {:?}",
                config.driver
            )
        );
        match config.driver {
            // Match on the enum
            #[cfg(feature = "redis")]
            AdapterDriver::Redis => {
                let redis_url = config
                    .redis
                    .redis_pub_options
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| {
                        format!("redis://{}:{}", db_config.redis.host, db_config.redis.port)
                    });

                let adapter_options = RedisAdapterOptions {
                    url: redis_url,
                    prefix: config.redis.prefix.clone(),
                    request_timeout_ms: config.redis.requests_timeout,
                    cluster_mode: config.redis.cluster_mode,
                };
                match RedisAdapter::new(adapter_options).await {
                    Ok(mut adapter) => {
                        adapter.set_cluster_health(&config.cluster_health).await?;
                        Ok(Arc::new(Mutex::new(adapter)))
                    }
                    Err(e) => {
                        warn!(
                            "{}",
                            format!(
                                "Failed to initialize Redis adapter: {}, falling back to local adapter",
                                e
                            )
                        );
                        Ok(Arc::new(Mutex::new(
                            LocalAdapter::new_with_buffer_multiplier(
                                config.buffer_multiplier_per_cpu,
                            ),
                        )))
                    }
                }
            }
            #[cfg(feature = "redis-cluster")]
            AdapterDriver::RedisCluster => {
                // Use nodes from the specific RedisClusterAdapterConfig if available, else from DatabaseConfig.redis
                let nodes = if !config.cluster.nodes.is_empty() {
                    config.cluster.nodes.clone()
                } else {
                    db_config
                        .redis
                        .cluster_nodes
                        .iter()
                        .map(|node| node.to_url())
                        .collect()
                };

                if nodes.is_empty() {
                    warn!("{}", "Redis Cluster Adapter selected, but no nodes configured. Falling back to local adapter.".to_string());
                    return Ok(Arc::new(Mutex::new(
                        LocalAdapter::new_with_buffer_multiplier(config.buffer_multiplier_per_cpu),
                    )));
                }

                let cluster_adapter_config = RedisClusterAdapterConfig {
                    // Use the specific config struct
                    nodes,
                    prefix: config.cluster.prefix.clone(),
                    request_timeout_ms: config.cluster.request_timeout_ms,
                    use_connection_manager: config.cluster.use_connection_manager,
                };
                match RedisClusterAdapter::new(cluster_adapter_config).await {
                    Ok(mut adapter) => {
                        adapter.set_cluster_health(&config.cluster_health).await?;
                        Ok(Arc::new(Mutex::new(adapter)))
                    }
                    Err(e) => {
                        warn!(
                            "{}",
                            format!(
                                "Failed to initialize Redis Cluster adapter: {}, falling back to local adapter",
                                e
                            )
                        );
                        Ok(Arc::new(Mutex::new(
                            LocalAdapter::new_with_buffer_multiplier(
                                config.buffer_multiplier_per_cpu,
                            ),
                        )))
                    }
                }
            }
            #[cfg(feature = "nats")]
            AdapterDriver::Nats => {
                let nats_cfg = NatsAdapterConfig {
                    // Directly use the NatsAdapterConfig from options
                    servers: config.nats.servers.clone(),
                    prefix: config.nats.prefix.clone(),
                    request_timeout_ms: config.nats.request_timeout_ms,
                    username: config.nats.username.clone(),
                    password: config.nats.password.clone(),
                    token: config.nats.token.clone(),
                    connection_timeout_ms: config.nats.connection_timeout_ms,
                    nodes_number: config.nats.nodes_number,
                };
                match NatsAdapter::new(nats_cfg).await {
                    Ok(mut adapter) => {
                        adapter.set_cluster_health(&config.cluster_health).await?;
                        Ok(Arc::new(Mutex::new(adapter)))
                    }
                    Err(e) => {
                        warn!(
                            "{}",
                            format!(
                                "Failed to initialize NATS adapter: {}, falling back to local adapter",
                                e
                            )
                        );
                        Ok(Arc::new(Mutex::new(
                            LocalAdapter::new_with_buffer_multiplier(
                                config.buffer_multiplier_per_cpu,
                            ),
                        )))
                    }
                }
            }
            AdapterDriver::Local => {
                // Handle unknown as Local or make it an error
                info!("{}", "Using local adapter.".to_string());
                Ok(Arc::new(Mutex::new(
                    LocalAdapter::new_with_buffer_multiplier(config.buffer_multiplier_per_cpu),
                )))
            }
            #[cfg(not(feature = "redis"))]
            AdapterDriver::Redis => {
                warn!(
                    "{}",
                    "Redis adapter requested but not compiled in. Falling back to local adapter."
                        .to_string()
                );
                Ok(Arc::new(Mutex::new(
                    LocalAdapter::new_with_buffer_multiplier(config.buffer_multiplier_per_cpu),
                )))
            }
            #[cfg(not(feature = "redis-cluster"))]
            AdapterDriver::RedisCluster => {
                warn!("{}", "Redis Cluster adapter requested but not compiled in. Falling back to local adapter.".to_string());
                Ok(Arc::new(Mutex::new(
                    LocalAdapter::new_with_buffer_multiplier(config.buffer_multiplier_per_cpu),
                )))
            }
            #[cfg(not(feature = "nats"))]
            AdapterDriver::Nats => {
                warn!(
                    "{}",
                    "NATS adapter requested but not compiled in. Falling back to local adapter."
                        .to_string()
                );
                Ok(Arc::new(Mutex::new(
                    LocalAdapter::new_with_buffer_multiplier(config.buffer_multiplier_per_cpu),
                )))
            }
        }
    }
}
