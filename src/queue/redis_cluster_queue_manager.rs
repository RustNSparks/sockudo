use crate::queue::{ArcJobProcessorFn, QueueInterface};
use crate::webhook::sender::JobProcessorFnAsync;
use crate::webhook::types::JobData;
use async_trait::async_trait;
use redis::cluster::ClusterClient;
use redis::cluster_async::ClusterConnection;
use redis::{AsyncCommands, RedisResult};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, info};

pub struct RedisClusterQueueManager {
    redis_client: ClusterClient,
    redis_connection: Arc<Mutex<ClusterConnection>>,
    // Store Arc'd callbacks to allow cloning them into worker tasks safely
    job_processors: dashmap::DashMap<String, ArcJobProcessorFn, ahash::RandomState>,
    prefix: String,
    concurrency: usize,
}

impl RedisClusterQueueManager {
    /// Creates a new RedisClusterQueueManager instance.
    /// Connects to Redis Cluster and returns a Result.
    pub async fn new(
        cluster_nodes: Vec<String>,
        prefix: &str,
        concurrency: usize,
    ) -> crate::error::Result<Self> {
        let client = ClusterClient::new(cluster_nodes.clone()).map_err(|e| {
            crate::error::Error::Config(format!("Failed to create Redis cluster client: {e}"))
        })?;

        let connection = client.get_async_connection().await.map_err(|e| {
            crate::error::Error::Connection(format!("Failed to get Redis cluster connection: {e}"))
        })?;

        info!(
            "Connected to Redis cluster with {} nodes, prefix: {}, concurrency: {}",
            cluster_nodes.len(),
            prefix,
            concurrency
        );

        Ok(Self {
            redis_client: client,
            redis_connection: Arc::new(Mutex::new(connection)),
            job_processors: dashmap::DashMap::with_hasher(ahash::RandomState::new()),
            prefix: prefix.to_string(),
            concurrency,
        })
    }

    // Note: start_processing is effectively done within process_queue for Redis Cluster
    #[allow(dead_code)]
    pub fn start_processing(&self) {
        // This method is not strictly needed for Redis Cluster as workers start in process_queue.
        // Could be used for other setup if required in the future.
    }

    async fn format_key(&self, queue_name: &str) -> String {
        format!("{}:queue:{}", self.prefix, queue_name)
    }
}

#[async_trait]
impl QueueInterface for RedisClusterQueueManager {
    /// Adds a job to the specified Redis cluster queue (list).
    /// Serializes the job data to JSON.
    async fn add_to_queue(&self, queue_name: &str, data: JobData) -> crate::error::Result<()>
    where
        JobData: Serialize, // Ensure JobData can be serialized
    {
        let queue_key = self.format_key(queue_name).await;
        let data_json = sonic_rs::to_string(&data)?; // Propagate serialization error

        let mut conn = self.redis_connection.lock().await;

        // Perform RPUSH and handle potential Redis errors
        conn.rpush::<_, _, ()>(&queue_key, data_json)
            .await
            .map_err(|e| {
                crate::error::Error::Queue(format!(
                    "Redis Cluster RPUSH failed for queue {queue_name}: {e}"
                ))
            })?; // Use custom error type

        // info!("{}", format!("Added job to Redis cluster queue: {}", queue_name)); // Optional: reduce log verbosity

        Ok(())
    }

    /// Registers a callback for a queue and starts worker tasks to process jobs.
    async fn process_queue(
        &self,
        queue_name: &str,
        callback: JobProcessorFnAsync,
    ) -> crate::error::Result<()>
    where
        JobData: DeserializeOwned + Send + 'static, // Ensure JobData can be deserialized and sent across threads
    {
        let queue_key = self.format_key(queue_name).await;

        // Wrap the callback in an Arc to share it safely with multiple worker tasks
        let processor_arc: ArcJobProcessorFn = Arc::from(callback);

        // Store the Arc'd callback
        self.job_processors
            .insert(queue_name.to_string(), processor_arc.clone());
        debug!(
            "{}",
            format!(
                "Registered processor and starting workers for Redis cluster queue: {}",
                queue_name
            )
        );

        // Start worker tasks
        for i in 0..self.concurrency {
            let worker_queue_key = queue_key.clone();
            let worker_redis_conn = self.redis_connection.clone();
            let worker_processor = processor_arc.clone(); // Clone the Arc for this worker
            let worker_queue_name = queue_name.to_string(); // Clone queue name for logging

            tokio::spawn(async move {
                debug!(
                    "{}",
                    format!(
                        "Starting Redis cluster queue worker {} for queue: {}",
                        i, worker_queue_name
                    )
                );

                loop {
                    let blpop_result: RedisResult<Option<(String, String)>> = {
                        // Type hint for clarity
                        let mut conn = worker_redis_conn.lock().await;
                        // Use BLPOP with a timeout (e.g., 0.01 second)
                        conn.blpop(&worker_queue_key, 0.01).await
                    };

                    match blpop_result {
                        // Successfully received a job
                        Ok(Some((_key, job_data_str))) => {
                            match sonic_rs::from_str::<JobData>(&job_data_str) {
                                Ok(job_data) => {
                                    // Execute the job processing callback
                                    match worker_processor(job_data).await {
                                        Ok(_) => {
                                            debug!("{}", "Cluster worker finished".to_string());
                                        }
                                        Err(e) => {
                                            error!("{}", format!("Cluster worker error: {}", e));
                                        }
                                    }
                                }
                                Err(e) => {
                                    // Failed to deserialize the job data
                                    error!(
                                        "{}",
                                        format!(
                                            "[Cluster Worker {}] Error deserializing job data from Redis cluster queue {}: {}. Data: '{}'",
                                            i, worker_queue_name, e, job_data_str
                                        )
                                    );
                                    // Potential: Move corrupted data to a specific place?
                                }
                            }
                        }
                        // BLPOP timed out, no job available
                        Ok(None) => {
                            // Continue loop to wait again
                            continue;
                        }
                        // Redis error during BLPOP
                        Err(e) => {
                            error!(
                                "{}",
                                format!(
                                    "[Cluster Worker {}] Redis cluster BLPOP error on queue {}: {}",
                                    i, worker_queue_name, e
                                )
                            );
                            // Avoid hammering Redis cluster on persistent errors
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            });
        }

        Ok(())
    }

    async fn disconnect(&self) -> crate::error::Result<()> {
        let mut conn = self.redis_connection.lock().await;
        let keys: Vec<String> = conn
            .keys(format!("{}:queue:*", self.prefix))
            .await
            .map_err(|e| {
                crate::error::Error::Queue(format!(
                    "Redis cluster disconnect error fetching keys: {e}"
                ))
            })?;

        for key in keys {
            if let Err(e) = conn.del::<_, ()>(&key).await {
                error!("Error deleting key {} during disconnect: {}", key, e);
            }
        }
        Ok(())
    }

    async fn check_health(&self) -> crate::error::Result<()> {
        // Create a separate connection for health check to avoid lock contention
        let mut conn = self
            .redis_client
            .get_async_connection()
            .await
            .map_err(|e| {
                crate::error::Error::Redis(format!("Queue Redis Cluster connection failed: {e}"))
            })?;

        let response = redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .map_err(|e| {
                crate::error::Error::Redis(format!("Queue Redis Cluster PING failed: {e}"))
            })?;

        if response == "PONG" {
            Ok(())
        } else {
            Err(crate::error::Error::Redis(format!(
                "Queue Redis Cluster PING returned unexpected response: {response}"
            )))
        }
    }
}
