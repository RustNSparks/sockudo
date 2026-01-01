// src/webhook/sender.rs
// Keep for App struct
use crate::app::config::App;
use crate::app::manager::AppManager; // Keep for AppManager trait
use crate::error::{Error, Result};
use crate::metrics::MetricsInterface;

#[cfg(feature = "lambda")]
use crate::webhook::lambda_sender::LambdaWebhookSender;
// JobData now contains app_secret and its payload.events is Vec<Value>
// PusherWebhookPayload is the structure for the final POST body
use crate::token::Token; // For HMAC SHA256 signing
use crate::webhook::types::{JobData, PusherWebhookPayload, Webhook};
use reqwest::{Client, header};
use serde_json::Value;
#[cfg(feature = "lambda")]
use serde_json::json; // json! macro only used in lambda feature
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, error, info, warn};

pub type JobProcessorFnAsync = Box<
    dyn Fn(JobData) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync + 'static,
>;

const MAX_CONCURRENT_WEBHOOKS: usize = 20;

/// Parameters for creating an HTTP webhook task
struct HttpWebhookTaskParams {
    url: url::Url,
    webhook_config: Webhook,
    permit: tokio::sync::OwnedSemaphorePermit,
    app_key: String,
    signature: String,
    body_to_send: String,
}

pub struct WebhookSender {
    client: Client,
    app_manager: Arc<dyn AppManager + Send + Sync>, // Still needed to fetch App if JobData doesn't have full App
    #[cfg(feature = "lambda")]
    lambda_sender: LambdaWebhookSender,
    webhook_semaphore: Arc<Semaphore>,
    metrics: Option<Arc<Mutex<dyn MetricsInterface + Send + Sync>>>,
}

impl WebhookSender {
    pub fn new(app_manager: Arc<dyn AppManager + Send + Sync>) -> Self {
        Self::with_metrics(app_manager, None)
    }

    pub fn with_metrics(
        app_manager: Arc<dyn AppManager + Send + Sync>,
        metrics: Option<Arc<Mutex<dyn MetricsInterface + Send + Sync>>>,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10)) // Timeout for HTTP requests
            .build()
            .unwrap_or_default();
        Self {
            client,
            app_manager,
            #[cfg(feature = "lambda")]
            lambda_sender: LambdaWebhookSender::new(),
            webhook_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_WEBHOOKS)),
            metrics,
        }
    }

    async fn get_app_config(&self, app_id: &str) -> Result<App> {
        match self.app_manager.find_by_id(app_id).await? {
            Some(app) => Ok(app),
            None => {
                error!("Webhook: Failed to find app with ID: {}", app_id);
                Err(Error::InvalidAppKey)
            }
        }
    }

    async fn validate_webhook_job(&self, app_id: &str, events: &[Value]) -> Result<()> {
        if events.is_empty() {
            warn!("Webhook job for app {} has no events.", app_id);
            return Ok(());
        }
        Ok(())
    }

    fn create_pusher_payload(&self, job: &JobData) -> Result<(PusherWebhookPayload, String)> {
        let pusher_payload = PusherWebhookPayload {
            time_ms: job.payload.time_ms,
            events: job.payload.events.clone(),
        };

        let body_json_string = serde_json::to_string(&pusher_payload)
            .map_err(|e| Error::Serialization(format!("Failed to serialize webhook body: {e}")))?;

        let _signature =
            Token::new(job.app_key.clone(), job.app_secret.clone()).sign(&body_json_string);
        Ok((pusher_payload, body_json_string))
    }

    fn find_relevant_webhooks<'a>(
        &self,
        events: &[Value],
        webhook_configs: &'a [Webhook],
    ) -> HashMap<String, &'a Webhook> {
        let mut relevant_configs = HashMap::new();

        for event_value in events {
            if let Some(event_name) = event_value.get("name").and_then(Value::as_str) {
                for wh_config in webhook_configs {
                    if wh_config.event_types.contains(&event_name.to_string()) {
                        let key = wh_config
                            .url
                            .as_ref()
                            .map(|u| u.to_string())
                            .or_else(|| wh_config.lambda_function.clone())
                            .or_else(|| wh_config.lambda.as_ref().map(|l| l.function_name.clone()))
                            .unwrap_or_else(String::new);

                        if !key.is_empty() {
                            relevant_configs.entry(key).or_insert(wh_config);
                        }
                    }
                }
            }
        }
        relevant_configs
    }

    pub async fn process_webhook_job(&self, job: JobData) -> Result<()> {
        let app_id = job.app_id.clone();
        let app_key = job.app_key.clone();
        debug!("Processing webhook job for app_id: {}", app_id);

        // Get app configuration
        let app_config = self.get_app_config(&app_id).await?;

        // Get webhook configurations
        let webhook_configs = match &app_config.webhooks {
            Some(hooks) => hooks,
            None => {
                debug!("No webhooks configured for app: {}", app_id);
                return Ok(());
            }
        };

        // Validate job
        self.validate_webhook_job(&app_id, &job.payload.events)
            .await?;

        // Create payload and signature
        let (pusher_payload, body_json_string) = self.create_pusher_payload(&job)?;
        let signature =
            Token::new(job.app_key.clone(), job.app_secret.clone()).sign(&body_json_string);

        // Find relevant webhooks
        let relevant_webhooks = self.find_relevant_webhooks(&job.payload.events, webhook_configs);
        if relevant_webhooks.is_empty() {
            debug!(
                "No matching webhook configurations for events in job for app {}",
                app_id
            );
            return Ok(());
        }

        log_webhook_processing_pusher_format(&app_id, &pusher_payload);

        // Extract event types for metrics
        let event_types: Vec<String> = job
            .payload
            .events
            .iter()
            .filter_map(|e| e.get("name").and_then(Value::as_str).map(String::from))
            .collect();
        let event_type_str = if event_types.len() == 1 {
            event_types[0].clone()
        } else {
            "batch".to_string()
        };

        // Process webhooks
        let mut tasks = Vec::new();
        for (_endpoint_key, webhook_config) in relevant_webhooks {
            let permit = self
                .webhook_semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| {
                    Error::Other(format!("Failed to acquire webhook semaphore permit: {e}"))
                })?;

            let task = self.create_webhook_task(
                webhook_config,
                permit,
                app_id.clone(),
                app_key.clone(),
                signature.clone(),
                body_json_string.clone(),
                event_type_str.clone(),
            );
            tasks.push(task);
        }

        // Wait for all tasks to complete
        for task_handle in tasks {
            if let Err(e) = task_handle.await {
                error!("Webhook task execution failed: {}", e);
            }
        }

        Ok(())
    }

    fn create_webhook_task(
        &self,
        webhook_config: &Webhook,
        permit: tokio::sync::OwnedSemaphorePermit,
        app_id: String,
        app_key: String,
        signature: String,
        body_to_send: String,
        event_type: String,
    ) -> tokio::task::JoinHandle<()> {
        if let Some(url) = &webhook_config.url {
            let params = HttpWebhookTaskParams {
                url: url.clone(),
                webhook_config: webhook_config.clone(),
                permit,
                app_key,
                signature,
                body_to_send,
            };

            self.create_http_webhook_task(params, app_id, event_type)
        } else if webhook_config.lambda.is_some() || webhook_config.lambda_function.is_some() {
            #[cfg(feature = "lambda")]
            {
                self.create_lambda_webhook_task(webhook_config, permit, app_id, body_to_send)
            }
            #[cfg(not(feature = "lambda"))]
            {
                warn!(
                    "Lambda webhook configured for app {} but Lambda support not compiled in.",
                    app_id
                );
                drop(permit);
                tokio::spawn(async {})
            }
        } else {
            warn!(
                "Webhook for app {} has neither URL nor Lambda config.",
                app_id
            );
            drop(permit);
            tokio::spawn(async {})
        }
    }

    fn create_http_webhook_task(
        &self,
        params: HttpWebhookTaskParams,
        app_id: String,
        event_type: String,
    ) -> tokio::task::JoinHandle<()> {
        let client = self.client.clone();
        let url_str = params.url.to_string();
        let custom_headers = params
            .webhook_config
            .headers
            .as_ref()
            .map(|h| h.headers.clone())
            .unwrap_or_default();
        let metrics = self.metrics.clone();

        tokio::spawn(async move {
            let _permit = params.permit;
            let start = Instant::now();

            let result = send_pusher_webhook(
                &client,
                &url_str,
                &params.app_key,
                &params.signature,
                params.body_to_send,
                custom_headers,
            )
            .await;

            let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

            // Record metrics
            if let Some(metrics_ref) = &metrics {
                let m = metrics_ref.lock().await;
                m.track_webhook_latency(&app_id, &event_type, latency_ms);
                match &result {
                    Ok(_) => m.mark_webhook_sent(&app_id, &event_type),
                    Err(e) => {
                        let error_type = categorize_webhook_error(e);
                        m.mark_webhook_failed(&app_id, &event_type, error_type);
                    }
                }
            }

            match result {
                Ok(_) => debug!("Successfully sent Pusher webhook to URL: {}", url_str),
                Err(e) => error!("Webhook send error to URL {}: {}", url_str, e),
            }
        })
    }

    #[cfg(feature = "lambda")]
    fn create_lambda_webhook_task(
        &self,
        webhook_config: &Webhook,
        permit: tokio::sync::OwnedSemaphorePermit,
        app_id: String,
        body_to_send: String,
    ) -> tokio::task::JoinHandle<()> {
        let lambda_sender = self.lambda_sender.clone();
        let webhook_clone = webhook_config.clone();
        let payload_for_lambda: Value = serde_json::from_str(&body_to_send).unwrap_or(json!({}));

        tokio::spawn(async move {
            let _permit = permit;
            if let Err(e) = lambda_sender
                .invoke_lambda(&webhook_clone, "batch_events", &app_id, payload_for_lambda)
                .await
            {
                error!("Lambda webhook error for app {}: {}", app_id, e);
            } else {
                debug!("Successfully invoked Lambda for app: {}", app_id);
            }
        })
    }
}

impl Clone for WebhookSender {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            app_manager: self.app_manager.clone(),
            #[cfg(feature = "lambda")]
            lambda_sender: self.lambda_sender.clone(),
            webhook_semaphore: self.webhook_semaphore.clone(),
            metrics: self.metrics.clone(),
        }
    }
}

/// Categorize webhook errors for metrics
fn categorize_webhook_error(error: &Error) -> &'static str {
    match error {
        Error::Internal(_) => "internal",
        Error::InvalidAppKey => "invalid_app",
        Error::Serialization(_) => "serialization",
        Error::Other(msg) => {
            if msg.contains("timeout") || msg.contains("Timeout") {
                "timeout"
            } else if msg.contains("connection") || msg.contains("Connection") {
                "connection"
            } else if msg.contains("status 4") {
                "client_error"
            } else if msg.contains("status 5") {
                "server_error"
            } else {
                "unknown"
            }
        }
        _ => "unknown",
    }
}

/// Helper function to send a Pusher-formatted webhook
async fn send_pusher_webhook(
    client: &Client,
    url: &str,
    app_key: &str,
    signature: &str,
    json_body: String, // Expects already serialized JSON string
    custom_headers_config: HashMap<String, String>,
) -> Result<()> {
    debug!("Sending Pusher webhook to URL: {}", url);

    let mut request_builder = client
        .post(url)
        .header(header::CONTENT_TYPE, "application/json")
        .header("X-Pusher-Key", app_key)
        .header("X-Pusher-Signature", signature);

    for (key, value) in custom_headers_config {
        request_builder = request_builder.header(key, value);
    }

    match request_builder.body(json_body).send().await {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                // 2XX status codes
                info!(
                    "Successfully sent Pusher webhook to {} (status: {})",
                    url, status
                );
                Ok(())
            } else {
                let error_text = response.text().await.unwrap_or_default();
                error!(
                    "{}",
                    format!(
                        "Pusher webhook to {} failed with status {}: {}",
                        url, status, error_text
                    )
                );
                Err(Error::Other(format!(
                    "Webhook to {url} failed with status {status}"
                )))
            }
        }
        Err(e) => {
            error!(
                "{}",
                format!("Failed to send Pusher webhook to {}: {}", url, e)
            );
            Err(Error::Other(format!(
                "HTTP request failed for webhook to {url}: {e}"
            )))
        }
    }
}

// Helper function to log webhook processing details (Pusher format)
fn log_webhook_processing_pusher_format(app_id: &str, payload: &PusherWebhookPayload) {
    debug!("Pusher Webhook for app ID: {}", app_id);
    for event in &payload.events {
        debug!("  Event: {:?}", event);
    }
}

#[cfg(test)]
mod tests {
    use crate::app::memory_app_manager::MemoryAppManager;

    use crate::webhook::types::JobPayload;

    use super::*;

    #[tokio::test]
    async fn test_creating_webhook_sender() {
        let webhook_sender = WebhookSender::new(Arc::new(MemoryAppManager::new()));
        assert!(webhook_sender.webhook_semaphore.available_permits() > 0);
        // Remove the timeout check since reqwest::Client doesn't have a timeout() method
        assert!(webhook_sender.app_manager.get_apps().await.is_ok());
    }

    #[tokio::test]
    async fn test_process_webhook_job_no_events() {
        let app_manager = Arc::new(MemoryAppManager::new());
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
        app_manager.create_app(app).await.unwrap();
        let webhook_sender = WebhookSender::new(app_manager.clone());

        let job = JobData {
            app_id: "test_app".to_string(),
            app_key: "test_key".to_string(),
            app_secret: "test_secret".to_string(),
            payload: JobPayload {
                time_ms: 1234567890,
                events: vec![],
            },
            original_signature: "test_signature".to_string(),
        };

        let result = webhook_sender.process_webhook_job(job).await;
        assert!(result.is_ok());
    }
    #[tokio::test]
    async fn test_process_webhook_job_with_events() {
        let app_manager = Arc::new(MemoryAppManager::new());
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
        app_manager.create_app(app).await.unwrap();
        let webhook_sender = WebhookSender::new(app_manager.clone());

        let job = JobData {
            app_id: "test_app".to_string(),
            app_key: "test_key".to_string(),
            app_secret: "test_secret".to_string(),
            payload: JobPayload {
                time_ms: 1234567890,
                events: vec![serde_json::json!({
                    "name": "channel_occupied",
                    "channel": "test-channel"
                })],
            },
            original_signature: "test_signature".to_string(),
        };

        let result = webhook_sender.process_webhook_job(job).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_webhook_job_invalid_app() {
        let app_manager = Arc::new(MemoryAppManager::new());
        let webhook_sender = WebhookSender::new(app_manager.clone());

        let job = JobData {
            app_id: "non_existent_app".to_string(),
            app_key: "test_key".to_string(),
            app_secret: "test_secret".to_string(),
            payload: JobPayload {
                time_ms: 1234567890,
                events: vec![],
            },
            original_signature: "test_signature".to_string(),
        };

        let result = webhook_sender.process_webhook_job(job).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_process_webhook_job_concurrent_requests() {
        let app_manager = Arc::new(MemoryAppManager::new());
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
        app_manager.create_app(app).await.unwrap();
        let webhook_sender = Arc::new(WebhookSender::new(app_manager.clone()));

        let mut handles = vec![];
        for i in 0..10 {
            let sender_clone = webhook_sender.clone();
            let job = JobData {
                app_id: "test_app".to_string(),
                app_key: "test_key".to_string(),
                app_secret: "test_secret".to_string(),
                payload: JobPayload {
                    time_ms: 1234567890 + i,
                    events: vec![serde_json::json!({
                        "name": "channel_occupied",
                        "channel": format!("test-channel-{}", i)
                    })],
                },
                original_signature: format!("test_signature_{i}"),
            };

            handles.push(tokio::spawn(async move {
                sender_clone.process_webhook_job(job).await
            }));
        }

        let results = futures::future::join_all(handles).await;
        for result in results {
            assert!(result.unwrap().is_ok());
        }
    }
}
