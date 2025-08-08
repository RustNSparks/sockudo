use crate::error::{Error, Result};

use crate::options::SqsQueueConfig; // Use the struct from options.rs
use crate::queue::{ArcJobProcessorFn, QueueInterface};
use crate::webhook::sender::JobProcessorFnAsync;
use async_trait::async_trait;
use aws_sdk_sqs as sqs;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::interval;
use tracing::{error, info};

/// SQS-based implementation of the QueueInterface
pub struct SqsQueueManager {
    /// SQS client
    client: sqs::Client,
    /// Configuration
    config: SqsQueueConfig, // Using the struct from options.rs

    // todo -> change Mutex<Hashmap> to Dashmap
    /// Cache of queue URLs
    queue_urls: Arc<Mutex<HashMap<String, String>>>,
    /// Active workers
    worker_handles: Arc<Mutex<HashMap<String, Vec<tokio::task::JoinHandle<()>>>>>,
    /// Flag to control worker shutdown
    shutdown: Arc<Mutex<bool>>,
}

impl SqsQueueManager {
    /// Create a new SQS queue manager
    pub async fn new(config: SqsQueueConfig) -> Result<Self> {
        // Build AWS config
        let mut aws_config_builder = aws_config::from_env();

        // Set region
        aws_config_builder =
            aws_config_builder.region(aws_sdk_sqs::config::Region::new(config.region.clone()));

        // Set custom endpoint if provided (for local testing)
        if let Some(endpoint) = &config.endpoint_url {
            aws_config_builder = aws_config_builder.endpoint_url(endpoint);
        }

        // Build the AWS config
        let aws_config = aws_config_builder.load().await;

        // Create SQS client
        let client = sqs::Client::new(&aws_config);

        Ok(Self {
            client,
            config,
            queue_urls: Arc::new(Mutex::new(HashMap::new())),
            worker_handles: Arc::new(Mutex::new(HashMap::new())),
            shutdown: Arc::new(Mutex::new(false)),
        })
    }

    /// Get or create the URL for a queue
    async fn get_queue_url(&self, queue_name: &str) -> Result<String> {
        // Check cache first
        let cached_url = {
            let queue_urls = self.queue_urls.lock().await;
            queue_urls.get(queue_name).cloned()
        };

        if let Some(url) = cached_url {
            return Ok(url);
        }

        // If custom prefix is provided, use that to construct queue URL
        if let Some(prefix) = &self.config.queue_url_prefix {
            let queue_url = if self.config.fifo {
                format!("{prefix}/{queue_name}.fifo")
            } else {
                format!("{prefix}/{queue_name}")
            };

            // Cache the URL
            let mut queue_urls = self.queue_urls.lock().await;
            queue_urls.insert(queue_name.to_string(), queue_url.clone());

            return Ok(queue_url);
        }

        // Try to get the queue URL from AWS
        let actual_queue_name = if self.config.fifo && !queue_name.ends_with(".fifo") {
            format!("{queue_name}.fifo")
        } else {
            queue_name.to_string()
        };

        match self
            .client
            .get_queue_url()
            .queue_name(&actual_queue_name)
            .send()
            .await
        {
            Ok(output) => {
                if let Some(url) = output.queue_url() {
                    // Cache the URL for future use
                    let mut queue_urls = self.queue_urls.lock().await;
                    queue_urls.insert(queue_name.to_string(), url.to_string());

                    Ok(url.to_string())
                } else {
                    Err(Error::Queue(format!(
                        "No URL returned for queue: {queue_name}"
                    )))
                }
            }
            Err(e) => {
                // If queue doesn't exist, try to create it
                if e.to_string().contains("QueueDoesNotExist")
                    || e.to_string().contains("NonExistentQueue")
                {
                    self.create_queue(queue_name).await
                } else {
                    Err(Error::Queue(format!("Failed to get queue URL: {e}")))
                }
            }
        }
    }

    /// Create a queue if it doesn't exist
    async fn create_queue(&self, queue_name: &str) -> Result<String> {
        info!("{}", format!("Creating SQS queue: {}", queue_name));

        let actual_queue_name = if self.config.fifo && !queue_name.ends_with(".fifo") {
            format!("{queue_name}.fifo")
        } else {
            queue_name.to_string()
        };

        // Set up attributes for the queue using proper QueueAttributeName enum
        let mut attributes = HashMap::new();

        // Use the enum type for attribute names
        use aws_sdk_sqs::types::QueueAttributeName;

        attributes.insert(
            QueueAttributeName::VisibilityTimeout,
            self.config.visibility_timeout.to_string(),
        );

        if self.config.fifo {
            attributes.insert(QueueAttributeName::FifoQueue, "true".to_string());
            // Content-based deduplication for FIFO queues
            attributes.insert(
                QueueAttributeName::ContentBasedDeduplication,
                "true".to_string(),
            );
        }

        // Create the queue
        let result = self
            .client
            .create_queue()
            .queue_name(&actual_queue_name)
            .set_attributes(Some(attributes))
            .send()
            .await
            .map_err(|e| Error::Queue(format!("Failed to create SQS queue {queue_name}: {e}")))?;

        if let Some(url) = result.queue_url() {
            // Cache the URL
            let mut queue_urls = self.queue_urls.lock().await;
            queue_urls.insert(queue_name.to_string(), url.to_string());

            Ok(url.to_string())
        } else {
            Err(Error::Queue(format!(
                "No URL returned after creating queue: {queue_name}"
            )))
        }
    }

    /// Start a worker for processing messages from the queue
    async fn start_worker(
        &self,
        queue_name: &str,
        queue_url: String,
        processor: ArcJobProcessorFn,
        worker_id: usize,
    ) -> tokio::task::JoinHandle<()> {
        // Clone values needed for the worker
        let client = self.client.clone();
        let config = self.config.clone();
        let shutdown = self.shutdown.clone();
        let queue_name = queue_name.to_string();

        // Spawn the worker task
        tokio::spawn(async move {
            info!(
                "{}",
                format!(
                    "Starting SQS worker #{} for queue: {}",
                    worker_id, queue_name
                )
            );

            let mut interval = interval(Duration::from_secs(1));

            loop {
                interval.tick().await;

                // Check if we should shutdown
                if *shutdown.lock().await {
                    info!(
                        "{}",
                        format!(
                            "SQS worker #{} for queue {} shutting down",
                            worker_id, queue_name
                        )
                    );
                    break;
                }

                // Receive messages from the queue
                let result = client
                    .receive_message()
                    .queue_url(&queue_url)
                    .max_number_of_messages(config.max_messages)
                    .visibility_timeout(config.visibility_timeout)
                    .wait_time_seconds(config.wait_time_seconds)
                    .send()
                    .await;

                match result {
                    Ok(response) => {
                        // Get messages from the response - messages() returns Option<&[Message]>
                        let messages = response.messages();

                        // Check if we received any messages
                        if !messages.is_empty() {
                            info!(
                                "{}",
                                format!(
                                    "SQS worker #{} received {} messages from queue {}",
                                    worker_id,
                                    messages.len(),
                                    queue_name
                                )
                            );

                            // Iterate through the messages in the slice
                            for message in messages {
                                if let Some(body) = message.body() {
                                    // Process the message
                                    match serde_json::from_str::<crate::webhook::types::JobData>(
                                        body,
                                    ) {
                                        Ok(job_data) => {
                                            // Call the processor
                                            match processor(job_data).await {
                                                Ok(_) => {
                                                    // Processing succeeded, delete the message
                                                    if let Some(receipt_handle) =
                                                        message.receipt_handle()
                                                        && let Err(e) = client
                                                            .delete_message()
                                                            .queue_url(&queue_url)
                                                            .receipt_handle(receipt_handle)
                                                            .send()
                                                            .await
                                                    {
                                                        error!(
                                                            "{}",
                                                            format!(
                                                                "Failed to delete message from SQS queue {}: {}",
                                                                queue_name, e
                                                            )
                                                        );
                                                    }
                                                }
                                                Err(e) => {
                                                    // Processing failed, log the error
                                                    error!(
                                                        "{}",
                                                        format!(
                                                            "Error processing message from SQS queue {}: {}",
                                                            queue_name, e
                                                        )
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            error!(
                                                "{}",
                                                format!(
                                                    "Failed to deserialize message from SQS queue {}: {}",
                                                    queue_name, e
                                                )
                                            );

                                            // Delete malformed messages
                                            if let Some(receipt_handle) = message.receipt_handle()
                                                && let Err(e) = client
                                                    .delete_message()
                                                    .queue_url(&queue_url)
                                                    .receipt_handle(receipt_handle)
                                                    .send()
                                                    .await
                                            {
                                                error!(
                                                    "{}",
                                                    format!(
                                                        "Failed to delete malformed message from SQS queue {}: {}",
                                                        queue_name, e
                                                    )
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            "{}",
                            format!(
                                "Failed to receive messages from SQS queue {}: {}",
                                queue_name, e
                            )
                        );

                        // Add a small delay before retrying on error
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        })
    }
}

#[async_trait]
impl QueueInterface for SqsQueueManager {
    /// Add a job to a queue
    async fn add_to_queue(
        &self,
        queue_name: &str,
        data: crate::webhook::types::JobData,
    ) -> Result<()> {
        // Get the queue URL
        let queue_url = self.get_queue_url(queue_name).await?;

        // Serialize the job data
        let data_json = serde_json::to_string(&data)
            .map_err(|e| Error::Queue(format!("Failed to serialize job data: {e}")))?;

        // Build send message request
        let mut send_message_request = self
            .client
            .send_message()
            .queue_url(queue_url)
            .message_body(data_json);

        // Add FIFO-specific attributes if needed
        if self.config.fifo {
            // For FIFO queues, we need a message group ID and deduplication ID
            if let Some(group_id) = &self.config.message_group_id {
                send_message_request = send_message_request.message_group_id(group_id);
            }

            // Use a unique ID based on the request ID for deduplication
            // Only needed if ContentBasedDeduplication is not enabled
            // send_message_request = send_message_request.message_deduplication_id(&data.request_id);
        }

        // Send the message to SQS
        let result = send_message_request.send().await.map_err(|e| {
            Error::Queue(format!(
                "Failed to send message to SQS queue {queue_name}: {e}"
            ))
        })?;

        info!(
            "{}",
            format!(
                "Added job to SQS queue {} with ID: {}",
                queue_name,
                result.message_id().unwrap_or("unknown")
            )
        );

        Ok(())
    }

    /// Process jobs from a queue
    async fn process_queue(&self, queue_name: &str, callback: JobProcessorFnAsync) -> Result<()> {
        // Get the queue URL
        let queue_url = self.get_queue_url(queue_name).await?;

        // Wrap the callback in an Arc for thread-safe sharing
        let processor: ArcJobProcessorFn = Arc::from(callback);

        // Start workers
        let mut worker_handles = Vec::new();
        let concurrency = self.config.concurrency as usize;
        for worker_id in 0..concurrency {
            let handle = self
                .start_worker(queue_name, queue_url.clone(), processor.clone(), worker_id)
                .await;

            worker_handles.push(handle);
        }

        // Store the worker handles
        let mut handles = self.worker_handles.lock().await;
        handles.insert(queue_name.to_string(), worker_handles);

        info!(
            "{}",
            format!(
                "Started {} workers for SQS queue: {}",
                concurrency, queue_name
            )
        );

        Ok(())
    }

    /// Disconnect and clean up
    async fn disconnect(&self) -> Result<()> {
        // Signal workers to shutdown
        {
            let mut shutdown = self.shutdown.lock().await;
            *shutdown = true;
        }

        // Wait for workers to finish
        {
            let mut handles = self.worker_handles.lock().await;
            for (queue_name, workers) in handles.drain() {
                info!(
                    "{}",
                    format!("Waiting for SQS queue {} workers to shutdown", queue_name)
                );

                for handle in workers {
                    // We don't want to await here as it could block indefinitely
                    // Just detach the tasks and let them complete on their own
                    handle.abort();
                }
            }
        }

        Ok(())
    }

    async fn check_health(&self) -> crate::error::Result<()> {
        self.client
            .list_queues()
            .send()
            .await
            .map_err(|e| crate::error::Error::Queue(format!("Queue SQS connection failed: {e}")))?;
        Ok(())
    }
}

impl Drop for SqsQueueManager {
    fn drop(&mut self) {
        // Signal workers to shutdown when the manager is dropped
        // We can't use async functions in drop, so we spawn a task
        let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            let mut lock = shutdown.lock().await;
            *lock = true;
        });
    }
}
