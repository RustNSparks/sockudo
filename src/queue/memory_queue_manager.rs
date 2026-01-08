// Memory-based queue manager implementation

use crate::queue::{ArcJobProcessorFn, QueueInterface};
use crate::webhook::sender::JobProcessorFnAsync;
use crate::webhook::types::JobData;
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};

/// Memory-based queue manager for simple deployments
pub struct MemoryQueueManager {
    // Use channels to simulate a queue in memory
    // DashMap<String, Vec<JobData>> is implicitly Send + Sync if JobData is Send
    queues: Arc<DashMap<String, Vec<JobData>, ahash::RandomState>>,
    // Store Arc'd callbacks to be consistent with Redis manager and avoid potential issues if Box wasn't 'static
    processors: Arc<DashMap<String, ArcJobProcessorFn, ahash::RandomState>>,
}

impl Default for MemoryQueueManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryQueueManager {
    pub fn new() -> Self {
        let queues = Arc::new(DashMap::with_hasher(ahash::RandomState::new()));
        let processors = Arc::new(DashMap::with_hasher(ahash::RandomState::new()));

        Self { queues, processors }
    }

    /// Starts the background processing loop. Should be called once after setup.
    pub fn start_processing(&self) {
        // Clone Arcs for the background task
        let queues = Arc::clone(&self.queues);
        let processors = Arc::clone(&self.processors);

        info!("{}", "Starting memory queue processing loop...".to_string());

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));

            loop {
                interval.tick().await;

                // Collect queue names first to avoid holding iterator while modifying
                // Only collect queues that have registered processors for efficiency
                let queue_names: Vec<String> = queues
                    .iter()
                    .map(|entry| entry.key().clone())
                    .filter(|name| processors.contains_key(name))
                    .collect();

                // Process each queue without holding any iterator
                for queue_name in queue_names {
                    // Get the processor for this queue (we know it exists from filter above)
                    if let Some(processor) = processors.get(&queue_name) {
                        // Try to remove and drain the queue atomically
                        if let Some((key, mut jobs_vec)) = queues.remove(&queue_name) {
                            // Only process if there are jobs
                            if !jobs_vec.is_empty() {
                                debug!(
                                    "Processing {} jobs from memory queue {}",
                                    jobs_vec.len(),
                                    key
                                );

                                // Re-insert empty queue immediately for new jobs
                                queues.insert(key, Vec::new());

                                // Spawn each job asynchronously to avoid blocking
                                for job in jobs_vec.drain(..) {
                                    let processor_clone = Arc::clone(&processor);
                                    tokio::spawn(async move {
                                        if let Err(e) = processor_clone(job).await {
                                            tracing::error!("Failed to process webhook job: {}", e);
                                        }
                                    });
                                }
                            } else {
                                // Re-insert the empty queue
                                queues.insert(key, jobs_vec);
                            }
                        }
                    }
                }
            }
        });
    }
}

#[async_trait]
impl QueueInterface for MemoryQueueManager {
    async fn add_to_queue(&self, queue_name: &str, data: JobData) -> crate::error::Result<()> {
        // Ensure queue Vec exists using entry API for atomicity
        self.queues
            .entry(queue_name.to_string())
            .or_default()
            .push(data);
        Ok(())
    }

    async fn process_queue(
        &self,
        queue_name: &str,
        callback: JobProcessorFnAsync,
    ) -> crate::error::Result<()> {
        // Register processor, wrapping it in Arc
        self.processors
            .insert(queue_name.to_string(), Arc::from(callback));
        debug!("Registered processor for memory queue: {}", queue_name);

        Ok(())
    }

    async fn disconnect(&self) -> crate::error::Result<()> {
        self.queues.clear();
        Ok(())
    }

    async fn check_health(&self) -> crate::error::Result<()> {
        Ok(())
    }

    async fn queue_depth(&self, queue_name: &str) -> crate::error::Result<u64> {
        if let Some(queue) = self.queues.get(queue_name) {
            Ok(queue.len() as u64)
        } else {
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::JobData;
    use crate::webhook::types::JobPayload;

    #[tokio::test]
    async fn test_add_to_queue() {
        let manager = MemoryQueueManager::new();
        let data = JobData {
            app_key: "test_key".to_string(),
            app_id: "test_id".to_string(),
            app_secret: "test_secret".to_string(),
            payload: JobPayload {
                time_ms: chrono::Utc::now().timestamp_millis(),
                events: vec![],
            },
            original_signature: "test_signature".to_string(),
        };

        manager
            .add_to_queue("test_queue", data.clone())
            .await
            .unwrap();

        assert_eq!(manager.queues.get("test_queue").unwrap().len(), 1);
    }

    //todo think how to test process_queue

    #[tokio::test]
    async fn test_disconnect() {
        let manager = MemoryQueueManager::new();
        let data = JobData {
            app_key: "test_key".to_string(),
            app_id: "test_id".to_string(),
            app_secret: "test_secret".to_string(),
            payload: JobPayload {
                time_ms: chrono::Utc::now().timestamp_millis(),
                events: vec![],
            },
            original_signature: "test_signature".to_string(),
        };

        manager.add_to_queue("test_queue", data).await.unwrap();
        assert!(!manager.queues.is_empty());

        manager.disconnect().await.unwrap();
        assert!(manager.queues.is_empty());
    }
}
