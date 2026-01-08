// src/metrics/mod.rs

pub mod prometheus;

pub use prometheus::PrometheusMetricsDriver;
use tokio::sync::Mutex;

use crate::websocket::SocketId;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Metrics Interface trait that any metrics driver should implement
#[async_trait]
pub trait MetricsInterface: Send + Sync {
    /// Initialize the metrics driver
    async fn init(&self) -> crate::error::Result<()>;

    /// Handle a new connection
    fn mark_new_connection(&self, app_id: &str, socket_id: &SocketId);

    /// Handle a disconnection
    fn mark_disconnection(&self, app_id: &str, socket_id: &SocketId);

    /// Handle a connection error
    fn mark_connection_error(&self, app_id: &str, error_type: &str);

    /// Track a rate limit check
    fn mark_rate_limit_check(&self, app_id: &str, limiter_type: &str);

    /// Track a rate limit check with request context
    fn mark_rate_limit_check_with_context(
        &self,
        app_id: &str,
        limiter_type: &str,
        request_context: &str,
    );

    /// Track a rate limit trigger (when limit is exceeded)
    fn mark_rate_limit_triggered(&self, app_id: &str, limiter_type: &str);

    /// Track a rate limit trigger with request context
    fn mark_rate_limit_triggered_with_context(
        &self,
        app_id: &str,
        limiter_type: &str,
        request_context: &str,
    );

    /// Track a channel subscription
    fn mark_channel_subscription(&self, app_id: &str, channel_type: &str);

    /// Track a channel unsubscription
    fn mark_channel_unsubscription(&self, app_id: &str, channel_type: &str);

    /// Update the count of active channels
    fn update_active_channels(&self, app_id: &str, channel_type: &str, count: i64);

    /// Handle a new API message event being received and sent out
    fn mark_api_message(
        &self,
        app_id: &str,
        incoming_message_size: usize,
        sent_message_size: usize,
    );

    /// Handle a new WS client message event being sent
    fn mark_ws_message_sent(&self, app_id: &str, sent_message_size: usize);

    /// Handle multiple WS client messages being sent (batch update for performance)
    fn mark_ws_messages_sent_batch(&self, app_id: &str, sent_message_size: usize, count: usize);

    /// Handle a new WS client message being received
    fn mark_ws_message_received(&self, app_id: &str, message_size: usize);

    /// Track the time in which horizontal adapter resolves requests from other nodes
    fn track_horizontal_adapter_resolve_time(&self, app_id: &str, time_ms: f64);

    /// Track the fulfillings in which horizontal adapter resolves requests from other nodes
    fn track_horizontal_adapter_resolved_promises(&self, app_id: &str, resolved: bool);

    /// Handle a new horizontal adapter request sent
    fn mark_horizontal_adapter_request_sent(&self, app_id: &str);

    /// Handle a new horizontal adapter request that was marked as received
    fn mark_horizontal_adapter_request_received(&self, app_id: &str);

    /// Handle a new horizontal adapter response from other node
    fn mark_horizontal_adapter_response_received(&self, app_id: &str);

    /// Track broadcast message latency
    fn track_broadcast_latency(
        &self,
        app_id: &str,
        channel_name: &str,
        recipient_count: usize,
        latency_ms: f64,
    );

    // Webhook metrics

    /// Track a webhook being sent successfully
    fn mark_webhook_sent(&self, app_id: &str, event_type: &str);

    /// Track a webhook failure
    fn mark_webhook_failed(&self, app_id: &str, event_type: &str, error_type: &str);

    /// Track webhook processing latency
    fn track_webhook_latency(&self, app_id: &str, event_type: &str, latency_ms: f64);

    /// Track webhook retry
    fn mark_webhook_retry(&self, app_id: &str, event_type: &str);

    /// Update webhook queue depth
    fn update_webhook_queue_depth(&self, queue_name: &str, depth: i64);

    /// Get the stored metrics as plain text, if possible
    async fn get_metrics_as_plaintext(&self) -> String;

    /// Get the stored metrics as JSON, if possible
    async fn get_metrics_as_json(&self) -> Value;

    /// Reset the metrics at the server level
    async fn clear(&self);
}

/// Factory for creating metrics instances
pub struct MetricsFactory;

impl MetricsFactory {
    /// Create a new metrics driver based on the specified driver type
    pub async fn create(
        driver_type: &str,
        port: u16,
        prefix: Option<&str>,
    ) -> Option<Arc<Mutex<dyn MetricsInterface + Send + Sync>>> {
        match driver_type.to_lowercase().as_str() {
            "prometheus" => {
                let driver = PrometheusMetricsDriver::new(port, prefix).await;
                Some(Arc::new(Mutex::new(driver)))
            }
            // Add more drivers here
            _ => None,
        }
    }
}
