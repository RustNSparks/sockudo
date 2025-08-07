// src/webhook/integration.rs
use crate::app::config::App;
use crate::app::manager::AppManager;
use crate::error::{Error, Result};

use crate::queue::manager::{QueueManager, QueueManagerFactory};
use crate::webhook::sender::WebhookSender;
use crate::webhook::types::{JobData, JobPayload};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::interval;
use tracing::{error, info};

/// Configuration for the webhook integration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub enabled: bool,
    pub batching: BatchingConfig,
    pub queue_driver: String,
    pub redis_url: Option<String>,
    pub redis_prefix: Option<String>,
    pub redis_concurrency: Option<usize>,
    pub process_id: String,
    pub debug: bool,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            batching: BatchingConfig::default(),
            queue_driver: "redis".to_string(),
            redis_url: None,
            redis_prefix: None,
            redis_concurrency: Some(5),
            process_id: uuid::Uuid::new_v4().to_string(),
            debug: false,
        }
    }
}

/// Configuration for webhook batching (Sockudo's internal batching)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchingConfig {
    pub enabled: bool,
    pub duration: u64, // in milliseconds
}

impl Default for BatchingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            duration: 50,
        }
    }
}

pub type JobProcessorFnAsync = Box<
    dyn Fn(JobData) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync + 'static,
>;

/// Webhook integration for processing events
pub struct WebhookIntegration {
    config: WebhookConfig,
    batched_webhooks: Arc<Mutex<HashMap<String, Vec<JobData>>>>,
    queue_manager: Option<Arc<Mutex<QueueManager>>>,
    app_manager: Arc<dyn AppManager + Send + Sync>,
}

impl WebhookIntegration {
    pub async fn new(
        config: WebhookConfig,
        app_manager: Arc<dyn AppManager + Send + Sync>,
    ) -> Result<Self> {
        let mut integration = Self {
            config,
            batched_webhooks: Arc::new(Mutex::new(HashMap::new())),
            queue_manager: None,
            app_manager,
        };

        if integration.config.enabled {
            integration.init_queue_manager().await?;
        }
        Ok(integration)
    }

    async fn init_queue_manager(&mut self) -> Result<()> {
        if self.is_enabled() {
            let driver = QueueManagerFactory::create(
                &self.config.queue_driver,
                self.config.redis_url.as_deref(),
                self.config.redis_prefix.as_deref(),
                self.config.redis_concurrency,
            )
            .await?;
            let queue_manager = Arc::new(Mutex::new(QueueManager::new(driver)));
            let webhook_sender = Arc::new(WebhookSender::new(self.app_manager.clone()));
            let queue_name = "webhooks".to_string();
            let sender_clone = webhook_sender.clone();

            let processor: JobProcessorFnAsync = Box::new(move |job_data| {
                let sender_for_task = sender_clone.clone();
                Box::pin(async move {
                    info!(
                        "{}",
                        format!("Processing webhook job from queue: {:?}", job_data.app_id)
                    );
                    sender_for_task.process_webhook_job(job_data).await
                })
            });

            {
                let manager = queue_manager.lock().await;
                manager.process_queue(&queue_name, processor).await?;
            }
            self.queue_manager = Some(queue_manager);

            if self.config.batching.enabled {
                self.start_batching_task();
            }
        }
        Ok(())
    }

    fn start_batching_task(&self) {
        if !self.config.batching.enabled {
            return;
        }
        let queue_manager_clone = self.queue_manager.clone();
        let batched_webhooks_clone = self.batched_webhooks.clone();
        let batch_duration = self.config.batching.duration;

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_millis(batch_duration));
            loop {
                interval.tick().await;
                let webhooks_to_process: HashMap<String, Vec<JobData>> = {
                    let mut batched = batched_webhooks_clone.lock().await;
                    std::mem::take(&mut *batched)
                };

                if webhooks_to_process.is_empty() {
                    continue;
                }
                info!(
                    "{}",
                    format!(
                        "Processing {} batched webhook queues (Sockudo internal batching)",
                        webhooks_to_process.len()
                    )
                );

                if let Some(manager_arc) = &queue_manager_clone {
                    for (queue_name, jobs) in webhooks_to_process {
                        let manager_locked = manager_arc.lock().await;
                        for job in jobs {
                            if let Err(e) = manager_locked.add_to_queue(&queue_name, job).await {
                                error!(
                                    "{}",
                                    format!(
                                        "Failed to add batched job to queue {}: {}",
                                        queue_name, e
                                    )
                                );
                            }
                        }
                    }
                }
            }
        });
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    async fn add_webhook(&self, queue_name: &str, job_data: JobData) -> Result<()> {
        if !self.is_enabled() {
            return Ok(());
        }
        if self.config.batching.enabled {
            let mut batched = self.batched_webhooks.lock().await;
            batched
                .entry(queue_name.to_string())
                .or_default()
                .push(job_data);
        } else if let Some(qm_arc) = &self.queue_manager {
            let manager = qm_arc.lock().await;
            manager.add_to_queue(queue_name, job_data).await?;
        } else {
            return Err(Error::Internal(
                "Queue manager not initialized for webhooks".to_string(),
            ));
        }
        Ok(())
    }

    fn create_job_data(
        &self,
        app: &App,
        events_payload: Vec<Value>,
        original_signature_for_queue: &str,
    ) -> JobData {
        let job_payload = JobPayload {
            time_ms: chrono::Utc::now().timestamp_millis(),
            events: events_payload,
        };
        JobData {
            app_key: app.key.clone(),
            app_id: app.id.clone(),
            app_secret: app.secret.clone(),
            payload: job_payload,
            original_signature: original_signature_for_queue.to_string(),
        }
    }

    async fn should_send_webhook(&self, app: &App, event_type_name: &str) -> bool {
        if !self.is_enabled() {
            return false;
        }
        app.webhooks.as_ref().is_some_and(|webhooks| {
            webhooks
                .iter()
                .any(|wh_config| wh_config.event_types.contains(&event_type_name.to_string()))
        })
    }

    pub async fn send_channel_occupied(&self, app: &App, channel: &str) -> Result<()> {
        if !self.should_send_webhook(app, "channel_occupied").await {
            return Ok(());
        }
        let event_obj = json!({
            "name": "channel_occupied",
            "channel": channel
        });
        let signature = format!("{}:{}:channel_occupied", app.id, channel);
        let job_data = self.create_job_data(app, vec![event_obj], &signature);
        self.add_webhook("webhooks", job_data).await
    }

    pub async fn send_channel_vacated(&self, app: &App, channel: &str) -> Result<()> {
        if !self.should_send_webhook(app, "channel_vacated").await {
            return Ok(());
        }
        let event_obj = json!({
            "name": "channel_vacated",
            "channel": channel
        });
        let signature = format!("{}:{}:channel_vacated", app.id, channel);
        let job_data = self.create_job_data(app, vec![event_obj], &signature);
        self.add_webhook("webhooks", job_data).await
    }

    pub async fn send_member_added(&self, app: &App, channel: &str, user_id: &str) -> Result<()> {
        if !self.should_send_webhook(app, "member_added").await {
            return Ok(());
        }
        let event_obj = json!({
            "name": "member_added",
            "channel": channel,
            "user_id": user_id
        });
        let signature = format!("{}:{}:{}:member_added", app.id, channel, user_id);
        let job_data = self.create_job_data(app, vec![event_obj], &signature);
        self.add_webhook("webhooks", job_data).await
    }

    pub async fn send_member_removed(&self, app: &App, channel: &str, user_id: &str) -> Result<()> {
        if !self.should_send_webhook(app, "member_removed").await {
            return Ok(());
        }
        let event_obj = json!({
            "name": "member_removed",
            "channel": channel,
            "user_id": user_id
        });
        let signature = format!("{}:{}:{}:member_removed", app.id, channel, user_id);
        let job_data = self.create_job_data(app, vec![event_obj], &signature);
        self.add_webhook("webhooks", job_data).await
    }

    pub async fn send_client_event(
        &self,
        app: &App,
        channel: &str,
        event_name: &str,
        event_data: Value,
        socket_id: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<()> {
        if !self.should_send_webhook(app, "client_event").await {
            return Ok(());
        }

        let mut client_event_pusher_payload = json!({
            "name": "client_event",
            "channel": channel,
            "event": event_name,
            "data": event_data,
            "socket_id": socket_id,
        });

        if channel.starts_with("presence-") {
            if let Some(uid) = user_id {
                client_event_pusher_payload["user_id"] = json!(uid);
            }
        }

        let signature = format!(
            "{}:{}:{}:client_event",
            app.id,
            channel,
            socket_id.unwrap_or("unknown")
        );
        let job_data = self.create_job_data(app, vec![client_event_pusher_payload], &signature);
        self.add_webhook("webhooks", job_data).await
    }

    pub async fn send_cache_missed(&self, app: &App, channel: &str) -> Result<()> {
        if !self.should_send_webhook(app, "cache_miss").await {
            return Ok(());
        }
        let event_obj = json!({
            "name": "cache_miss",
            "channel": channel
        });
        let signature = format!("{}:{}:cache_miss", app.id, channel);
        let job_data = self.create_job_data(app, vec![event_obj], &signature);
        self.add_webhook("webhooks", job_data).await
    }

    /// Sends a webhook when the subscription count for a channel changes.
    pub async fn send_subscription_count_changed(
        &self,
        app: &App,
        channel: &str,
        subscription_count: usize,
    ) -> Result<()> {
        // Check if any webhook is configured for "subscription_count" events for this app
        if !self.should_send_webhook(app, "subscription_count").await {
            return Ok(());
        }

        // Construct the event payload according to Pusher documentation
        let event_obj = json!({
            "name": "subscription_count",
            "channel": channel,
            "subscription_count": subscription_count // Pusher docs show this as a string, but number is more common
        });

        // Create a unique signature for queue deduplication or logging if needed
        let signature = format!(
            "{}:{}:subscription_count:{}",
            app.id, channel, subscription_count
        );

        let job_data = self.create_job_data(app, vec![event_obj], &signature);
        self.add_webhook("webhooks", job_data).await
    }

    /// Check the health of the queue manager used by webhook integration
    pub async fn check_queue_health(&self) -> Result<()> {
        if let Some(queue_manager_arc) = &self.queue_manager {
            let queue_manager = queue_manager_arc.lock().await;
            queue_manager.check_health().await
        } else {
            // If no queue manager is configured, consider it healthy
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{config::App, memory_app_manager::MemoryAppManager};

    #[tokio::test]
    async fn test_send_cache_missed() {
        let app = App {
            id: "test_app".to_string(),
            key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_client_events_per_second: 100,
            ..Default::default()
        };
        let app_manager = Arc::new(MemoryAppManager::new());
        let integration = WebhookIntegration::new(WebhookConfig::default(), app_manager.clone())
            .await
            .unwrap();

        let result = integration.send_cache_missed(&app, "test_channel").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_subscription_count_changed() {
        let app = App {
            id: "test_app".to_string(),
            key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_client_events_per_second: 100,
            ..Default::default()
        };
        let app_manager = Arc::new(MemoryAppManager::new());
        let integration = WebhookIntegration::new(WebhookConfig::default(), app_manager.clone())
            .await
            .unwrap();

        // Test when webhook should not be sent
        let result = integration
            .send_subscription_count_changed(&app, "test_channel", 5)
            .await;
        assert!(result.is_ok());

        // Test with webhook enabled
        let config = WebhookConfig {
            enabled: true,
            ..Default::default()
        };
        let integration = WebhookIntegration::new(config, app_manager).await.unwrap();

        let result = integration
            .send_subscription_count_changed(&app, "test_channel", 5)
            .await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_webhook_config_serialization() {
        let config = WebhookConfig {
            enabled: true,
            batching: BatchingConfig {
                enabled: true,
                duration: 1000,
            },
            queue_driver: "redis".to_string(),
            redis_url: Some("redis://localhost".to_string()),
            redis_prefix: Some("webhook".to_string()),
            redis_concurrency: Some(5),
            process_id: "test-process".to_string(),
            debug: false,
        };

        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: WebhookConfig = serde_json::from_str(&serialized).unwrap();

        assert_eq!(config.enabled, deserialized.enabled);
        assert_eq!(config.batching.enabled, deserialized.batching.enabled);
        assert_eq!(config.batching.duration, deserialized.batching.duration);
    }

    #[tokio::test]
    async fn test_webhook_integration_new() {
        let app_manager = Arc::new(MemoryAppManager::new());
        let config = WebhookConfig::default();

        let integration = WebhookIntegration::new(config, app_manager).await;
        assert!(integration.is_ok());
    }

    #[tokio::test]
    async fn test_webhook_integration_send_event() {
        let app = App {
            id: "test_app".to_string(),
            key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_client_events_per_second: 100,
            ..Default::default()
        };
        let app_manager = Arc::new(MemoryAppManager::new());
        let integration = WebhookIntegration::new(WebhookConfig::default(), app_manager.clone())
            .await
            .unwrap();

        let result = integration
            .send_client_event(
                &app,
                "test_channel",
                "test_event",
                json!("test_data"),
                None,
                None,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_webhook_integration_send_client_event() {
        let app = App {
            id: "test_app".to_string(),
            key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_client_events_per_second: 100,
            ..Default::default()
        };
        let app_manager = Arc::new(MemoryAppManager::new());
        let integration = WebhookIntegration::new(WebhookConfig::default(), app_manager.clone())
            .await
            .unwrap();

        let result = integration
            .send_client_event(
                &app,
                "test_channel",
                "test_event",
                json!("test_data"),
                None,
                None,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_webhook_integration_send_member_added() {
        let app = App {
            id: "test_app".to_string(),
            key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_client_events_per_second: 100,
            ..Default::default()
        };
        let app_manager = Arc::new(MemoryAppManager::new());
        let integration = WebhookIntegration::new(WebhookConfig::default(), app_manager.clone())
            .await
            .unwrap();

        let result = integration
            .send_member_added(&app, "test_channel", "test_user")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_webhook_integration_send_member_removed() {
        let app = App {
            id: "test_app".to_string(),
            key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_client_events_per_second: 100,
            ..Default::default()
        };
        let app_manager = Arc::new(MemoryAppManager::new());
        let integration = WebhookIntegration::new(WebhookConfig::default(), app_manager.clone())
            .await
            .unwrap();

        let result = integration
            .send_member_removed(&app, "test_channel", "test_user")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_webhook_integration_send_channel_occupied() {
        let app = App {
            id: "test_app".to_string(),
            key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_client_events_per_second: 100,
            ..Default::default()
        };
        let app_manager = Arc::new(MemoryAppManager::new());
        let integration = WebhookIntegration::new(WebhookConfig::default(), app_manager.clone())
            .await
            .unwrap();

        let result = integration
            .send_channel_occupied(&app, "test_channel")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_webhook_integration_send_channel_vacated() {
        let app = App {
            id: "test_app".to_string(),
            key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_client_events_per_second: 100,
            ..Default::default()
        };
        let app_manager = Arc::new(MemoryAppManager::new());
        let integration = WebhookIntegration::new(WebhookConfig::default(), app_manager.clone())
            .await
            .unwrap();

        let result = integration.send_channel_vacated(&app, "test_channel").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_webhook_integration_send_subscription_count_changed() {
        let app = App {
            id: "test_app".to_string(),
            key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_client_events_per_second: 100,
            ..Default::default()
        };
        let app_manager = Arc::new(MemoryAppManager::new());
        let integration = WebhookIntegration::new(WebhookConfig::default(), app_manager.clone())
            .await
            .unwrap();

        let result = integration
            .send_subscription_count_changed(&app, "test_channel", 5)
            .await;
        assert!(result.is_ok());
    }
}
